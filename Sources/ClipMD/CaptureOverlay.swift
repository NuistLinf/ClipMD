import AppKit
import ScreenCaptureKit
import SwiftUI

/// 截图的结果
enum CaptureResult {
    /// 选区（连同标注）放到剪贴板
    case copy(CGImage)
    /// 钉在桌面：图片和它在屏幕上的位置（全局 AppKit 坐标）
    case pin(CGImage, frame: NSRect)
    /// 识别选区里的文字
    case recognize(CGImage)
    /// 按 C 复制的色号
    case color(String)
    case cancelled
}

/// 一次截图：每块屏幕盖一个全屏的截图层，画面是开始时冻结下来的截图。
/// 选区只在一块屏幕上；结束时窗口全部关掉，冻结的截图跟着释放。
@MainActor
final class CaptureSession {
    /// 「截图并识别文字」：选好区域就直接识别，不出工具栏
    let recognizeOnly: Bool
    let tools = CaptureToolState()
    private(set) var overlays: [CaptureOverlay] = []
    private let onFinish: (CaptureResult) -> Void
    private var previousApp: NSRunningApplication?
    private var finished = false
    /// 测试时不显示、不挡鼠标、不抢焦点，只用测试命令驱动
    private let hiddenForTesting: Bool

    init(screens: [FrozenScreen], recognizeOnly: Bool, hiddenForTesting: Bool = false, onFinish: @escaping (CaptureResult) -> Void) {
        self.recognizeOnly = recognizeOnly
        self.hiddenForTesting = hiddenForTesting
        self.onFinish = onFinish
        overlays = screens.map { CaptureOverlay(frozen: $0, session: self) }
    }

    func begin() {
        if hiddenForTesting {
            for overlay in overlays {
                overlay.alphaValue = 0
                overlay.ignoresMouseEvents = true
                overlay.canvas.ignoresRealPointer = true
                overlay.orderFrontRegardless()
            }
            return
        }
        let front = NSWorkspace.shared.frontmostApplication
        previousApp = front?.processIdentifier == ProcessInfo.processInfo.processIdentifier ? nil : front
        for overlay in overlays { overlay.orderFrontRegardless() }
        // 激活才能改鼠标指针、收到按键；结束时把焦点还给原来的应用
        NSApp.activate()
        let mouse = NSEvent.mouseLocation
        let current = overlays.first { NSMouseInRect(mouse, $0.frame, false) } ?? overlays.first
        current?.makeKey()
        for overlay in overlays { overlay.canvas.start(mouse: mouse) }
    }

    /// 选区开始后，其他屏幕只保留变暗，不再响应
    func selectionStarted(on canvas: CaptureCanvas) {
        for overlay in overlays where overlay.canvas !== canvas { overlay.canvas.becomePassive() }
    }

    func finish(_ result: CaptureResult) {
        guard !finished else { return }
        finished = true
        for overlay in overlays { overlay.close() }
        overlays.removeAll()
        NSCursor.arrow.set()
        if let previousApp {
            NSApp.yieldActivation(to: previousApp)
            previousApp.activate()
        }
        onFinish(result)
    }

    /// 正在选区的那块屏幕；还没开始选时是鼠标所在的那块
    var activeCanvas: CaptureCanvas? {
        overlays.first { $0.canvas.isSelecting }?.canvas
            ?? overlays.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) }?.canvas
            ?? overlays.first?.canvas
    }
}

/// 盖住一整块屏幕的截图窗口（在菜单栏和程序坞之上）
final class CaptureOverlay: NSPanel {
    let canvas: CaptureCanvas

    init(frozen: FrozenScreen, session: CaptureSession) {
        canvas = CaptureCanvas(frozen: frozen, session: session)
        super.init(contentRect: frozen.frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .screenSaver
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        isOpaque = true
        backgroundColor = .black
        hasShadow = false
        isReleasedWhenClosed = false
        animationBehavior = .none
        hidesOnDeactivate = false
        acceptsMouseMovedEvents = true

        let container = FlippedView(frame: NSRect(origin: .zero, size: frozen.frame.size))
        canvas.frame = container.bounds
        canvas.autoresizingMask = [.width, .height]
        container.addSubview(canvas)
        contentView = container
        setFrame(frozen.frame, display: false)
        makeFirstResponder(canvas)
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// 点过工具栏后焦点可能不在截图层上，按键在这里兜底
    override func keyDown(with event: NSEvent) {
        if !canvas.handleKey(event.charactersIgnoringModifiers ?? "", keyCode: event.keyCode, modifiers: event.modifierFlags) {
            super.keyDown(with: event)
        }
    }

    /// NSPanel 默认按 Esc 会直接关窗口，这里改成正常地结束截图
    override func cancelOperation(_ sender: Any?) {
        canvas.handleKey("", keyCode: 53)
    }
}

private final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// 截图层：冻结的画面、变暗的遮罩、选区、分辨率、放大镜和标注都用 Core Animation 图层画，
/// 鼠标移动时只改图层属性，不重画整块屏幕。坐标单位是点，原点在屏幕左上角。
final class CaptureCanvas: NSView {
    private enum Phase {
        /// 鼠标下的窗口自动选中；按下后拖动就是自己框选
        case choosing
        case pressing(CGPoint)
        case dragging(CGPoint)
        /// 选好了：可以拖动、调整选区，或者用工具栏标注
        case editing
        /// 选区在别的屏幕上
        case passive
    }

    private enum Interaction {
        case none
        case moving(start: CGPoint, original: CGRect)
        case resizing(Handle, original: CGRect)
        case drawing
    }

    enum Handle: CaseIterable {
        case topLeft, top, topRight, right, bottomRight, bottom, bottomLeft, left
    }

    private struct Annotation {
        var mark: CaptureMark
        let layer: CAShapeLayer
    }

    let frozen: FrozenScreen
    private weak var session: CaptureSession?
    private let scale: CGFloat
    private var phase: Phase = .choosing
    private var interaction: Interaction = .none
    private(set) var selection: CGRect = .zero
    private var pointer: CGPoint?

    private var annotations: [Annotation] = []
    private var drawing: Annotation?
    private var drawingStart: CGPoint = .zero

    private let root = CALayer()
    private let imageLayer = CALayer()
    /// 标注放在这一层，裁到选区里
    private let annotationLayer = CALayer()
    private var mosaicLayer: CALayer?
    private let mosaicMask = CALayer()
    private var pixelated: CGImage?
    private let dimLayer = CAShapeLayer()
    private let borderLayer = CAShapeLayer()
    private let handlesLayer = CAShapeLayer()
    private let sizeBadge = CALayer()
    private let sizeText = CATextLayer()
    private let loupe: Loupe
    private var toolbar: CaptureToolbarHost?
    /// 测试时只听测试命令，不管真实鼠标
    var ignoresRealPointer = false

    init(frozen: FrozenScreen, session: CaptureSession) {
        self.frozen = frozen
        self.session = session
        scale = frozen.scale
        loupe = Loupe(contentsScale: frozen.screen.backingScaleFactor)
        super.init(frame: NSRect(origin: .zero, size: frozen.frame.size))
        // 自己管理图层（layer-hosting），和视图一样以左上角为原点
        root.isGeometryFlipped = true
        layer = root
        wantsLayer = true
        setUpLayers()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    var isSelecting: Bool {
        switch phase {
        case .pressing, .dragging, .editing: true
        case .choosing, .passive: false
        }
    }

    private var isEditing: Bool {
        if case .editing = phase { return true }
        return false
    }

    private func setUpLayers() {
        let bounds = CGRect(origin: .zero, size: frozen.frame.size)
        let accent = NSColor.controlAccentColor.cgColor
        let contentsScale = frozen.screen.backingScaleFactor

        imageLayer.frame = bounds
        imageLayer.contents = frozen.image
        imageLayer.contentsGravity = .resize

        annotationLayer.masksToBounds = true

        dimLayer.frame = bounds
        dimLayer.fillColor = NSColor.black.withAlphaComponent(0.42).cgColor
        dimLayer.fillRule = .evenOdd
        dimLayer.path = CGPath(rect: bounds, transform: nil)

        borderLayer.frame = bounds
        borderLayer.fillColor = nil
        borderLayer.strokeColor = accent
        borderLayer.lineWidth = 1.5
        borderLayer.isHidden = true

        handlesLayer.frame = bounds
        handlesLayer.fillColor = NSColor.white.cgColor
        handlesLayer.strokeColor = accent
        handlesLayer.lineWidth = 1.5
        handlesLayer.isHidden = true

        sizeBadge.backgroundColor = NSColor(white: 0, alpha: 0.72).cgColor
        sizeBadge.cornerRadius = 5
        sizeBadge.isHidden = true
        sizeText.contentsScale = contentsScale
        sizeBadge.addSublayer(sizeText)

        for layer in [imageLayer, annotationLayer, dimLayer, borderLayer, handlesLayer, sizeBadge, loupe.layer] {
            root.addSublayer(layer)
        }
    }

    // MARK: 开始和结束

    func start(mouse: NSPoint) {
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0
        fade.toValue = 1
        fade.duration = 0.18
        dimLayer.add(fade, forKey: "fade")
        let point = local(mouse)
        if bounds.contains(point) { pointerMoved(to: point) }
    }

    func becomePassive() {
        phase = .passive
        clearHighlight()
    }

    private func local(_ global: NSPoint) -> CGPoint {
        CGPoint(x: global.x - frozen.frame.minX, y: frozen.frame.maxY - global.y)
    }

    private func clearHighlight() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        selection = .zero
        dimLayer.path = CGPath(rect: bounds, transform: nil)
        borderLayer.isHidden = true
        sizeBadge.isHidden = true
        handlesLayer.isHidden = true
        loupe.layer.isHidden = true
        CATransaction.commit()
        toolbar?.isHidden = true
    }

    // MARK: 鼠标

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .cursorUpdate, .activeAlways, .inVisibleRect],
                                       owner: self))
    }

    override func mouseMoved(with event: NSEvent) {
        guard !ignoresRealPointer else { return }
        pointerMoved(to: convert(event.locationInWindow, from: nil))
    }
    override func mouseDown(with event: NSEvent) {
        pointerDown(at: convert(event.locationInWindow, from: nil), clickCount: event.clickCount)
    }
    override func mouseDragged(with event: NSEvent) {
        pointerDragged(to: convert(event.locationInWindow, from: nil), constrained: event.modifierFlags.contains(.shift))
    }
    override func mouseUp(with event: NSEvent) { pointerUp(at: convert(event.locationInWindow, from: nil)) }
    override func rightMouseDown(with event: NSEvent) { secondaryClick() }
    override func cursorUpdate(with event: NSEvent) { updateCursor() }

    override func mouseEntered(with event: NSEvent) {
        window?.makeKey()
    }

    override func mouseExited(with event: NSEvent) {
        guard !ignoresRealPointer else { return }
        pointer = nil
        // 多块屏幕时，鼠标移到别的屏幕上，这块就不再高亮
        if case .choosing = phase { clearHighlight() }
    }

    /// 鼠标移动（测试命令也走这里）
    func pointerMoved(to point: CGPoint) {
        pointer = point
        switch phase {
        case .choosing:
            let rect = windowRect(at: point)
            if rect != selection { showSelection(rect, animated: !selection.isEmpty) }
            updateLoupe()
        case .editing:
            break
        case .pressing, .dragging, .passive:
            break
        }
        updateCursor()
    }

    func pointerDown(at point: CGPoint, clickCount: Int) {
        pointer = point
        switch phase {
        case .choosing:
            phase = .pressing(point)
            session?.selectionStarted(on: self)
        case .editing:
            if clickCount >= 2, selection.contains(point) {
                confirm()
                return
            }
            if let tool = session?.tools.tool {
                if selection.contains(point) { beginDrawing(tool, at: point) }
            } else if let handle = handle(at: point) {
                interaction = .resizing(handle, original: selection)
                toolbar?.isHidden = true
                updateLoupe()
            } else if selection.contains(point) {
                interaction = .moving(start: point, original: selection)
                toolbar?.isHidden = true
            }
        case .pressing, .dragging, .passive:
            break
        }
        updateCursor()
    }

    func pointerDragged(to point: CGPoint, constrained: Bool = false) {
        pointer = point
        switch phase {
        case .pressing(let start):
            guard hypot(point.x - start.x, point.y - start.y) > 3 else { return }
            phase = .dragging(start)
            pointerDragged(to: point, constrained: constrained)
            return
        case .dragging(let start):
            showSelection(aligned(rect(from: start, to: point)), animated: false)
            updateLoupe()
        case .editing:
            switch interaction {
            case .moving(let start, let original):
                var moved = original.offsetBy(dx: point.x - start.x, dy: point.y - start.y)
                moved.origin.x = min(max(moved.minX, 0), bounds.width - moved.width)
                moved.origin.y = min(max(moved.minY, 0), bounds.height - moved.height)
                showSelection(aligned(moved), animated: false)
            case .resizing(let handle, let original):
                showSelection(aligned(resized(original, handle: handle, to: point)), animated: false)
                updateLoupe()
            case .drawing:
                continueDrawing(to: point, constrained: constrained)
            case .none:
                break
            }
        case .choosing, .passive:
            break
        }
    }

    func pointerUp(at point: CGPoint) {
        pointer = point
        switch phase {
        case .pressing:
            // 没拖动就是点了一下：选中鼠标下的窗口（已经高亮的那块）
            enterEditing()
        case .dragging(let start):
            let dragged = aligned(rect(from: start, to: point))
            if dragged.width < 3 || dragged.height < 3 { showSelection(windowRect(at: start), animated: false) }
            enterEditing()
        case .editing:
            switch interaction {
            case .drawing: endDrawing()
            case .moving, .resizing:
                loupe.layer.isHidden = true
                showToolbar()
            case .none: break
            }
            interaction = .none
        case .choosing, .passive:
            break
        }
        updateCursor()
    }

    /// 右键：还没选好时退出截图；选好了、还没标注时回到重新选择
    private func secondaryClick() {
        switch phase {
        case .choosing, .pressing, .dragging, .passive:
            session?.finish(.cancelled)
        case .editing:
            guard annotations.isEmpty else { return }
            session?.tools.tool = nil
            phase = .choosing
            toolbar?.isHidden = true
            handlesLayer.isHidden = true
            if let pointer { showSelection(windowRect(at: pointer), animated: true) }
            updateLoupe()
            updateCursor()
        }
    }

    private func enterEditing() {
        phase = .editing
        loupe.layer.isHidden = true
        if session?.recognizeOnly == true {
            finish { .recognize($0) }
            return
        }
        handlesLayer.isHidden = session?.tools.tool != nil
        showToolbar()
    }

    private func updateCursor() {
        guard let pointer else { return }
        switch phase {
        case .choosing, .pressing, .dragging:
            NSCursor.crosshair.set()
        case .passive:
            NSCursor.arrow.set()
        case .editing:
            switch interaction {
            case .moving: NSCursor.closedHand.set()
            case .resizing(let handle, _): handle.cursor.set()
            case .drawing: NSCursor.crosshair.set()
            case .none:
                if session?.tools.tool != nil {
                    (selection.contains(pointer) ? NSCursor.crosshair : NSCursor.arrow).set()
                } else if let handle = handle(at: pointer) {
                    handle.cursor.set()
                } else {
                    (selection.contains(pointer) ? NSCursor.openHand : NSCursor.arrow).set()
                }
            }
        }
    }

    // MARK: 按键

    /// 主菜单「编辑」里的 ⌘C、⌘Z 会先找到这里
    @objc func copy(_ sender: Any?) {
        if isEditing { confirm() }
    }

    @objc func undo(_ sender: Any?) {
        undo()
    }

    override func keyDown(with event: NSEvent) {
        if !handleKey(event.charactersIgnoringModifiers ?? "", keyCode: event.keyCode, modifiers: event.modifierFlags) {
            super.keyDown(with: event)
        }
    }

    /// Esc 退出、回车完成、C 复制色号（⇧C 复制 RGB）、⌘Z 撤销、R/O/M 切换标注工具、T 识别文字、P 钉在桌面
    @discardableResult
    func handleKey(_ characters: String, keyCode: UInt16 = 0, modifiers: NSEvent.ModifierFlags = []) -> Bool {
        let flags = modifiers.intersection(.deviceIndependentFlagsMask)
        let key = characters.lowercased()
        switch keyCode {
        case 53:
            session?.finish(.cancelled)
            return true
        case 36, 76:
            switch phase {
            case .editing: confirm()
            case .choosing, .pressing: enterEditing()
            case .dragging, .passive: break
            }
            return true
        default:
            break
        }
        if flags.contains(.command) {
            switch key {
            case "z": undo()
            case "c" where isEditing: confirm()
            default: return false
            }
            return true
        }
        if key == "c", !loupe.layer.isHidden, let color = loupe.color {
            session?.finish(.color(flags.contains(.shift) ? color.rgbText : color.hexText))
            return true
        }
        guard isEditing, session?.recognizeOnly == false else { return false }
        switch key {
        case "r": perform(.tool(.rectangle))
        case "o", "e": perform(.tool(.ellipse))
        case "m": perform(.tool(.mosaic))
        case "t": perform(.recognize)
        case "p": perform(.pin)
        default: return false
        }
        return true
    }

    // MARK: 选区

    /// 鼠标下最上面的窗口；没有窗口（桌面）时是整块屏幕
    private func windowRect(at point: CGPoint) -> CGRect {
        frozen.windows.first { $0.contains(point) } ?? bounds
    }

    private func rect(from start: CGPoint, to end: CGPoint) -> CGRect {
        let end = CGPoint(x: min(max(end.x, 0), bounds.width), y: min(max(end.y, 0), bounds.height))
        return CGRect(x: min(start.x, end.x), y: min(start.y, end.y), width: abs(end.x - start.x), height: abs(end.y - start.y))
    }

    /// 对齐到像素，截出来的图边缘不会糊
    private func aligned(_ rect: CGRect) -> CGRect {
        let minX = (rect.minX * scale).rounded() / scale, minY = (rect.minY * scale).rounded() / scale
        let maxX = (rect.maxX * scale).rounded() / scale, maxY = (rect.maxY * scale).rounded() / scale
        return CGRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY)
    }

    private func showSelection(_ rect: CGRect, animated: Bool) {
        selection = rect
        CATransaction.begin()
        CATransaction.setDisableActions(!animated)
        if animated {
            CATransaction.setAnimationDuration(0.12)
            CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeOut))
        }
        let dim = CGMutablePath()
        dim.addRect(bounds)
        dim.addRect(rect)
        dimLayer.path = dim
        // 边框画在选区外面，不挡住选区边上的像素
        borderLayer.path = CGPath(rect: rect.insetBy(dx: -0.75, dy: -0.75), transform: nil)
        borderLayer.isHidden = false
        updateSizeBadge(for: rect)
        CATransaction.commit()

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        annotationLayer.frame = rect
        annotationLayer.bounds = rect
        handlesLayer.path = handlesPath(for: rect)
        CATransaction.commit()
        if isEditing, case .none = interaction { layoutToolbar() }
    }

    /// 左上角的分辨率：实际截出来的图片有多少像素
    private func updateSizeBadge(for rect: CGRect) {
        let text = "\(Int((rect.width * scale).rounded())) × \(Int((rect.height * scale).rounded()))"
        let attributed = NSAttributedString(string: text, attributes: [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium),
            .foregroundColor: NSColor.white,
        ])
        let textSize = attributed.size()
        let size = CGSize(width: ceil(textSize.width) + 14, height: 22)
        var origin = CGPoint(x: rect.minX, y: rect.minY - size.height - 6)
        if origin.y < 4 { origin = CGPoint(x: rect.minX + 6, y: rect.minY + 6) }
        origin.x = min(max(origin.x, 4), bounds.width - size.width - 4)
        sizeBadge.frame = CGRect(origin: origin, size: size)
        sizeText.frame = CGRect(x: 7, y: (size.height - ceil(textSize.height)) / 2, width: ceil(textSize.width) + 1, height: ceil(textSize.height))
        sizeText.string = attributed
        sizeBadge.isHidden = false
    }

    private func handlePoint(_ handle: Handle, in rect: CGRect) -> CGPoint {
        switch handle {
        case .topLeft: CGPoint(x: rect.minX, y: rect.minY)
        case .top: CGPoint(x: rect.midX, y: rect.minY)
        case .topRight: CGPoint(x: rect.maxX, y: rect.minY)
        case .right: CGPoint(x: rect.maxX, y: rect.midY)
        case .bottomRight: CGPoint(x: rect.maxX, y: rect.maxY)
        case .bottom: CGPoint(x: rect.midX, y: rect.maxY)
        case .bottomLeft: CGPoint(x: rect.minX, y: rect.maxY)
        case .left: CGPoint(x: rect.minX, y: rect.midY)
        }
    }

    private func handlesPath(for rect: CGRect) -> CGPath {
        let path = CGMutablePath()
        for handle in Handle.allCases {
            let center = handlePoint(handle, in: rect)
            path.addEllipse(in: CGRect(x: center.x - 3.5, y: center.y - 3.5, width: 7, height: 7))
        }
        return path
    }

    /// 鼠标在哪个控制点或哪条边上
    private func handle(at point: CGPoint) -> Handle? {
        guard !selection.isEmpty else { return nil }
        if let corner = Handle.allCases.first(where: {
            let center = handlePoint($0, in: selection)
            return hypot(center.x - point.x, center.y - point.y) <= 8
        }) { return corner }
        let tolerance: CGFloat = 5
        let insideX = point.x > selection.minX && point.x < selection.maxX
        let insideY = point.y > selection.minY && point.y < selection.maxY
        if insideX, abs(point.y - selection.minY) <= tolerance { return .top }
        if insideX, abs(point.y - selection.maxY) <= tolerance { return .bottom }
        if insideY, abs(point.x - selection.minX) <= tolerance { return .left }
        if insideY, abs(point.x - selection.maxX) <= tolerance { return .right }
        return nil
    }

    private func resized(_ original: CGRect, handle: Handle, to point: CGPoint) -> CGRect {
        let point = CGPoint(x: min(max(point.x, 0), bounds.width), y: min(max(point.y, 0), bounds.height))
        var minX = original.minX, maxX = original.maxX, minY = original.minY, maxY = original.maxY
        switch handle {
        case .topLeft: minX = point.x; minY = point.y
        case .top: minY = point.y
        case .topRight: maxX = point.x; minY = point.y
        case .right: maxX = point.x
        case .bottomRight: maxX = point.x; maxY = point.y
        case .bottom: maxY = point.y
        case .bottomLeft: minX = point.x; maxY = point.y
        case .left: minX = point.x
        }
        let rect = CGRect(x: min(minX, maxX), y: min(minY, maxY), width: abs(maxX - minX), height: abs(maxY - minY))
        return CGRect(x: rect.minX, y: rect.minY, width: max(rect.width, 1 / scale), height: max(rect.height, 1 / scale))
    }

    // MARK: 放大镜

    private func updateLoupe() {
        guard let pointer else {
            loupe.layer.isHidden = true
            return
        }
        let x = min(max(Int(pointer.x * scale), 0), frozen.image.width - 1)
        let y = min(max(Int(pointer.y * scale), 0), frozen.image.height - 1)
        loupe.update(image: frozen.image, pixelX: x, pixelY: y, near: pointer, in: bounds)
    }

    // MARK: 工具栏

    private func showToolbar() {
        guard let session, !session.recognizeOnly else { return }
        if toolbar == nil {
            let host = CaptureToolbarHost(rootView: CaptureToolbar(state: session.tools) { [weak self] in self?.perform($0) })
            host.sizingOptions = [.intrinsicContentSize]
            superview?.addSubview(host)
            toolbar = host
            host.alphaValue = 0
            layoutToolbar()
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.18
                host.animator().alphaValue = 1
            }
        }
        toolbar?.isHidden = false
        layoutToolbar()
    }

    /// 选了工具后工具栏多出一行：等 SwiftUI 更新完再量一次大小
    private func relayoutToolbarSoon() {
        DispatchQueue.main.async { [weak self] in self?.layoutToolbar() }
    }

    /// 放在选区右下角下面；下面放不下就放上面，都放不下就放进选区里
    private func layoutToolbar() {
        guard let toolbar, isEditing else { return }
        let size = toolbar.fittingSize
        let gap: CGFloat = 8
        let x = min(max(selection.maxX - size.width, 6), bounds.width - size.width - 6)
        let y: CGFloat
        if selection.maxY + gap + size.height <= bounds.height - 6 {
            y = selection.maxY + gap
        } else if selection.minY - gap - size.height >= 6 {
            y = selection.minY - gap - size.height
        } else {
            y = max(selection.maxY - gap - size.height, 6)
        }
        toolbar.frame = NSRect(x: x, y: y, width: size.width, height: size.height)
    }

    private func perform(_ command: CaptureCommand) {
        guard let session else { return }
        // 焦点留在截图层，Esc、回车、⌘Z 才一直有效
        window?.makeFirstResponder(self)
        switch command {
        case .tool(let tool):
            session.tools.tool = session.tools.tool == tool ? nil : tool
            handlesLayer.isHidden = session.tools.tool != nil
            updateCursor()
            relayoutToolbarSoon()
        case .color(let index): session.tools.colorIndex = index
        case .width(let index): session.tools.widthIndex = index
        case .undo: undo()
        case .recognize: finish { .recognize($0) }
        case .pin:
            let frame = NSRect(x: frozen.frame.minX + selection.minX, y: frozen.frame.maxY - selection.maxY,
                               width: selection.width, height: selection.height)
            finish { .pin($0, frame: frame) }
        case .cancel: session.finish(.cancelled)
        case .confirm: confirm()
        }
    }

    private func confirm() {
        finish { .copy($0) }
    }

    private func finish(_ result: (CGImage) -> CaptureResult) {
        guard let image = renderSelection() else {
            Log.app.error("capture render failed selection=\(String(describing: self.selection), privacy: .public)")
            session?.finish(.cancelled)
            return
        }
        session?.finish(result(image))
    }

    func renderSelection() -> CGImage? {
        CaptureRenderer.render(frozen.image, scale: scale, selection: selection, marks: annotations.map(\.mark), pixelated: pixelated)
    }

    // MARK: 标注

    private func beginDrawing(_ tool: CaptureTool, at point: CGPoint) {
        guard let tools = session?.tools else { return }
        let layer = CAShapeLayer()
        layer.frame = bounds
        layer.lineWidth = tools.width
        layer.fillColor = nil
        layer.actions = ["path": NSNull(), "bounds": NSNull(), "position": NSNull()]
        let mark: CaptureMark
        switch tool {
        case .mosaic:
            ensureMosaicLayer()
            layer.strokeColor = NSColor.black.cgColor
            layer.lineCap = .round
            layer.lineJoin = .round
            mosaicMask.addSublayer(layer)
            mark = CaptureMark(shape: .mosaic([point]), color: NSColor.black.cgColor, width: tools.width)
        case .rectangle, .ellipse:
            layer.strokeColor = tools.color.cgColor
            layer.lineJoin = .miter
            annotationLayer.addSublayer(layer)
            let empty = CGRect(origin: point, size: .zero)
            mark = CaptureMark(shape: tool == .rectangle ? .rectangle(empty) : .ellipse(empty), color: tools.color.cgColor, width: tools.width)
        }
        layer.path = mark.path
        drawing = Annotation(mark: mark, layer: layer)
        drawingStart = point
        interaction = .drawing
    }

    private func continueDrawing(to raw: CGPoint, constrained: Bool) {
        guard var annotation = drawing else { return }
        let point = CGPoint(x: min(max(raw.x, selection.minX), selection.maxX), y: min(max(raw.y, selection.minY), selection.maxY))
        switch annotation.mark.shape {
        case .mosaic(var points):
            if let last = points.last, hypot(last.x - point.x, last.y - point.y) < 1 { return }
            points.append(point)
            annotation.mark.shape = .mosaic(points)
        case .rectangle:
            annotation.mark.shape = .rectangle(shapeRect(to: point, square: constrained))
        case .ellipse:
            annotation.mark.shape = .ellipse(shapeRect(to: point, square: constrained))
        }
        annotation.layer.path = annotation.mark.path
        drawing = annotation
    }

    /// 按住 ⇧ 画正方形、圆
    private func shapeRect(to point: CGPoint, square: Bool) -> CGRect {
        var dx = point.x - drawingStart.x, dy = point.y - drawingStart.y
        if square {
            let side = min(abs(dx), abs(dy))
            dx = dx < 0 ? -side : side
            dy = dy < 0 ? -side : side
        }
        return CGRect(x: min(drawingStart.x, drawingStart.x + dx), y: min(drawingStart.y, drawingStart.y + dy), width: abs(dx), height: abs(dy))
    }

    private func endDrawing() {
        guard let annotation = drawing else { return }
        drawing = nil
        switch annotation.mark.shape {
        case .rectangle(let rect), .ellipse(let rect):
            // 只点了一下，不留
            guard rect.width >= 3 || rect.height >= 3 else {
                annotation.layer.removeFromSuperlayer()
                return
            }
        case .mosaic:
            break
        }
        annotations.append(annotation)
        session?.tools.canUndo = true
    }

    func undo() {
        guard let last = annotations.popLast() else { return }
        last.layer.removeFromSuperlayer()
        session?.tools.canUndo = !annotations.isEmpty
    }

    /// 马赛克：整屏缩小成色块图，放大时不插值；画笔的笔画当作遮罩，画到哪里就露出哪里的色块
    private func ensureMosaicLayer() {
        guard mosaicLayer == nil else { return }
        pixelated = CaptureRenderer.pixelate(frozen.image, blockPixels: Int((CaptureRenderer.mosaicBlock * scale).rounded()))
        let layer = CALayer()
        layer.frame = bounds
        layer.contents = pixelated
        layer.contentsGravity = .resize
        layer.magnificationFilter = .nearest
        mosaicMask.frame = layer.bounds
        layer.mask = mosaicMask
        annotationLayer.insertSublayer(layer, at: 0)
        mosaicLayer = layer
    }

    // MARK: 测试

    var debugState: String {
        let phaseName: String = switch phase {
        case .choosing: "choosing"
        case .pressing: "pressing"
        case .dragging: "dragging"
        case .editing: "editing"
        case .passive: "passive"
        }
        let rect = selection
        return "phase=\(phaseName) selection=\(Int(rect.minX)),\(Int(rect.minY)) \(Int(rect.width))x\(Int(rect.height)) "
            + "badge=\((sizeText.string as? NSAttributedString)?.string ?? "-") loupe=\(loupe.layer.isHidden ? "hidden" : loupe.debugText) "
            + "tool=\(session?.tools.tool?.rawValue ?? "none") marks=\(annotations.count) toolbar=\(toolbar.map { $0.isHidden ? "hidden" : "\(Int($0.frame.minX)),\(Int($0.frame.minY)) \(Int($0.frame.width))x\(Int($0.frame.height))" } ?? "none") "
            + "windows=\(frozen.windows.count) scale=\(scale)"
    }

    /// 测试命令：坐标是截图层里的点，比如 `shot-down?100,200`、`shot-drag?400,500,1`（最后的 1 表示按住 ⇧）
    func handleTest(_ command: String, _ argument: String) {
        let numbers = argument.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
        let point = numbers.count >= 2 ? CGPoint(x: numbers[0], y: numbers[1]) : pointer ?? .zero
        switch command {
        case "/shot-move": pointerMoved(to: point)
        case "/shot-down": pointerDown(at: point, clickCount: numbers.count >= 3 ? Int(numbers[2]) : 1)
        case "/shot-drag": pointerDragged(to: point, constrained: numbers.count >= 3 && numbers[2] == 1)
        case "/shot-up": pointerUp(at: point)
        case "/shot-right": secondaryClick()
        case "/shot-key":
            switch argument {
            case "esc": handleKey("", keyCode: 53)
            case "enter": handleKey("\r", keyCode: 36)
            case "cmd-z": handleKey("z", modifiers: .command)
            case "cmd-c": handleKey("c", modifiers: .command)
            case "C": handleKey("C", modifiers: .shift)
            default: handleKey(argument)
            }
        case "/shot-cmd":
            let parts = argument.split(separator: ":")
            let index = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
            switch parts.first ?? "" {
            case "rect": perform(.tool(.rectangle))
            case "ellipse": perform(.tool(.ellipse))
            case "mosaic": perform(.tool(.mosaic))
            case "color": perform(.color(index))
            case "width": perform(.width(index))
            case "undo": perform(.undo)
            case "ocr": perform(.recognize)
            case "pin": perform(.pin)
            case "cancel": perform(.cancel)
            case "confirm": perform(.confirm)
            default: break
            }
        case "/shot-snapshot": snapshot(to: URL(filePath: argument, directoryHint: .isDirectory))
        case "/shot-screen": captureScreenForTesting(to: URL(filePath: argument))
        default: break
        }
        Log.app.notice("capture \(command, privacy: .public) \(argument, privacy: .public) → \(self.debugState, privacy: .public)")
    }

    /// 真的截一下屏幕上的截图层（要有录制屏幕权限）：看玻璃效果、指针下的实际样子
    private func captureScreenForTesting(to url: URL) {
        guard let window else { return }
        let primaryHeight = NSScreen.screens.first?.frame.height ?? window.frame.height
        let rect = CGRect(x: window.frame.minX, y: primaryHeight - window.frame.maxY, width: window.frame.width, height: window.frame.height)
        let config = SCScreenshotConfiguration()
        config.showsCursor = false
        config.dynamicRange = .sdr
        Task {
            do {
                let output = try await SCScreenshotManager.captureScreenshot(rect: rect, configuration: config)
                if let image = output.sdrImage { CaptureRenderer.writePNG(image, to: url) }
            } catch {
                Log.app.error("test screen capture failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// 把整个截图层画成 PNG（图层树 + 工具栏的排版），再把导出的选区单独存一张
    func snapshot(to folder: URL) {
        let pointScale = frozen.screen.backingScaleFactor
        let width = Int(bounds.width * pointScale), height = Int(bounds.height * pointScale)
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return }
        // 图层树以左上角为原点，位图以左下角为原点
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: pointScale, y: -pointScale)
        root.render(in: context)
        if let toolbar, !toolbar.isHidden, let session {
            let renderer = ImageRenderer(content: CaptureToolbar(state: session.tools) { _ in })
            renderer.scale = pointScale
            if let image = renderer.cgImage {
                context.saveGState()
                context.translateBy(x: toolbar.frame.minX, y: toolbar.frame.maxY)
                context.scaleBy(x: 1, y: -1)
                context.draw(image, in: CGRect(x: 0, y: 0, width: CGFloat(image.width) / pointScale, height: CGFloat(image.height) / pointScale))
                context.restoreGState()
            }
        }
        if let image = context.makeImage() { CaptureRenderer.writePNG(image, to: folder.appending(path: "capture-overlay.png")) }
        if let image = renderSelection() { CaptureRenderer.writePNG(image, to: folder.appending(path: "capture-export.png")) }
    }
}

private extension CaptureCanvas.Handle {
    var cursor: NSCursor {
        let position: NSCursor.FrameResizePosition = switch self {
        case .topLeft: .topLeft
        case .top: .top
        case .topRight: .topRight
        case .right: .right
        case .bottomRight: .bottomRight
        case .bottom: .bottom
        case .bottomLeft: .bottomLeft
        case .left: .left
        }
        return NSCursor.frameResize(position: position, directions: .all)
    }
}

/// 取到的颜色（sRGB，和网页里的颜色值一致）
struct PickedColor {
    let red: UInt8, green: UInt8, blue: UInt8
    var hexText: String { String(format: "#%02X%02X%02X", red, green, blue) }
    var rgbText: String { "rgb(\(red), \(green), \(blue))" }
    var cgColor: CGColor { CGColor(srgbRed: CGFloat(red) / 255, green: CGFloat(green) / 255, blue: CGFloat(blue) / 255, alpha: 1) }
}

/// 放大镜：鼠标周围 17 × 11 个像素，每个像素放大成 10 点见方；下面是坐标、色号和快捷键提示
@MainActor
private final class Loupe {
    static let columns = 17, rows = 11
    static let cell: CGFloat = 10
    static let infoHeight: CGFloat = 64
    static var zoomSize: CGSize { CGSize(width: CGFloat(columns) * cell, height: CGFloat(rows) * cell) }
    static var size: CGSize { CGSize(width: zoomSize.width, height: zoomSize.height + infoHeight) }

    let layer = CALayer()
    private let body = CALayer()
    private let zoom = CALayer()
    private let zoomImage = CALayer()
    private let crosshair = CAShapeLayer()
    private let centerBox = CAShapeLayer()
    private let positionText = CATextLayer()
    private let swatch = CALayer()
    private let colorText = CATextLayer()
    private let hintText = CATextLayer()
    private(set) var color: PickedColor?
    private var position = (x: 0, y: 0)

    var debugText: String { "\(position.x),\(position.y) \(color?.hexText ?? "-")" }

    init(contentsScale: CGFloat) {
        let size = Self.size, zoomSize = Self.zoomSize
        layer.anchorPoint = .zero
        layer.bounds = CGRect(origin: .zero, size: size)
        layer.isHidden = true
        layer.shadowColor = NSColor.black.cgColor
        layer.shadowOpacity = 0.35
        layer.shadowRadius = 10
        layer.shadowOffset = .zero
        layer.shadowPath = CGPath(roundedRect: layer.bounds, cornerWidth: 10, cornerHeight: 10, transform: nil)

        body.frame = layer.bounds
        body.cornerRadius = 10
        body.masksToBounds = true
        body.backgroundColor = NSColor(white: 0.1, alpha: 0.92).cgColor
        body.borderColor = NSColor(white: 1, alpha: 0.18).cgColor
        body.borderWidth = 0.5
        layer.addSublayer(body)

        zoom.frame = CGRect(origin: .zero, size: zoomSize)
        zoom.backgroundColor = NSColor.black.cgColor
        zoom.masksToBounds = true
        body.addSublayer(zoom)

        zoomImage.magnificationFilter = .nearest
        zoomImage.contentsGravity = .resize
        zoom.addSublayer(zoomImage)

        // 十字准线：中间一行、一列淡淡地染上强调色
        let middleColumn = CGFloat(Self.columns / 2) * Self.cell, middleRow = CGFloat(Self.rows / 2) * Self.cell
        let bands = CGMutablePath()
        bands.addRect(CGRect(x: 0, y: middleRow, width: zoomSize.width, height: Self.cell))
        bands.addRect(CGRect(x: middleColumn, y: 0, width: Self.cell, height: zoomSize.height))
        crosshair.frame = zoom.bounds
        crosshair.path = bands
        crosshair.fillRule = .evenOdd
        crosshair.fillColor = NSColor.controlAccentColor.withAlphaComponent(0.28).cgColor
        zoom.addSublayer(crosshair)

        centerBox.frame = zoom.bounds
        centerBox.path = CGPath(rect: CGRect(x: middleColumn, y: middleRow, width: Self.cell, height: Self.cell).insetBy(dx: -0.5, dy: -0.5), transform: nil)
        centerBox.fillColor = nil
        centerBox.strokeColor = NSColor.white.cgColor
        centerBox.lineWidth = 1.5
        centerBox.shadowColor = NSColor.black.cgColor
        centerBox.shadowOpacity = 0.6
        centerBox.shadowRadius = 1
        centerBox.shadowOffset = .zero
        zoom.addSublayer(centerBox)

        let separator = CALayer()
        separator.frame = CGRect(x: 0, y: zoomSize.height, width: zoomSize.width, height: 0.5)
        separator.backgroundColor = NSColor(white: 1, alpha: 0.15).cgColor
        body.addSublayer(separator)

        let top = zoomSize.height
        positionText.frame = CGRect(x: 10, y: top + 7, width: zoomSize.width - 20, height: 16)
        swatch.frame = CGRect(x: 10, y: top + 26, width: 14, height: 14)
        swatch.cornerRadius = 3
        swatch.borderColor = NSColor(white: 1, alpha: 0.6).cgColor
        swatch.borderWidth = 0.5
        colorText.frame = CGRect(x: 30, y: top + 25, width: zoomSize.width - 40, height: 16)
        hintText.frame = CGRect(x: 10, y: top + 45, width: zoomSize.width - 20, height: 14)
        for text in [positionText, colorText, hintText] {
            text.contentsScale = contentsScale
            text.isWrapped = false
            text.truncationMode = .end
            body.addSublayer(text)
        }
        body.addSublayer(swatch)
        hintText.string = NSAttributedString(string: "C 复制色号 · ⇧C 复制 RGB", attributes: [
            .font: NSFont.systemFont(ofSize: 10.5),
            .foregroundColor: NSColor(white: 1, alpha: 0.55),
        ])
    }

    func update(image: CGImage, pixelX: Int, pixelY: Int, near point: CGPoint, in bounds: CGRect) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }

        // 靠近屏幕边缘时，超出屏幕的那部分留黑
        let request = CGRect(x: pixelX - Self.columns / 2, y: pixelY - Self.rows / 2, width: Self.columns, height: Self.rows)
        let available = request.intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
        if !available.isNull, let crop = image.cropping(to: available) {
            zoomImage.contents = crop
            zoomImage.frame = CGRect(x: (available.minX - request.minX) * Self.cell, y: (available.minY - request.minY) * Self.cell,
                                     width: available.width * Self.cell, height: available.height * Self.cell)
            zoomImage.isHidden = false
        } else {
            zoomImage.isHidden = true
        }

        position = (pixelX, pixelY)
        color = Self.sample(image, x: pixelX, y: pixelY)
        let font = NSFont.monospacedDigitSystemFont(ofSize: 11.5, weight: .medium)
        positionText.string = NSAttributedString(string: "坐标  \(pixelX), \(pixelY)", attributes: [.font: font, .foregroundColor: NSColor.white])
        colorText.string = NSAttributedString(string: color?.hexText ?? "—", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 11.5, weight: .semibold), .foregroundColor: NSColor.white,
        ])
        swatch.backgroundColor = color?.cgColor

        // 默认在鼠标右下方，靠近右边或下边时换到另一侧
        let size = Self.size
        var origin = CGPoint(x: point.x + 22, y: point.y + 26)
        if origin.x + size.width > bounds.maxX - 4 { origin.x = point.x - 22 - size.width }
        if origin.y + size.height > bounds.maxY - 4 { origin.y = point.y - 26 - size.height }
        layer.position = origin
        layer.isHidden = false
    }

    /// 读一个像素，转成 sRGB
    private static func sample(_ image: CGImage, x: Int, y: Int) -> PickedColor? {
        guard let pixel = image.cropping(to: CGRect(x: x, y: y, width: 1, height: 1)),
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
              let data = context.data
        else { return nil }
        context.draw(pixel, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        let bytes = data.assumingMemoryBound(to: UInt8.self)
        return PickedColor(red: bytes[0], green: bytes[1], blue: bytes[2])
    }
}

/// 一个标注：坐标是截图层里的点（左上角原点）
struct CaptureMark {
    enum Shape {
        case rectangle(CGRect)
        case ellipse(CGRect)
        /// 马赛克画笔经过的点
        case mosaic([CGPoint])
    }

    var shape: Shape
    let color: CGColor
    let width: CGFloat

    var path: CGPath {
        switch shape {
        case .rectangle(let rect): CGPath(rect: rect, transform: nil)
        case .ellipse(let rect): CGPath(ellipseIn: rect, transform: nil)
        case .mosaic(let points): Self.polyline(points)
        }
    }

    var isMosaic: Bool {
        if case .mosaic = shape { return true }
        return false
    }

    static func polyline(_ points: [CGPoint]) -> CGPath {
        let path = CGMutablePath()
        guard let first = points.first else { return path }
        // 只点了一下也要留一个圆点
        path.addLines(between: points.count == 1 ? [first, CGPoint(x: first.x + 0.01, y: first.y)] : points)
        return path
    }
}

/// 把选区和标注合成最终的图片（原始像素，不经过屏幕显示）
enum CaptureRenderer {
    /// 马赛克色块的边长（点）
    static let mosaicBlock: CGFloat = 8

    private static let bitmapInfo = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue

    /// 整屏缩小成色块：每块是这一片像素的平均色
    static func pixelate(_ image: CGImage, blockPixels: Int) -> CGImage? {
        let block = max(blockPixels, 2)
        let width = max(1, image.width / block), height = max(1, image.height / block)
        guard let space = image.colorSpace ?? CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: bitmapInfo)
        else { return nil }
        context.interpolationQuality = .medium
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return context.makeImage()
    }

    static func render(_ image: CGImage, scale: CGFloat, selection: CGRect, marks: [CaptureMark], pixelated: CGImage?) -> CGImage? {
        let full = CGRect(x: 0, y: 0, width: image.width, height: image.height)
        let pixelRect = CGRect(x: (selection.minX * scale).rounded(), y: (selection.minY * scale).rounded(),
                               width: (selection.width * scale).rounded(), height: (selection.height * scale).rounded()).intersection(full)
        guard !pixelRect.isNull, pixelRect.width >= 1, pixelRect.height >= 1, let base = image.cropping(to: pixelRect),
              let space = image.colorSpace ?? CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: base.width, height: base.height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: bitmapInfo)
        else { return nil }
        let height = CGFloat(base.height)
        // 重新画一遍（而不是直接用裁出来的图），这样不会一直引用整块屏幕的截图
        context.draw(base, in: CGRect(x: 0, y: 0, width: base.width, height: base.height))

        // 截图层的点坐标（左上角原点）→ 这张图的像素坐标（左下角原点）
        var transform = CGAffineTransform(a: scale, b: 0, c: 0, d: -scale, tx: -pixelRect.minX, ty: height + pixelRect.minY)
        // 和屏幕上一样：马赛克在下面，矩形、椭圆在上面
        let ordered = marks.filter(\.isMosaic) + marks.filter { !$0.isMosaic }
        for mark in ordered {
            switch mark.shape {
            case .mosaic(let points):
                guard let pixelated else { continue }
                let stroke = CaptureMark.polyline(points).copy(strokingWithWidth: mark.width, lineCap: .round, lineJoin: .round, miterLimit: 10)
                guard let clip = stroke.copy(using: &transform) else { continue }
                context.saveGState()
                context.addPath(clip)
                context.clip()
                context.interpolationQuality = .none
                context.draw(pixelated, in: CGRect(x: -pixelRect.minX, y: height + pixelRect.minY - full.height, width: full.width, height: full.height))
                context.restoreGState()
            case .rectangle(let rect):
                stroke(CGPath(rect: rect, transform: &transform), mark: mark, scale: scale, in: context)
            case .ellipse(let rect):
                stroke(CGPath(ellipseIn: rect, transform: &transform), mark: mark, scale: scale, in: context)
            }
        }
        return context.makeImage()
    }

    private static func stroke(_ path: CGPath, mark: CaptureMark, scale: CGFloat, in context: CGContext) {
        context.addPath(path)
        context.setStrokeColor(mark.color)
        context.setLineWidth(mark.width * scale)
        context.setLineJoin(.miter)
        context.strokePath()
    }

    nonisolated static func pngData(_ image: CGImage) -> Data? {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, "public.png" as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination) ? data as Data : nil
    }

    static func writePNG(_ image: CGImage, to url: URL) {
        guard let data = pngData(image) else { return }
        try? data.write(to: url)
        Log.app.notice("capture snapshot \(url.lastPathComponent, privacy: .public) \(image.width)x\(image.height)")
    }
}
