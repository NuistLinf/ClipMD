import AppKit
import ClipMDCore
import SwiftUI
import UniformTypeIdentifiers

/// 不抢焦点的浮动面板：点它时前台应用（比如微信）仍然保持焦点，点完可以直接 ⌘V。
final class FloatingPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    // 允许窗口一部分在屏幕外，贴边收起要用
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

/// 窗口不是 key window 时，第一次点击也直接响应
final class FirstClickHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// 拖进悬浮窗的东西
enum DroppedContent {
    /// 文字（附带网页格式更好）
    case text(String, html: String?)
    /// 文本类文件（.md、.txt、.html、.rtf……），包括"放下后才生成"的文件
    case files([URL])
}

/// 松手后做什么：悬浮窗上半块存成 Markdown，下半块只放到剪贴板
enum DropAction: Equatable {
    case save
    case clipboard
}

/// 拖着东西经过悬浮窗时的状态
enum DropHover: Equatable {
    case none
    /// 能接住，鼠标停在哪一块
    case accept(DropAction)
    /// 接不住，给个提示
    case reject(String)

    var isAccepting: Bool {
        if case .accept = self { return true }
        return false
    }
}

/// 整个面板的鼠标进出和拖放。鼠标用 `.activeAlways`，应用不在前台时也有效。
///
/// 拖放收文字和文本类文件：拖拽用的是单独的拖拽剪贴板，不会触发系统的粘贴询问，也不会动系统剪贴板。
/// 有的应用（比如微信拖聊天气泡）给的不是普通文本，而是 RTF、临时文件或"放下后才生成的文件"，所以多登记几种类型。
/// 从悬浮窗自己拖出去的文件不接。
final class TrackingContainerView: NSView {
    /// 以左上角为原点：窗口改高度时（顶边不动），里面的内容不用跟着重新定位。
    /// 原点在左下角的话，窗口先变了大小、内容还没来得及挪的那一帧，整个面板会跳一下
    override var isFlipped: Bool { true }

    var onHoverChange: ((Bool) -> Void)?
    var onDropHoverChange: ((DropHover) -> Void)?
    var onDrop: ((DroppedContent, DropAction) -> Void)?

    static let plainTextTypes: [NSPasteboard.PasteboardType] = [
        .string, .init("public.utf16-plain-text"), .init("public.utf16-external-plain-text"),
        .init("public.plain-text"), .init("NSStringPboardType"),
    ]
    static let rejectMessage = "只能保存文字，或 .md、.txt 这类文本文件"
    /// Qt 写的应用（比如新版微信）拖自己的东西时，只放一个空的标记，真正的数据留在它自己进程里
    static let qtMarker = NSPasteboard.PasteboardType("com.trolltech.qt.MimeTypeName")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        let promiseTypes = NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) }
        // 图片、文件类型也登记上：接不住时能给提示，而不是毫无反应
        registerForDraggedTypes(Self.plainTextTypes + [.html, .rtf, .rtfd, .fileURL, .URL, .tiff, .png,
                                                       .init("NSFilenamesPboardType"), .init("public.jpeg"), Self.qtMarker] + promiseTypes)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private enum Verdict: Equatable { case ignore, accept, reject(String) }

    private func verdict(for info: NSDraggingInfo) -> Verdict {
        // 从悬浮窗自己拖出去的文件
        guard info.draggingSource == nil else { return .ignore }
        let pasteboard = info.draggingPasteboard
        if let files = Self.fileURLs(in: pasteboard) {
            if files.contains(where: DroppedText.isTextFile) { return .accept }
            return .reject(files.allSatisfy(Self.isImage) ? Self.imageMessage : "这类文件存不了。" + Self.rejectMessage)
        }
        if Self.text(in: pasteboard) != nil || pasteboard.types?.contains(.html) == true { return .accept }
        if let promises = Self.promises(in: info) {
            // 有的应用不告诉文件类型，先收下，放下后再看是不是文本
            let types = promises.flatMap(\.fileTypes).compactMap { UTType($0) }
            let looksLikeText = types.isEmpty || types.contains { $0.conforms(to: .text) }
            return looksLikeText ? .accept : .reject(types.allSatisfy { $0.conforms(to: .image) } ? Self.imageMessage : "这类文件存不了。" + Self.rejectMessage)
        }
        return .reject(Self.reason(forUnreadable: pasteboard))
    }

    private static let imageMessage = "图片存不了。" + rejectMessage

    private static func isImage(_ url: URL) -> Bool {
        UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) == true
    }

    /// 拖进来的东西里没有文字，也没有文件：按里面有什么来说明，不猜是哪种场景
    private static func reason(forUnreadable pasteboard: NSPasteboard) -> String {
        let types = pasteboard.types ?? []
        if types.contains(where: { [.tiff, .png, .init("public.jpeg")].contains($0) }) { return imageMessage }
        // Qt 写的应用（比如新版微信）拖它自己的东西时，只放一个 0 字节的标记，真正的数据留在它自己进程里
        if types.contains(qtMarker) {
            let name = NSWorkspace.shared.frontmostApplication?.localizedName ?? "这个应用"
            return "\(name) 拖出来的是它内部用的数据，没有文字。选中文字再拖，或复制后按 ⌃⌥S"
        }
        return "这里面没有文字。" + rejectMessage
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        let types = (sender.draggingPasteboard.types ?? []).map(\.rawValue)
        let verdict = verdict(for: sender)
        Log.app.notice("drag entered verdict=\(String(describing: verdict), privacy: .public) types=\(types.joined(separator: ", "), privacy: .public)")
        switch verdict {
        case .ignore: return []
        case .accept:
            onDropHoverChange?(.accept(action(at: sender)))
            return .copy
        case .reject(let message):
            onDropHoverChange?(.reject(message))
            return []
        }
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard verdict(for: sender) == .accept else { return [] }
        onDropHoverChange?(.accept(action(at: sender)))
        return .copy
    }

    /// 玻璃的上半块存成 Markdown，下半块放到剪贴板（这里的坐标原点在左上角）
    private func action(at info: NSDraggingInfo) -> DropAction {
        let point = convert(info.draggingLocation, from: nil)
        let glass = bounds.insetBy(dx: PanelMetrics.margin, dy: PanelMetrics.margin)
        return point.y <= glass.midY ? .save : .clipboard
    }

    override func draggingExited(_ sender: NSDraggingInfo?) { onDropHoverChange?(.none) }
    override func draggingEnded(_ sender: NSDraggingInfo) { onDropHoverChange?(.none) }
    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { verdict(for: sender) == .accept }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let pasteboard = sender.draggingPasteboard
        let action = action(at: sender)
        if let files = Self.fileURLs(in: pasteboard) {
            onDrop?(.files(files.filter(DroppedText.isTextFile)), action)
            return true
        }
        let html = pasteboard.string(forType: .html)
        if let text = Self.text(in: pasteboard) {
            onDrop?(.text(text, html: html), action)
            return true
        }
        if let html {
            onDrop?(.text("", html: html), action)
            return true
        }
        if let promises = Self.promises(in: sender) {
            receive(promises, action: action)
            return true
        }
        return false
    }

    /// "放下后才生成的文件"：先收到临时文件夹，收齐了再当文件处理
    private func receive(_ promises: [NSFilePromiseReceiver], action: DropAction) {
        let folder = FileManager.default.temporaryDirectory.appending(path: "ClipMD-drop-\(UUID().uuidString)", directoryHint: .isDirectory)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let queue = OperationQueue()
        let group = DispatchGroup()
        let lock = NSLock()
        var received: [URL] = []
        for promise in promises {
            group.enter()
            promise.receivePromisedFiles(atDestination: folder, options: [:], operationQueue: queue) { url, error in
                if error == nil {
                    lock.lock()
                    received.append(url)
                    lock.unlock()
                }
                group.leave()
            }
        }
        group.notify(queue: .main) { [weak self] in
            Log.app.notice("received \(received.count) promised file(s)")
            MainActor.assumeIsolated { self?.onDrop?(.files(received), action) }
        }
    }

    /// 各种纯文本格式，再试 RTF
    static func text(in pasteboard: NSPasteboard) -> String? {
        for type in plainTextTypes {
            if let text = pasteboard.string(forType: type), text.contains(where: { !$0.isWhitespace }) { return text }
        }
        for type in [NSPasteboard.PasteboardType.rtf, .rtfd] {
            if let data = pasteboard.data(forType: type),
               let text = (try? NSAttributedString(data: data, options: [:], documentAttributes: nil))?.string,
               text.contains(where: { !$0.isWhitespace }) { return text }
        }
        return nil
    }

    static func fileURLs(in pasteboard: NSPasteboard) -> [URL]? {
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]
        return urls?.isEmpty == false ? urls : nil
    }

    static func promises(in info: NSDraggingInfo) -> [NSFilePromiseReceiver]? {
        var receivers: [NSFilePromiseReceiver] = []
        info.enumerateDraggingItems(options: [], for: nil, classes: [NSFilePromiseReceiver.self], searchOptions: [:]) { item, _, _ in
            if let receiver = item.item as? NSFilePromiseReceiver { receivers.append(receiver) }
        }
        return receivers.isEmpty ? nil : receivers
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero,
                                       options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self))
    }

    override func mouseEntered(with event: NSEvent) { onHoverChange?(true) }
    override func mouseExited(with event: NSEvent) { onHoverChange?(false) }
}

@MainActor
final class PanelController {
    enum State { case floating, tucked, peeking }

    enum Corner: CaseIterable {
        case topRight, bottomRight, topLeft, bottomLeft

        var title: String {
            switch self {
            case .topRight: "右上角"
            case .bottomRight: "右下角"
            case .topLeft: "左上角"
            case .bottomLeft: "左下角"
            }
        }

        var isTop: Bool { self == .topRight || self == .topLeft }
        var isRight: Bool { self == .topRight || self == .bottomRight }
    }

    private enum Keys {
        static let x = "panelX", y = "panelY", tucked = "panelTucked", pinned = "panelPinned", tab = "panelTab"
    }

    private let store: NoteStore
    private let history: ClipHistory
    private let model = PanelModel()
    private var actions: PanelActions?
    private var dropHandler: (@MainActor (DroppedContent, DropAction) -> Void)?
    private let panel: FloatingPanel

    private(set) var state: State = .floating
    private var edge: DockEdge = .right
    private var dockScreen: NSScreen?
    /// 展开状态下窗口左上角的位置，收起再展开时回到这里
    private var floatingTopLeft: NSPoint?
    private var contentHeight: CGFloat = 160
    /// SwiftUI 最近一次量到的内容高度（窗口缩小会晚一点跟上）
    private var measuredHeight: CGFloat = 160
    private var mouseInside = false
    /// 拖动开始时的鼠标位置和窗口原点
    private var drag: (mouse: NSPoint, origin: NSPoint)?
    private var hintChangeCount = -1

    private var hoverTask: Task<Void, Never>?
    private var shrinkTask: Task<Void, Never>?
    private var feedbackTask: Task<Void, Never>?
    private var noticeTask: Task<Void, Never>?
    private var freshCopyTask: Task<Void, Never>?
    private var undoAction: (@MainActor () -> Void)?
    private let onTrashed: @MainActor (URL) -> Void

    var debugState: String {
        "freshCopy=\(model.freshCopy) justSaved=\(model.justSaved) notice=\(model.notice?.message ?? "-") "
            + "tab=\(model.tab.rawValue) content=\(Int(contentHeight)) measured=\(Int(measuredHeight)) window=\(Int(panel.frame.height - PanelMetrics.margin * 2))"
    }

    /// 测试：把窗口里真实的图层树画下来（ImageRenderer 画不出滚动视图，这里能看到滚动后的样子）
    func renderLayers(to url: URL) {
        guard let layer = panel.contentView?.layer else { return }
        let scale = panel.backingScaleFactor
        let size = layer.bounds.size
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: Int(size.width * scale), height: Int(size.height * scale), bitsPerComponent: 8,
                                      bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return }
        context.setFillColor(CGColor(gray: 0.85, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: size.width * scale, height: size.height * scale))
        context.scaleBy(x: scale, y: scale)
        if layer.isGeometryFlipped || panel.contentView?.isFlipped == true {
            context.translateBy(x: 0, y: size.height)
            context.scaleBy(x: 1, y: -1)
        }
        layer.render(in: context)
        if let image = context.makeImage() { CaptureRenderer.writePNG(image, to: url) }
    }

    /// 测试滚轮：在列表中间做一次命中测试（看滚轮事件会不会落到列表上），再给列表发一个滚轮事件
    func simulateScroll(_ dy: Int32) {
        guard let content = panel.contentView,
              let cgEvent = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0),
              let event = NSEvent(cgEvent: cgEvent) else { return }
        func descendants(_ view: NSView) -> [NSView] { view.subviews + view.subviews.flatMap(descendants) }
        let scrollViews = descendants(content).compactMap { $0 as? NSScrollView }
        // 两个标签的列表都在，只有当前标签的那个接得到鼠标
        func lands(in list: NSScrollView) -> Bool {
            let center = content.convert(NSPoint(x: list.bounds.midX, y: list.bounds.midY), from: list)
            return content.hitTest(center).map { $0 === list || $0.isDescendant(of: list) } ?? false
        }
        guard let list = scrollViews.first(where: lands) ?? scrollViews.last else {
            Log.app.notice("scroll test: no scroll view")
            return
        }
        let lands = lands(in: list)
        list.scrollWheel(with: event)
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard let self else { return }
            Log.app.notice("scroll test: scrollViews=\(scrollViews.count) hitLandsInList=\(lands) offset=\(self.model.listScrollOffset) listHeight=\(list.frame.height)")
        }
    }
    var isVisible: Bool { panel.isVisible }
    var isTucked: Bool { state != .floating }
    var isPinned: Bool { model.pinned }

    init(store: NoteStore,
         history: ClipHistory,
         onSave: @escaping @MainActor () -> Void,
         onPreview: @escaping @MainActor (SavedNote) -> Void,
         onTrashed: @escaping @MainActor (URL) -> Void,
         onOpenFolder: @escaping @MainActor () -> Void,
         onFix: @escaping @MainActor (FailureFix) -> Void,
         onOpenSettings: @escaping @MainActor () -> Void,
         onDrop: @escaping @MainActor (DroppedContent, DropAction) -> Void,
         onSaveClip: @escaping @MainActor (ClipItem) -> Void,
         onCopyClip: @escaping @MainActor (ClipItem) -> Void,
         onPreviewClip: @escaping @MainActor (ClipItem) -> Void,
         onRecognizeClip: @escaping @MainActor (ClipItem) -> Void,
         onRevealClip: @escaping @MainActor (ClipItem) -> Void,
         onScreenshot: @escaping @MainActor (Bool) -> Void) {
        self.store = store
        self.history = history
        self.onTrashed = onTrashed
        panel = FloatingPanel(
            contentRect: NSRect(x: 0, y: 0, width: PanelMetrics.width + PanelMetrics.margin * 2, height: 200),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        // 拖动由标题栏的手势处理（见 dragChanged）
        panel.isMovableByWindowBackground = false
        panel.hidesOnDeactivate = false
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isReleasedWhenClosed = false
        panel.animationBehavior = .none

        model.pinned = UserDefaults.standard.bool(forKey: Keys.pinned)
        model.tab = UserDefaults.standard.string(forKey: Keys.tab).flatMap(PanelTab.init(rawValue:)) ?? .saved

        let actions = PanelActions(
            save: onSave,
            copy: { [weak self] in self?.copy($0) ?? false },
            preview: onPreview,
            reveal: { [weak self] in self?.reveal($0) },
            hide: { [weak self] in self?.hide($0) },
            trash: { [weak self] in self?.trash($0) },
            undo: { [weak self] in self?.undo() },
            dismissNotice: { [weak self] in self?.dismissNotice() },
            openFolder: onOpenFolder,
            toggleTuck: { [weak self] in self?.toggleTuck() },
            togglePin: { [weak self] in self?.togglePin() },
            dragChanged: { [weak self] in self?.dragChanged() },
            dragEnded: { [weak self] in self?.dragEnded() },
            fix: onFix,
            openSettings: onOpenSettings,
            selectTab: { [weak self] in self?.select($0) },
            copyClip: onCopyClip,
            previewClip: onPreviewClip,
            saveClip: onSaveClip,
            recognizeClip: onRecognizeClip,
            revealClip: onRevealClip,
            screenshot: onScreenshot,
            removeClip: { [weak self] in self?.removeClip($0) },
            clearClips: { [weak self] in self?.clearClips() },
            dismissFailure: { [weak self] in self?.dismissFailure() },
            heightChanged: { [weak self] in self?.contentHeightChanged($0) }
        )
        let container = TrackingContainerView(frame: NSRect(origin: .zero, size: panel.frame.size))
        let hosting = FirstClickHostingView(rootView: PanelView(store: store, history: history, model: model, actions: actions))
        // 窗口大小由我们自己控制，不让 SwiftUI 去改
        hosting.sizingOptions = []
        hosting.frame = container.bounds
        hosting.autoresizingMask = [.width, .height]
        container.addSubview(hosting)
        container.onHoverChange = { [weak self] in self?.hoverChanged($0) }
        container.onDropHoverChange = { [weak self] in self?.dropHoverChanged($0) }
        container.onDrop = { onDrop($0, $1) }
        // SwiftUI 的宿主视图不接拖放，统一交给外层的 container
        hosting.unregisterDraggedTypes()
        self.dropHandler = onDrop
        self.actions = actions
        panel.contentView = container
    }

    // MARK: 显示与隐藏

    func show() {
        guard !panel.isVisible else { return }
        refreshHint(force: true)
        let topLeft = clamped(savedTopLeft() ?? defaultTopLeft())
        floatingTopLeft = topLeft
        panel.setFrame(frame(topLeft: topLeft), display: false)
        panel.orderFrontRegardless()
        if UserDefaults.standard.bool(forKey: Keys.tucked) { tuck(animated: false) }
    }

    func hide() {
        panel.orderOut(nil)
        onCollapse?()
    }

    /// 悬浮窗收起或隐藏时调用（预览窗口跟着关）
    var onCollapse: (() -> Void)?

    /// 玻璃卡片在屏幕上的位置（去掉给阴影留的透明边）
    var cardFrame: NSRect { panel.frame.insetBy(dx: PanelMetrics.margin, dy: PanelMetrics.margin) }

    // MARK: 保存结果

    func show(_ feedback: Feedback) {
        refreshHint(force: true)
        updatePasteTip()
        feedbackTask?.cancel()

        switch feedback {
        case .saved(_, _, let onClipboard):
            freshCopyTask?.cancel()
            withAnimation(.snappy(duration: 0.3)) {
                model.failure = nil
                model.notice = nil
                model.freshCopy = false
                model.dropHover = .none
                model.savedOnClipboard = onClipboard
                model.justSaved = true
            }
            peekBriefly(for: .seconds(1.8))
            feedbackTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(2.2))
                guard let self, !Task.isCancelled else { return }
                withAnimation(.smooth(duration: 0.35)) { self.model.justSaved = false }
            }
        case .copied:
            withAnimation(.snappy(duration: 0.3)) { model.dropHover = .none }
            peekBriefly(for: .seconds(1.5))
        case .working:
            // 只在屏幕顶部提示，悬浮窗不动
            break
        case .failed(let failure):
            withAnimation(.snappy(duration: 0.3)) {
                model.justSaved = false
                model.failure = failure
            }
            peekBriefly(for: .seconds(4))
            // 带修复按钮的错误一直留着，直到你关掉或下一次保存
            guard failure.fix == nil else { return }
            feedbackTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(6))
                guard let self, !Task.isCancelled else { return }
                withAnimation(.smooth(duration: 0.35)) { self.model.failure = nil }
            }
        }
    }

    /// 复制了一段文字：保存按钮弹一下，提示可以点它保存，5 秒后恢复。收起状态下不探出，免得每次复制都打扰。
    func noteTextCopied() {
        refreshHint(force: true)
        guard !model.justSaved else { return }
        freshCopyTask?.cancel()
        model.copyPulse += 1
        withAnimation(.smooth(duration: 0.3)) { model.freshCopy = true }
        freshCopyTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(5))
            guard let self, !Task.isCancelled else { return }
            withAnimation(.smooth(duration: 0.4)) { self.model.freshCopy = false }
        }
    }

    func updatePasteTip() {
        let canRead = Pasteboard.accessBehavior == .alwaysAllow
        if canRead != model.canReadClipboard { model.canReadClipboard = canRead }
        let asks = Pasteboard.accessBehavior == .ask
        guard asks != model.pasteAsksEveryTime else { return }
        withAnimation(.smooth(duration: 0.3)) { model.pasteAsksEveryTime = asks }
    }

    private func dismissFailure() {
        feedbackTask?.cancel()
        withAnimation(.smooth(duration: 0.3)) { model.failure = nil }
    }

    /// 剪贴板变了才重新看一次类型（不读内容，不会触发系统询问）
    private func refreshHint(force: Bool = false) {
        let count = Pasteboard.changeCount
        guard force || count != hintChangeCount else { return }
        hintChangeCount = count
        let hint = Pasteboard.hint()
        if hint != model.hint { model.hint = hint }
    }

    // MARK: 拖动与固定

    func togglePin() {
        withAnimation(.snappy(duration: 0.25)) { model.pinned.toggle() }
        UserDefaults.standard.set(model.pinned, forKey: Keys.pinned)
    }

    private func dragChanged() {
        guard !model.pinned else { return }
        let mouse = NSEvent.mouseLocation
        if drag == nil {
            hoverTask?.cancel()
            if state != .floating { leaveDock() }
            drag = (mouse, panel.frame.origin)
        }
        guard let drag else { return }
        panel.setFrameOrigin(NSPoint(x: drag.origin.x + mouse.x - drag.mouse.x,
                                     y: drag.origin.y + mouse.y - drag.mouse.y))
    }

    private func dragEnded() {
        guard drag != nil else { return }
        drag = nil
        let glass = panel.frame.insetBy(dx: PanelMetrics.margin, dy: PanelMetrics.margin)
        let screen = activeScreen.frame
        if glass.maxX >= screen.maxX - 1 || glass.minX <= screen.minX + 1 {
            // 拖到屏幕边上就收起；展开时回到屏幕内离这里最近的位置
            floatingTopLeft = clamped(topLeft)
            tuck(rememberPosition: false)
        } else {
            let target = clamped(topLeft)
            if target != topLeft { animate(to: frame(topLeft: target)) }
            remember(target)
        }
    }

    /// 移到屏幕的某个角。固定状态下也可以用，移过去之后仍然固定。
    func move(to corner: Corner) {
        if state != .floating { leaveDock() }
        let visible = activeScreen.visibleFrame
        let margin = PanelMetrics.margin, inset: CGFloat = 12
        let x = corner.isRight ? visible.maxX - inset - PanelMetrics.width - margin : visible.minX + inset - margin
        let y = corner.isTop ? visible.maxY - inset + margin : visible.minY + inset + contentHeight + margin
        let target = NSPoint(x: x, y: y)
        floatingTopLeft = target
        remember(target)
        if panel.isVisible {
            animate(to: frame(topLeft: target))
        } else {
            show()
        }
    }

    private func remember(_ point: NSPoint) {
        floatingTopLeft = point
        UserDefaults.standard.set(Double(point.x), forKey: Keys.x)
        UserDefaults.standard.set(Double(point.y), forKey: Keys.y)
    }

    // MARK: 贴边收起

    func toggleTuck() {
        state == .floating ? tuck() : untuck()
    }

    func tuck(rememberPosition: Bool = true, animated: Bool = true) {
        guard state == .floating else { return }
        let screen = activeScreen
        if rememberPosition { floatingTopLeft = topLeft }
        dockScreen = screen
        edge = panel.frame.midX >= screen.frame.midX ? .right : .left
        state = .tucked
        model.tuckedEdge = edge
        UserDefaults.standard.set(true, forKey: Keys.tucked)
        onCollapse?()
        moveHorizontally(to: dockedX(tucked: true), animated: animated)
    }

    func untuck() {
        guard state != .floating else { return }
        leaveDock()
        let target = clamped(floatingTopLeft ?? topLeft)
        floatingTopLeft = target
        animate(to: frame(topLeft: target))
    }

    private func leaveDock() {
        state = .floating
        model.tuckedEdge = nil
        UserDefaults.standard.set(false, forKey: Keys.tucked)
    }

    private func peek() {
        guard state == .tucked else { return }
        state = .peeking
        model.tuckedEdge = nil
        moveHorizontally(to: dockedX(tucked: false), animated: true)
    }

    private func unpeek() {
        guard state == .peeking else { return }
        state = .tucked
        model.tuckedEdge = edge
        moveHorizontally(to: dockedX(tucked: true), animated: true)
    }

    /// 收起状态下探出来给你看一眼保存结果，再缩回去
    private func peekBriefly(for duration: Duration) {
        guard panel.isVisible, state != .floating else { return }
        peek()
        hoverTask?.cancel()
        hoverTask = Task { [weak self] in
            try? await Task.sleep(for: duration)
            guard let self, !Task.isCancelled, !self.mouseInside else { return }
            self.unpeek()
        }
    }

    private func dropHoverChanged(_ hover: DropHover) {
        guard hover != model.dropHover else { return }
        withAnimation(.snappy(duration: 0.25)) { model.dropHover = hover }
        hoverTask?.cancel()
        if hover != .none {
            peek()
        } else if state == .peeking {
            // 松手保存时 show(_:) 会接着处理；没松手就离开时缩回去
            hoverTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(600))
                guard let self, !Task.isCancelled, !self.mouseInside else { return }
                self.unpeek()
            }
        }
    }

    /// 测试用：不经过真实拖拽，直接走松手后的流程
    func simulateDrop(_ content: DroppedContent, action: DropAction = .save) {
        dropHandler?(content, action)
    }

    /// 测试用
    func setDropHover(_ hover: DropHover) { dropHoverChanged(hover) }

    /// 测试用：把悬浮窗内容画成 PNG。ImageRenderer 画不出玻璃效果，底下垫一块浅灰色代替
    func snapshot(to folder: URL) {
        guard let actions else { return }
        model.snapshotMode = true
        defer { model.snapshotMode = false }
        for dark in [false, true] {
            let view = PanelView(store: store, history: history, model: model, actions: actions)
                .frame(width: PanelMetrics.width + PanelMetrics.margin * 2)
                .background(dark ? Color(white: 0.18) : Color(white: 0.9))
                .environment(\.colorScheme, dark ? .dark : .light)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            guard let image = renderer.cgImage,
                  let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { continue }
            try? png.write(to: folder.appending(path: dark ? "panel-dark.png" : "panel-light.png"))
        }
    }

    private func hoverChanged(_ inside: Bool) {
        mouseInside = inside
        if inside {
            refreshHint()
            updatePasteTip()
            store.refreshClock()
            store.reloadIfStale()
        }
        guard drag == nil else { return }
        hoverTask?.cancel()
        if inside {
            guard state == .tucked else { return }
            // 稍微停一下再探出，鼠标只是路过屏幕边时不打扰
            hoverTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(120))
                guard let self, !Task.isCancelled, self.mouseInside else { return }
                self.peek()
            }
        } else if state == .peeking {
            hoverTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(350))
                guard let self, !Task.isCancelled, !self.mouseInside else { return }
                self.unpeek()
            }
        }
    }

    // MARK: 行操作

    private func copy(_ note: SavedNote) -> Bool {
        guard FileManager.default.fileExists(atPath: note.url.path(percentEncoded: false)) else {
            store.noteMissing()
            return false
        }
        Pasteboard.writeFile(note.url)
        refreshHint(force: true)
        return true
    }

    private func reveal(_ note: SavedNote) {
        NSWorkspace.shared.activateFileViewerSelecting([note.url])
    }

    // MARK: 标签与剪贴板记录

    func select(_ tab: PanelTab) {
        model.tab = tab
        UserDefaults.standard.set(tab.rawValue, forKey: Keys.tab)
        if tab == .clipboard { updatePasteTip() }
    }

    private func removeClip(_ item: ClipItem) {
        guard let index = history.items.firstIndex(where: { $0.id == item.id }) else { return }
        history.remove(item)
        offerUndo("已从剪贴板记录里删除") { [weak self] in self?.history.restore(item, at: index) }
    }

    private func clearClips() {
        let removed = history.items
        guard !removed.isEmpty else { return }
        history.clear()
        offerUndo("已清空 \(removed.count) 条剪贴板记录") { [weak self] in
            guard let self else { return }
            for (index, item) in removed.enumerated() { self.history.restore(item, at: index) }
        }
    }

    // MARK: 删除与撤销

    func hide(_ note: SavedNote) {
        store.hide(note)
        Log.app.notice("hid \(note.url.lastPathComponent, privacy: .public)")
        offerUndo("已从列表移除，文件还在") { [weak self] in self?.store.unhide(note) }
    }

    func trash(_ note: SavedNote) {
        do {
            let trashed = try store.trash(note)
            Log.app.notice("trashed \(note.url.lastPathComponent, privacy: .public)")
            onTrashed(note.url)
            offerUndo("已移到废纸篓") { [weak self] in
                do {
                    try self?.store.restore(note, from: trashed)
                } catch {
                    self?.show(.failed(Failure(title: "没能从废纸篓恢复",
                                               detail: "可以到废纸篓里找到它：\(trashed.lastPathComponent)")))
                }
            }
        } catch {
            Log.app.error("trash failed: \(String(describing: error), privacy: .public)")
            show(.failed(Failure(title: "没能移到废纸篓", detail: error.localizedDescription)))
        }
    }

    func offerUndo(_ message: String, undo: @escaping @MainActor () -> Void) {
        undoAction = undo
        noticeTask?.cancel()
        withAnimation(.snappy(duration: 0.3)) {
            model.failure = nil
            model.notice = Notice(message: message)
        }
        noticeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(6))
            guard let self, !Task.isCancelled else { return }
            self.dismissNotice()
        }
    }

    func undo() {
        let action = undoAction
        dismissNotice()
        action?()
    }

    private func dismissNotice() {
        noticeTask?.cancel()
        undoAction = nil
        withAnimation(.smooth(duration: 0.3)) { model.notice = nil }
    }

    // MARK: 窗口尺寸与位置

    private func contentHeightChanged(_ height: CGFloat) {
        let height = ceil(height)
        measuredHeight = height
        // 任何新的高度都先作废还没执行的缩小：快速来回切换标签时，
        // 旧的缩小会在内容又变高之后才执行，把内容截掉
        shrinkTask?.cancel()
        shrinkTask = nil
        guard abs(height - contentHeight) > 0.5 else { return }
        if height > contentHeight {
            // 变高：窗口立刻变高，玻璃在里面用弹簧动画长出来
            contentHeight = height
            applyContentHeight()
        } else {
            // 变矮：等玻璃收完再缩窗口，否则会被截断。缩到那时量到的最新高度
            shrinkTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(500))
                guard let self, !Task.isCancelled, self.measuredHeight < self.contentHeight else { return }
                self.contentHeight = self.measuredHeight
                self.applyContentHeight()
            }
        }
    }

    private func applyContentHeight() {
        var frame = panel.frame
        let top = frame.maxY
        frame.size = windowSize
        frame.origin.y = top - frame.height
        // 放在屏幕下方时，变高不能把底部挤出屏幕
        let bottomLimit = activeScreen.visibleFrame.minY + 8 - PanelMetrics.margin
        if frame.minY < bottomLimit { frame.origin.y = bottomLimit }
        panel.setFrame(frame, display: true)
        if state == .floating, drag == nil, floatingTopLeft != nil { floatingTopLeft = topLeft }
    }

    private var windowSize: NSSize {
        NSSize(width: PanelMetrics.width + PanelMetrics.margin * 2, height: contentHeight + PanelMetrics.margin * 2)
    }

    private var topLeft: NSPoint { NSPoint(x: panel.frame.minX, y: panel.frame.maxY) }

    /// 收起时窗口大部分在屏幕外，用收起时记下的屏幕
    private var activeScreen: NSScreen {
        (state != .floating ? dockScreen : nil) ?? panel.screen ?? NSScreen.main ?? NSScreen.screens[0]
    }

    private func frame(topLeft: NSPoint) -> NSRect {
        NSRect(x: topLeft.x, y: topLeft.y - windowSize.height, width: windowSize.width, height: windowSize.height)
    }

    private func dockedX(tucked: Bool) -> CGFloat {
        let screen = activeScreen.frame
        let margin = PanelMetrics.margin, width = PanelMetrics.width
        switch (edge, tucked) {
        case (.right, true): return screen.maxX - PanelMetrics.sliver - margin
        case (.right, false): return screen.maxX - PanelMetrics.peekInset - width - margin
        case (.left, true): return screen.minX + PanelMetrics.sliver - width - margin
        case (.left, false): return screen.minX + PanelMetrics.peekInset - margin
        }
    }

    private func moveHorizontally(to x: CGFloat, animated: Bool) {
        var target = panel.frame
        target.origin.x = x
        if animated {
            animate(to: target)
        } else {
            panel.setFrame(target, display: true)
        }
    }

    private func animate(to target: NSRect) {
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        NSAnimationContext.runAnimationGroup { context in
            context.duration = reduceMotion ? 0 : 0.38
            context.timingFunction = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.25, 1)
            panel.animator().setFrame(target, display: true)
        }
    }

    /// 让玻璃完整地留在屏幕可见区域里
    private func clamped(_ point: NSPoint) -> NSPoint {
        let screen = NSScreen.screens.first { $0.frame.contains(point) } ?? activeScreen
        let visible = screen.visibleFrame
        let margin = PanelMetrics.margin, inset: CGFloat = 8
        let x = min(max(point.x, visible.minX + inset - margin), visible.maxX - inset - PanelMetrics.width - margin)
        let y = min(max(point.y, visible.minY + inset + contentHeight + margin), visible.maxY - inset + margin)
        return NSPoint(x: x, y: y)
    }

    private func savedTopLeft() -> NSPoint? {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: Keys.x) != nil else { return nil }
        return NSPoint(x: defaults.double(forKey: Keys.x), y: defaults.double(forKey: Keys.y))
    }

    private func defaultTopLeft() -> NSPoint {
        let visible = (NSScreen.main ?? NSScreen.screens[0]).visibleFrame
        return NSPoint(x: visible.maxX - PanelMetrics.width - PanelMetrics.margin - 24, y: visible.maxY - 60)
    }
}
