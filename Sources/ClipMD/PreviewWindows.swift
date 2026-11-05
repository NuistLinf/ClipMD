import AppKit
import ClipMDCore
import SwiftUI

/// 预览窗口。菜单里的 ⌘S、⌘1/2/3 通过响应链找到它。
final class PreviewWindow: NSWindow {
    weak var document: NoteDocument?

    @objc func saveDocument(_ sender: Any?) {
        guard let document else { return }
        Task { await document.saveNow() }
    }

    @objc func showMode(_ sender: NSMenuItem) {
        let modes = NoteDocument.Mode.allCases
        guard modes.indices.contains(sender.tag) else { return }
        document?.setMode(modes[sender.tag])
    }
}

/// 管理所有打开的预览窗口。同一个文件只开一个窗口；关掉后释放视图和内容。
@MainActor
final class PreviewWindows: NSObject, NSWindowDelegate {
    private struct Entry {
        let window: PreviewWindow
        let document: NoteDocument
    }

    private let store: NoteStore
    private var entries: [URL: Entry] = [:]

    init(store: NoteStore) {
        self.store = store
    }

    func show(_ note: SavedNote) {
        guard FileManager.default.fileExists(atPath: note.url.path(percentEncoded: false)) else {
            store.noteMissing()
            return
        }
        NSApp.activate()
        if let entry = entries[note.url] {
            entry.window.makeKeyAndOrderFront(nil)
            return
        }

        let document = NoteDocument(note: note)
        document.onSaved = { [weak store] in store?.reload() }
        let url = note.url
        let actions = PreviewActions(
            copyFile: { Pasteboard.writeFile(url) },
            reveal: { NSWorkspace.shared.activateFileViewerSelecting([url]) },
            apps: MarkdownApps.list(for: url),
            openWith: { app in
                NSWorkspace.shared.open([url], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration())
            }
        )

        let hosting = NSHostingController(rootView: PreviewView(document: document, actions: actions))
        // 让 SwiftUI 的 .toolbar / .navigationTitle 变成真正的窗口工具栏和标题
        hosting.sceneBridgingOptions = [.toolbars, .title]
        hosting.sizingOptions = []

        let window = PreviewWindow(contentViewController: hosting)
        window.document = document
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbarStyle = .unified
        window.title = note.title
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.setContentSize(NSSize(width: 780, height: 660))
        window.contentMinSize = NSSize(width: 480, height: 360)
        if let last = entries.values.first?.window {
            window.setFrameTopLeftPoint(last.cascadeTopLeft(from: NSPoint(x: last.frame.minX, y: last.frame.maxY)))
        } else {
            window.center()
        }

        entries[url] = Entry(window: window, document: document)
        window.makeKeyAndOrderFront(nil)
        Log.app.notice("preview opened; toolbar items=\(window.toolbar?.items.count ?? -1)")
        Task { await document.load() }
    }

    /// 文件被移到废纸篓：关掉它的窗口，不再写回
    func discard(_ url: URL) {
        guard let entry = entries[url] else { return }
        entry.document.discard()
        entry.window.close()
    }

    func saveAllSynchronously() {
        for entry in entries.values { entry.document.saveSynchronously() }
    }

    func closeAll() {
        entries.values.forEach { $0.window.close() }
    }

    var documents: [NoteDocument] { entries.values.map(\.document) }

    var toolbarItemCounts: String {
        entries.values.map { "\($0.window.toolbar?.items.count ?? -1)" }.joined(separator: ",")
    }

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? PreviewWindow,
              let (url, entry) = entries.first(where: { $0.value.window === window }) else { return }
        entries[url] = nil
        let document = entry.document
        Task { await document.saveNow() }
        // 最后一个窗口关掉后，把焦点还给之前的应用
        if entries.isEmpty { NSApp.deactivate() }
    }
}
