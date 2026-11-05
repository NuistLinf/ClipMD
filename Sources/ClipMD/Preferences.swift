import AppKit
import Observation
import UniformTypeIdentifiers

struct AppChoice: Identifiable, Equatable {
    let url: URL
    let name: String
    let icon: NSImage
    var id: URL { url }

    static func == (lhs: AppChoice, rhs: AppChoice) -> Bool { lhs.url == rhs.url }
}

/// 电脑上能打开 md 的编辑器（Typora、VS Code、文本编辑……）
@MainActor
enum MarkdownApps {
    private static var markdownType: UTType {
        UTType("net.daringfireball.markdown") ?? UTType(filenameExtension: "md") ?? .plainText
    }

    /// 声称能打开 md 的应用很多（浏览器、Excel、网盘……），只列常见的 Markdown / 文本编辑器，加上系统默认应用。
    /// 其他应用可以用 `pickOtherApp()` 自己选。
    private static let knownEditors: Set<String> = [
        "abnerworks.Typora", "com.microsoft.VSCode", "com.vscodium", "com.todesktop.230313mzl4w4u92",
        "md.obsidian", "pro.writer.mac", "com.uranusjr.macdown", "com.brettterpstra.marked2",
        "com.sublimetext.4", "com.sublimetext.3", "com.barebones.bbedit", "dev.zed.Zed",
        "com.panic.Nova", "com.coteditor.CotEditor", "com.apple.TextEdit", "com.apple.dt.Xcode",
        "com.multimarkdown.composer.mac", "com.bear-writer.Bear", "net.shinyfrog.bear",
    ]

    /// 默认应用排第一，不含 ClipMD 自己，同一个应用装了几份也只列一次
    static func list(for file: URL? = nil) -> [AppChoice] {
        let workspace = NSWorkspace.shared
        let candidates = file.map { workspace.urlsForApplications(toOpen: $0) } ?? workspace.urlsForApplications(toOpen: markdownType)
        let defaultApp = file.flatMap { workspace.urlForApplication(toOpen: $0) } ?? workspace.urlForApplication(toOpen: markdownType)
        let ownID = Bundle.main.bundleIdentifier
        var seen = Set<String>()
        let apps = candidates.filter { app in
            let id = Bundle(url: app)?.bundleIdentifier ?? app.path(percentEncoded: false)
            guard id != ownID, app == defaultApp || knownEditors.contains(id) else { return false }
            return seen.insert(id).inserted
        }
        return (apps.filter { $0 == defaultApp } + apps.filter { $0 != defaultApp })
            .map { choice(for: $0, isDefault: $0 == defaultApp) }
    }

    /// 让用户从"应用程序"里挑一个
    static func pickOtherApp() -> URL? {
        let picker = NSOpenPanel()
        picker.allowedContentTypes = [.application]
        picker.directoryURL = URL(filePath: "/Applications", directoryHint: .isDirectory)
        picker.prompt = "选择"
        picker.message = "选一个用来打开 md 文件的应用"
        NSApp.activate()
        return picker.runModal() == .OK ? picker.url : nil
    }

    static func choice(for app: URL, isDefault: Bool = false) -> AppChoice {
        let icon = NSWorkspace.shared.icon(forFile: app.path(percentEncoded: false))
        icon.size = NSSize(width: 16, height: 16)
        var name = FileManager.default.displayName(atPath: app.path(percentEncoded: false))
        if name.hasSuffix(".app") { name.removeLast(4) }
        if isDefault { name += "（系统默认）" }
        return AppChoice(url: app, name: name, icon: icon)
    }
}

/// 简单的偏好设置
@MainActor
@Observable
final class Preferences {
    static let shared = Preferences()
    private static let previewAppKey = "previewApp"
    private static let convertHTMLKey = "convertHTML"

    /// 点眼睛按钮时用哪个应用打开；nil 表示用 ClipMD 内置预览
    private(set) var previewApp: URL?
    /// 从网页复制时，把网页格式转成 Markdown（保留标题、列表、代码块……）
    private(set) var convertHTML = UserDefaults.standard.object(forKey: Preferences.convertHTMLKey) as? Bool ?? true

    private init() {
        previewApp = UserDefaults.standard.string(forKey: Self.previewAppKey)
            .map { URL(filePath: $0, directoryHint: .isDirectory) }
            // 应用被删了就回到内置预览
            .flatMap { FileManager.default.fileExists(atPath: $0.path(percentEncoded: false)) ? $0 : nil }
    }

    func setPreviewApp(_ app: URL?) {
        previewApp = app
        if let app {
            UserDefaults.standard.set(app.path(percentEncoded: false), forKey: Self.previewAppKey)
        } else {
            UserDefaults.standard.removeObject(forKey: Self.previewAppKey)
        }
    }

    func setConvertHTML(_ enabled: Bool) {
        convertHTML = enabled
        UserDefaults.standard.set(enabled, forKey: Self.convertHTMLKey)
    }

    var previewAppName: String? {
        previewApp.map { MarkdownApps.choice(for: $0).name }
    }
}
