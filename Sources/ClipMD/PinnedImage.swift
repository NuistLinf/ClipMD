import AppKit

/// 钉在桌面的截图：浮在所有窗口上面，出现在截图时的位置。
/// 拖动移动，滚轮或双指捏合缩放；双击或 Esc 关掉，右键有复制、识别文字等操作。
@MainActor
final class PinBoard {
    private var windows: [PinnedImageWindow] = []
    var onCopy: ((CGImage) -> Void)?
    var onRecognize: ((CGImage) -> Void)?

    /// 测试时钉出来的图不显示、不挡鼠标
    var hiddenForTesting = false

    var count: Int { windows.count }
    var debugState: String {
        windows.map { "\(Int($0.frame.minX)),\(Int($0.frame.minY)) \(Int($0.frame.width))x\(Int($0.frame.height)) zoom=\($0.zoom)" }.joined(separator: " | ")
    }

    func zoomLatest(_ zoom: CGFloat) {
        windows.last?.setZoom(zoom)
    }

    func pin(_ image: CGImage, frame: NSRect) {
        let window = PinnedImageWindow(image: image, frame: frame)
        if hiddenForTesting {
            window.alphaValue = 0
            window.ignoresMouseEvents = true
            windows.append(window)
            window.orderFrontRegardless()
            window.onClose = { [weak self] closed in self?.windows.removeAll { $0 === closed } }
            return
        }
        window.onClose = { [weak self] closed in self?.windows.removeAll { $0 === closed } }
        window.onCopy = { [weak self] in self?.onCopy?($0) }
        window.onRecognize = { [weak self] in self?.onRecognize?($0) }
        windows.append(window)
        window.present()
    }

    func closeAll() {
        windows.forEach { $0.dismiss() }
    }
}

final class PinnedImageWindow: NSPanel {
    let image: CGImage
    private let baseSize: NSSize
    private(set) var zoom: CGFloat = 1
    private var closing = false
    var onClose: ((PinnedImageWindow) -> Void)?
    var onCopy: ((CGImage) -> Void)?
    var onRecognize: ((CGImage) -> Void)?

    init(image: CGImage, frame: NSRect) {
        self.image = image
        baseSize = frame.size
        super.init(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .floating
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        isReleasedWhenClosed = false
        hidesOnDeactivate = false
        animationBehavior = .none
        contentView = PinnedImageView(frame: NSRect(origin: .zero, size: frame.size), image: image)
        setFrame(frame, display: false)
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    func present() {
        alphaValue = 0
        orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.16
            animator().alphaValue = 1
        }
    }

    func dismiss() {
        guard !closing else { return }
        closing = true
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.12
            animator().alphaValue = 0
        }
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(130))
            guard let self else { return }
            self.orderOut(nil)
            self.onClose?(self)
        }
    }

    override func keyDown(with event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if event.keyCode == 53 {
            dismiss()
        } else if flags == .command, event.charactersIgnoringModifiers == "c" {
            onCopy?(image)
        } else {
            super.keyDown(with: event)
        }
    }

    /// 缩放时鼠标下的那一点保持不动
    func setZoom(_ newZoom: CGFloat, anchor: NSPoint? = nil) {
        let zoom = min(max(newZoom, 0.2), 5)
        guard zoom != self.zoom else { return }
        let old = frame
        let anchor = anchor ?? NSPoint(x: old.midX, y: old.midY)
        let size = NSSize(width: (baseSize.width * zoom).rounded(), height: (baseSize.height * zoom).rounded())
        let rx = (anchor.x - old.minX) / old.width, ry = (anchor.y - old.minY) / old.height
        self.zoom = zoom
        setFrame(NSRect(x: anchor.x - rx * size.width, y: anchor.y - ry * size.height, width: size.width, height: size.height), display: true)
        (contentView as? PinnedImageView)?.showZoom(zoom)
        invalidateShadow()
    }
}

private final class PinnedImageView: NSView {
    private let imageLayer = CALayer()
    private let zoomBadge = CATextLayer()
    private let closeButton = NSButton()
    private var badgeTask: Task<Void, Never>?

    init(frame: NSRect, image: CGImage) {
        super.init(frame: frame)
        wantsLayer = true
        guard let layer else { return }
        layer.cornerRadius = 6
        layer.masksToBounds = true
        layer.borderWidth = 0.5
        layer.borderColor = NSColor(white: 0.5, alpha: 0.35).cgColor

        imageLayer.contents = image
        imageLayer.contentsGravity = .resize
        imageLayer.minificationFilter = .trilinear
        imageLayer.frame = bounds
        imageLayer.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
        layer.addSublayer(imageLayer)

        zoomBadge.contentsScale = NSScreen.main?.backingScaleFactor ?? 2
        zoomBadge.backgroundColor = NSColor(white: 0, alpha: 0.6).cgColor
        zoomBadge.cornerRadius = 5
        zoomBadge.alignmentMode = .center
        zoomBadge.opacity = 0
        layer.addSublayer(zoomBadge)

        closeButton.image = NSImage(systemSymbolName: "xmark.circle.fill", accessibilityDescription: "关闭")?
            .withSymbolConfiguration(.init(pointSize: 17, weight: .medium).applying(.init(paletteColors: [.white, NSColor(white: 0, alpha: 0.55)])))
        closeButton.isBordered = false
        closeButton.imagePosition = .imageOnly
        closeButton.target = self
        closeButton.action = #selector(close)
        closeButton.toolTip = "关闭（双击图片或按 Esc 也可以）"
        closeButton.frame = NSRect(x: bounds.width - 26, y: bounds.height - 26, width: 22, height: 22)
        closeButton.autoresizingMask = [.minXMargin, .minYMargin]
        closeButton.alphaValue = 0
        addSubview(closeButton)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private var pinned: PinnedImageWindow? { window as? PinnedImageWindow }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) {
        closeButton.animator().alphaValue = 1
    }

    override func mouseExited(with event: NSEvent) {
        closeButton.animator().alphaValue = 0
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.makeKey()
        if event.clickCount == 2 {
            pinned?.dismiss()
        } else {
            window?.performDrag(with: event)
        }
    }

    override func scrollWheel(with event: NSEvent) {
        guard let pinned else { return }
        let delta = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY * 0.004 : event.scrollingDeltaY * 0.08
        pinned.setZoom(pinned.zoom * (1 + delta), anchor: NSEvent.mouseLocation)
    }

    override func magnify(with event: NSEvent) {
        guard let pinned else { return }
        pinned.setZoom(pinned.zoom * (1 + event.magnification), anchor: NSEvent.mouseLocation)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let pinned else { return nil }
        let menu = NSMenu()
        menu.addItem(ClosureMenuItem(title: "复制图片") { [weak pinned] in
            guard let pinned else { return }
            pinned.onCopy?(pinned.image)
        })
        menu.addItem(ClosureMenuItem(title: "识别文字") { [weak pinned] in
            guard let pinned else { return }
            pinned.onRecognize?(pinned.image)
        })
        let actual = ClosureMenuItem(title: "实际大小") { [weak pinned] in pinned?.setZoom(1) }
        actual.isEnabled = pinned.zoom != 1
        menu.addItem(actual)
        menu.addItem(.separator())
        menu.addItem(ClosureMenuItem(title: "关闭") { [weak pinned] in pinned?.dismiss() })
        return menu
    }

    @objc private func close() {
        pinned?.dismiss()
    }

    /// 缩放时在中间短暂显示比例
    func showZoom(_ zoom: CGFloat) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        zoomBadge.string = NSAttributedString(string: "\(Int((zoom * 100).rounded()))%", attributes: [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .semibold), .foregroundColor: NSColor.white,
        ])
        zoomBadge.frame = CGRect(x: (bounds.width - 54) / 2, y: (bounds.height - 22) / 2, width: 54, height: 22)
        zoomBadge.opacity = 1
        CATransaction.commit()
        badgeTask?.cancel()
        badgeTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(700))
            guard !Task.isCancelled else { return }
            self?.zoomBadge.opacity = 0
        }
    }
}
