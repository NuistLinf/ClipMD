import AppKit
import SwiftUI

/// 列表行的点击和拖拽区域。
///
/// 不用 SwiftUI 的 `.onDrag`：它只允许"拷贝"，拖到废纸篓时系统不会处理原文件，我们也收不到通知。
/// 这里自己当拖拽源：拖到别的应用（微信、Finder）只允许拷贝，不会把文件从 ClipMD 的文件夹挪走；
/// 拖到废纸篓时系统会要求"删除"，由我们把原文件移到废纸篓（可撤销）。
struct FileDragArea: NSViewRepresentable {
    struct MenuItem {
        let title: String
        let symbol: String
        var startsGroup = false
        let action: @MainActor () -> Void
    }

    let url: URL
    let help: String
    let onClick: @MainActor () -> Void
    let onDropToTrash: @MainActor () -> Void
    let menuItems: [MenuItem]

    func makeNSView(context: Context) -> FileDragView {
        let view = FileDragView()
        view.setAccessibilityElement(false)
        updateNSView(view, context: context)
        return view
    }

    func updateNSView(_ view: FileDragView, context: Context) {
        view.url = url
        view.toolTip = help
        view.onClick = onClick
        view.onDropToTrash = onDropToTrash
        view.menuItems = menuItems
    }
}

final class FileDragView: NSView, NSDraggingSource {
    var url: URL?
    var onClick: (@MainActor () -> Void)?
    var onDropToTrash: (@MainActor () -> Void)?
    var menuItems: [FileDragArea.MenuItem] = []

    private var mouseDownEvent: NSEvent?
    private var isDragging = false

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override var mouseDownCanMoveWindow: Bool { false }

    override func mouseDown(with event: NSEvent) {
        mouseDownEvent = event
        isDragging = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard !isDragging, let down = mouseDownEvent, let url else { return }
        let dx = event.locationInWindow.x - down.locationInWindow.x
        let dy = event.locationInWindow.y - down.locationInWindow.y
        guard hypot(dx, dy) > 3 else { return }
        isDragging = true

        let item = NSDraggingItem(pasteboardWriter: url as NSURL)
        let icon = NSWorkspace.shared.icon(forFile: url.path(percentEncoded: false))
        let size: CGFloat = 40
        let point = convert(down.locationInWindow, from: nil)
        item.setDraggingFrame(NSRect(x: point.x - size / 2, y: point.y - size / 2, width: size, height: size), contents: icon)
        beginDraggingSession(with: [item], event: event, source: self)
    }

    override func mouseUp(with event: NSEvent) {
        if !isDragging, mouseDownEvent != nil { onClick?() }
        mouseDownEvent = nil
        isDragging = false
    }

    // MARK: NSDraggingSource

    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        context == .outsideApplication ? [.copy, .delete] : []
    }

    func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        Log.app.notice("row drag ended operation=\(operation.rawValue)")
        if operation.contains(.delete) { onDropToTrash?() }
        mouseDownEvent = nil
        isDragging = false
    }

    // MARK: 右键菜单

    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        for item in menuItems {
            if item.startsGroup { menu.addItem(.separator()) }
            let menuItem = ClosureMenuItem(title: item.title, action: item.action)
            menuItem.image = NSImage(systemSymbolName: item.symbol, accessibilityDescription: nil)
            menu.addItem(menuItem)
        }
        return menu
    }
}

/// 点了就执行闭包的菜单项
@MainActor
final class ClosureMenuItem: NSMenuItem {
    private let handler: @MainActor () -> Void

    init(title: String, action handler: @escaping @MainActor () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
    }

    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func run() { handler() }
}
