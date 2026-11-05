import AppKit
import ClipMDCore
import SwiftUI

/// 只在启动参数带 `-enableTestURLs YES` 时生效的测试命令，用来在没有鼠标的情况下走完整流程：
///
///     open -g "clipmd://test/preview-latest"
///     open -g "clipmd://test/mode?edit"
///     open -g "clipmd://test/type?<URL 编码的新内容>"
///     open -g "clipmd://test/hide-latest" | trash-latest | undo | close-previews | hide-panel | show-panel | state
@MainActor
enum TestURLs {
    static var isEnabled: Bool { UserDefaults.standard.bool(forKey: "enableTestURLs") }

    static func handle(_ url: URL, store: NoteStore, panel: PanelController, previews: PreviewWindows, history: ClipHistory,
                       openSettings: () -> Void, eyeButton: (SavedNote) -> Void, saveClip: (ClipItem) -> Void,
                       recognizeClip: (ClipItem) -> Void, capture: (String, String) -> Void) {
        guard isEnabled else { return }
        let argument = url.query(percentEncoded: false) ?? ""
        let latest = store.notes.first
        switch url.path(percentEncoded: false) {
        case "/preview-latest": if let latest { previews.show(latest) }
        case "/mode": if let mode = NoteDocument.Mode(rawValue: argument) { previews.documents.forEach { $0.setMode(mode) } }
        case "/type": previews.documents.forEach { $0.update(text: argument) }
        case "/hide-latest": if let latest { panel.hide(latest) }
        case "/trash-latest": if let latest { panel.trash(latest) }
        case "/undo": panel.undo()
        case "/close-previews": previews.closeAll()
        case "/snapshot": snapshot(previews.documents.first, to: URL(filePath: argument, directoryHint: .isDirectory))
        case "/install-fonts": Task { await MathFontInstaller.shared.install() }
        case "/remove-fonts": MathFontInstaller.shared.remove()
        case "/settings": openSettings()
        case "/tab": panel.select(PanelTab(rawValue: argument) ?? .saved)
        // 和点标签按钮一样带动画
        case "/tab-click": withAnimation(.snappy(duration: 0.28)) { panel.select(PanelTab(rawValue: argument) ?? .saved) }
        case "/clip-save-latest": if let item = history.items.first { saveClip(item) }
        case "/clip-ocr-latest": if let item = history.items.first(where: \.isImage) { recognizeClip(item) }
        case "/clip-copy":
            let index = Int(argument) ?? 0
            if history.items.indices.contains(index) { Pasteboard.writeText(history.items[index].text, html: history.items[index].html) }
        case "/eye-latest": if let latest { eyeButton(latest) }
        case "/drop": panel.simulateDrop(.text(argument, html: nil))
        case "/drop-clipboard": panel.simulateDrop(.text(argument, html: nil), action: .clipboard)
        case "/drop-file": panel.simulateDrop(.files([URL(filePath: argument)]))
        case "/drop-html":
            // 参数是一个 HTML 文件的路径；纯文本用 HTML 去掉标签后的文字
            if let html = try? String(contentsOfFile: argument, encoding: .utf8) {
                let plain = html.replacingOccurrences(of: "<[^>]+>", with: " ", options: .regularExpression)
                panel.simulateDrop(.text(plain, html: html))
            }
        case "/drop-target":
            switch argument {
            case "save", "1": panel.setDropHover(.accept(.save))
            case "clipboard": panel.setDropHover(.accept(.clipboard))
            case "reject": panel.setDropHover(.reject(TrackingContainerView.rejectMessage))
            default: panel.setDropHover(.none)
            }
        case "/render-panel": panel.renderLayers(to: URL(filePath: argument))
        case "/scroll-clips": panel.simulateScroll(Int32(argument) ?? -120)
        case "/snapshot-panel": panel.snapshot(to: URL(filePath: argument, directoryHint: .isDirectory))
        case "/hide-panel": panel.hide()
        case "/show-panel": panel.show()
        case let command where command.hasPrefix("/shot") || command.hasPrefix("/clip-preview"): capture(command, argument)
        case "/state":
            let titles = store.notes.map(\.title).joined(separator: " | ")
            let docs = previews.documents.map { "\($0.title) [\($0.mode.rawValue), \($0.blocks.count) blocks, \($0.parsed.math.count) math, fonts=\(MathFontInstaller.shared.state), \($0.statusText)]" }
            let clips = history.items.prefix(6).map { "\($0.preview)\($0.savedFileName == nil ? "" : "[已存]")" }.joined(separator: " | ")
            Log.app.notice("state screenAccess=\(Screenshot.hasPermission)")
            Log.app.notice("state clips(\(history.items.count))=\(clips, privacy: .public) notes=\(titles, privacy: .public) previews=\(docs.joined(separator: "; "), privacy: .public) toolbar=\(previews.toolbarItemCounts, privacy: .public) panel=\(panel.debugState, privacy: .public)")
        default: Log.app.error("unknown test url \(url.absoluteString, privacy: .public)")
        }
    }

    /// 用 ImageRenderer 把预览内容画成 PNG（浅色、深色各一张），看排版和公式的实际效果
    private static func snapshot(_ document: NoteDocument?, to folder: URL) {
        guard let document else { return }
        for dark in [false, true] {
            let view = MarkdownContent(document: document.parsed, lazy: false)
                .frame(width: 640, alignment: .leading)
                .padding(28)
                .background(dark ? Color(white: 0.12) : Color.white)
                .environment(\.colorScheme, dark ? .dark : .light)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            guard let image = renderer.cgImage,
                  let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { continue }
            let file = folder.appending(path: dark ? "preview-dark.png" : "preview-light.png")
            try? png.write(to: file)
            Log.app.notice("snapshot written \(file.lastPathComponent, privacy: .public) \(image.width)x\(image.height)")
        }
    }
}
