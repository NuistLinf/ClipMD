import AppKit
import ClipMDCore
import ImageIO
import SwiftUI

/// 剪贴板记录的预览：在悬浮窗旁边弹出一个小窗。文字可以看排版后的效果或原文，图片看大图。
/// 窗口只建一次，关掉时 orderOut；按 Esc、点别的应用、再点一次眼睛按钮都会关掉。
@MainActor
final class ClipPreviewController {
    struct Actions {
        var copy: @MainActor (ClipItem) -> Void
        var save: @MainActor (ClipItem) -> Void
        var recognize: @MainActor (ClipItem) -> Void
        var pin: @MainActor (ClipItem) -> Void
        var reveal: @MainActor (ClipItem) -> Void
    }

    private let history: ClipHistory
    private let actions: Actions
    let model = ClipPreviewModel()
    private lazy var panel = makePanel()
    private var outsideClickMonitor: Any?
    private(set) var isShown = false
    /// 测试时不显示、不挡鼠标、不抢焦点
    var hiddenForTesting = false

    init(history: ClipHistory, actions: Actions) {
        self.history = history
        self.actions = actions
    }

    var shownItemID: UUID? { isShown ? model.item?.id : nil }

    /// 测试：图片视图有没有真的拿到图片、大小对不对
    var debugImageState: String {
        guard let content = panel.contentView else { return "no content" }
        func descendants(_ view: NSView) -> [NSView] { view.subviews + view.subviews.flatMap(descendants) }
        return descendants(content).compactMap { $0 as? ImageLayerView }.map {
            "imageView \(Int($0.frame.width))x\(Int($0.frame.height)) image=\($0.image.map { "\($0.width)x\($0.height)" } ?? "nil") layerContents=\($0.layer?.contents != nil)"
        }.joined(separator: "; ")
    }
    var frame: NSRect? { isShown ? panel.frame : nil }

    /// 同一条再点一次就关掉
    func toggle(_ item: ClipItem, beside anchor: NSRect) {
        if shownItemID == item.id { close() } else { show(item, beside: anchor) }
    }

    func show(_ item: ClipItem, beside anchor: NSRect) {
        model.load(item, imageURL: history.imageURL(for: item))
        let size = Self.preferredSize(for: item, scale: NSScreen.main?.backingScaleFactor ?? 2)
        let target = Self.frame(size: size, beside: anchor)
        if isShown {
            panel.setFrame(target, display: true, animate: false)
            return
        }
        isShown = true
        panel.setFrame(target, display: false)
        if !(panel.contentView is NSHostingView<ClipPreviewView>) {
            panel.contentView = makeContent()
        }
        if hiddenForTesting {
            panel.alphaValue = 0
            panel.ignoresMouseEvents = true
            panel.orderFrontRegardless()
            return
        }
        panel.ignoresMouseEvents = false
        panel.alphaValue = 0
        panel.orderFrontRegardless()
        panel.makeKey()
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.16
            panel.animator().alphaValue = 1
        }
        // 点到别的应用就收起（只看别的应用里的点击；点悬浮窗里的另一条会直接换成那一条）
        outsideClickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
    }

    func close() {
        guard isShown else { return }
        isShown = false
        if let outsideClickMonitor { NSEvent.removeMonitor(outsideClickMonitor) }
        outsideClickMonitor = nil
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.12
            panel.animator().alphaValue = 0
        }
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(130))
            guard let self, !self.isShown else { return }
            // 先在（已经透明的）窗口里换成空内容，SwiftUI 才会放掉画过的大图；再收起窗口
            self.model.clear()
            try? await Task.sleep(for: .milliseconds(60))
            guard !self.isShown else { return }
            self.panel.orderOut(nil)
            self.panel.contentView = nil
        }
    }

    /// 记录被删掉或清空时跟着关
    private func itemsChanged() {
        guard let id = shownItemID, !history.items.contains(where: { $0.id == id }) else { return }
        close()
    }

    private func makePanel() -> ClipPreviewPanel {
        let panel = ClipPreviewPanel(contentRect: NSRect(x: 0, y: 0, width: 480, height: 540),
                                     styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.animationBehavior = .none
        panel.onEscape = { [weak self] in self?.close() }
        return panel
    }

    /// 内容视图每次打开时新建、关掉时拆掉：SwiftUI 的排版和玻璃效果不常驻内存
    private func makeContent() -> NSView {
        let view = ClipPreviewView(model: model, history: history, actions: actions,
                                   close: { [weak self] in self?.close() },
                                   itemsChanged: { [weak self] in self?.itemsChanged() })
        let hosting = NSHostingView(rootView: view)
        hosting.sizingOptions = []
        return hosting
    }

    /// 文字固定大小；图片按比例，长边不超过 640 × 520
    static func preferredSize(for item: ClipItem, scale: CGFloat) -> NSSize {
        guard item.isImage, let width = item.imageWidth, let height = item.imageHeight, width > 0, height > 0 else {
            return NSSize(width: 480, height: 540)
        }
        let maxContent = NSSize(width: 640, height: 520)
        let natural = NSSize(width: CGFloat(width) / scale, height: CGFloat(height) / scale)
        let fit = min(1, maxContent.width / natural.width, maxContent.height / natural.height)
        let content = NSSize(width: max(natural.width * fit, 200), height: max(natural.height * fit, 120))
        let chrome = NSSize(width: 28, height: ClipPreviewView.headerHeight + ClipPreviewView.footerHeight + 4)
        return NSSize(width: max(content.width + chrome.width, 380).rounded(), height: (content.height + chrome.height).rounded())
    }

    /// 放在悬浮窗旁边：悬浮窗在屏幕右半边就放左边，否则放右边；顶边和悬浮窗对齐
    static func frame(size: NSSize, beside anchor: NSRect) -> NSRect {
        let screen = NSScreen.screens.first { $0.frame.intersects(anchor) } ?? NSScreen.main ?? NSScreen.screens[0]
        let visible = screen.visibleFrame
        let gap: CGFloat = 10
        var x = anchor.midX > visible.midX ? anchor.minX - gap - size.width : anchor.maxX + gap
        if x < visible.minX + 8 { x = anchor.maxX + gap }
        x = min(max(x, visible.minX + 8), visible.maxX - size.width - 8)
        let y = min(max(anchor.maxY - size.height, visible.minY + 8), visible.maxY - size.height - 8)
        return NSRect(x: x, y: y, width: size.width, height: size.height)
    }

    /// 测试：把预览画成浅色、深色两张 PNG（不带滚动视图，内容全部展开）
    func snapshot(to folder: URL) {
        guard let item = model.item else { return }
        let size = Self.preferredSize(for: item, scale: 2)
        for dark in [false, true] {
            let view = ClipPreviewView(model: model, history: history, actions: actions, close: {}, itemsChanged: {}, scrollable: false)
                .frame(width: size.width, height: item.isImage ? size.height : nil)
                .background(dark ? Color(white: 0.2) : Color(white: 0.88))
                .environment(\.colorScheme, dark ? .dark : .light)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            guard let image = renderer.cgImage else { continue }
            CaptureRenderer.writePNG(image, to: folder.appending(path: dark ? "clip-preview-dark.png" : "clip-preview-light.png"))
        }
    }
}

/// 预览窗口：能接收按键（Esc 关闭、⌘C 复制选中的文字），但不会把 ClipMD 切到前台
final class ClipPreviewPanel: NSPanel {
    var onEscape: (() -> Void)?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { onEscape?() } else { super.keyDown(with: event) }
    }

    override func cancelOperation(_ sender: Any?) {
        onEscape?()
    }

    /// ClipMD 不在前台时主菜单不起作用，⌘C、⌘A 在这里转给选中的文字
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard flags == .command else { return super.performKeyEquivalent(with: event) }
        switch event.charactersIgnoringModifiers {
        case "c": return NSApp.sendAction(#selector(NSText.copy(_:)), to: nil, from: self)
        case "a": return NSApp.sendAction(#selector(NSText.selectAll(_:)), to: nil, from: self)
        case "w":
            onEscape?()
            return true
        default: return super.performKeyEquivalent(with: event)
        }
    }
}

@MainActor
@Observable
final class ClipPreviewModel {
    enum Mode: String, CaseIterable {
        case formatted = "排版"
        case plain = "原文"
    }

    private(set) var item: ClipItem?
    var mode: Mode = .plain
    /// 排版用的 Markdown（网页格式会先转成 Markdown，和「存成 md」得到的一样）
    private(set) var document: MarkdownDocument?
    private(set) var image: CGImage?
    @ObservationIgnored private var task: Task<Void, Never>?

    func load(_ item: ClipItem, imageURL: URL?) {
        task?.cancel()
        self.item = item
        document = nil
        image = nil
        if let imageURL {
            let display = NSScreen.main?.colorSpace?.cgColorSpace ?? CGColorSpace(name: CGColorSpace.sRGB)!
            task = Task {
                let decoded = await Task.detached(priority: .userInitiated) { Self.decode(imageURL, maxPixels: 1400, colorSpace: display) }.value
                guard !Task.isCancelled else { return }
                image = decoded
            }
        } else {
            mode = item.html != nil || ClipboardMarkdown.looksLikeMarkdown(item.text) ? .formatted : .plain
            let text = item.text, html = item.html
            task = Task {
                let parsed = await Task.detached(priority: .userInitiated) {
                    MarkdownParser.parse(ClipboardMarkdown.best(plain: text, html: html).text)
                }.value
                guard !Task.isCancelled else { return }
                document = parsed
            }
        }
    }

    func clear() {
        task?.cancel()
        item = nil
        document = nil
        image = nil
    }

    /// 只解码到预览里显示的大小（最大 640 × 520 点，Retina 屏约 1400 像素），不把 5K 截图整张读进内存
    /// 解码后先转成屏幕的色彩空间：显示时就不用再转换。显示时转换的话，
    /// CoreGraphics 会把转换结果留在它的色彩转换缓存里，实测每预览一次多占一份（6 MB 左右）
    private nonisolated static func decode(_ url: URL, maxPixels: Int, colorSpace: CGColorSpace) -> CGImage? {
        guard let thumbnail = thumbnail(url, maxPixels: maxPixels) else { return nil }
        if thumbnail.colorSpace?.name != nil, thumbnail.colorSpace?.name == colorSpace.name { return thumbnail }
        guard let context = CGContext(data: nil, width: thumbnail.width, height: thumbnail.height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: colorSpace,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return thumbnail }
        context.interpolationQuality = .none
        context.draw(thumbnail, in: CGRect(x: 0, y: 0, width: thumbnail.width, height: thumbnail.height))
        return context.makeImage()
    }

    private nonisolated static func thumbnail(_ url: URL, maxPixels: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixels,
            // 不让 CoreGraphics 把解码中间结果留在它自己的缓存里（实测每预览一次就多留一份）
            kCGImageSourceShouldCache: false,
            kCGImageSourceShouldCacheImmediately: false,
        ] as CFDictionary)
    }
}

struct ClipPreviewView: View {
    static let headerHeight: CGFloat = 58
    static let footerHeight: CGFloat = 52

    let model: ClipPreviewModel
    let history: ClipHistory
    let actions: ClipPreviewController.Actions
    let close: () -> Void
    let itemsChanged: () -> Void
    var scrollable = true

    @State private var copied = false

    /// 用记录里最新的那份（比如刚存成 md，要显示「已存」）
    private var item: ClipItem? {
        guard let item = model.item else { return nil }
        return history.items.first { $0.id == item.id } ?? item
    }

    var body: some View {
        Group {
            if let item {
                VStack(spacing: 0) {
                    header(item)
                    content(item)
                    footer(item)
                }
            } else {
                Color.clear
            }
        }
        .frame(maxWidth: .infinity, maxHeight: scrollable ? .infinity : nil)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .onChange(of: history.items.map(\.id)) { itemsChanged() }
        .onChange(of: model.item?.id) { copied = false }
    }

    // MARK: 顶部

    private func header(_ item: ClipItem) -> some View {
        HStack(spacing: 10) {
            Image(systemName: item.isImage ? "photo" : "text.alignleft")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 30, height: 30)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(.primary.opacity(0.07)))

            VStack(alignment: .leading, spacing: 2) {
                Text(item.title.isEmpty ? "（空白）" : item.title)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                Text(meta(item))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            if !item.isImage {
                Picker("显示方式", selection: Binding(get: { model.mode }, set: { model.mode = $0 })) {
                    ForEach(ClipPreviewModel.Mode.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .help("排版：按 Markdown 显示（网页格式会先转换，和存成 md 的结果一样）；原文：复制时的纯文本")
            }

            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .bold))
                    .frame(width: 24, height: 24)
                    .background(Circle().fill(.primary.opacity(0.08)))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("关闭（Esc）")
            .accessibilityLabel("关闭预览")
        }
        .padding(.horizontal, 14)
        .frame(height: Self.headerHeight)
    }

    private func meta(_ item: ClipItem) -> String {
        var parts = [item.date.formatted(date: .abbreviated, time: .shortened)]
        if item.isImage, let width = item.imageWidth, let height = item.imageHeight {
            parts.append("\(width) × \(height)")
        } else {
            parts.append("\(item.characterCount.formatted()) 字")
        }
        if let source = item.sourceApp { parts.append(source) }
        return parts.joined(separator: " · ")
    }

    // MARK: 内容

    @ViewBuilder
    private func content(_ item: ClipItem) -> some View {
        Group {
            if item.isImage {
                imageContent
            } else {
                textContent(item)
                    .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color(nsColor: .textBackgroundColor).opacity(0.92)))
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: scrollable ? .infinity : nil)
        .padding(.horizontal, 14)
    }

    @ViewBuilder
    private var imageContent: some View {
        if let image = model.image {
            Group {
                if scrollable {
                    LayerImage(image: image)
                } else {
                    // 测试快照画不出 AppKit 视图，用 SwiftUI 的 Image
                    Image(decorative: image, scale: 2).resizable()
                }
            }
            .aspectRatio(CGFloat(image.width) / CGFloat(max(image.height, 1)), contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.primary.opacity(0.1), lineWidth: 0.5))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onTapGesture(count: 2) { if let item { actions.pin(item) } }
                .help("双击钉在桌面")
        } else {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder
    private func textContent(_ item: ClipItem) -> some View {
        let body = Group {
            switch model.mode {
            case .formatted:
                if let document = model.document {
                    MarkdownContent(document: document, lazy: scrollable)
                } else {
                    ProgressView().controlSize(.small).frame(maxWidth: .infinity, minHeight: 80)
                }
            case .plain:
                Text(item.text)
                    .font(.system(size: 13))
                    .lineSpacing(3)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .textSelection(.enabled)

        if scrollable {
            ScrollView { body }
        } else {
            body
        }
    }

    // MARK: 底部操作

    private func footer(_ item: ClipItem) -> some View {
        HStack(spacing: 8) {
            Button {
                actions.copy(item)
                withAnimation(.snappy(duration: 0.25)) { copied = true }
                Task {
                    try? await Task.sleep(for: .seconds(1.4))
                    withAnimation(.smooth(duration: 0.3)) { copied = false }
                }
            } label: {
                Label(copied ? "已放回剪贴板" : "放回剪贴板", systemImage: copied ? "checkmark" : "doc.on.clipboard")
                    .contentTransition(.opacity)
            }
            .buttonStyle(.glassProminent)

            if item.isImage {
                Button("识别文字", systemImage: "text.viewfinder") { actions.recognize(item) }
                    .buttonStyle(.glass)
                Button("钉在桌面", systemImage: "pin") { actions.pin(item) }
                    .buttonStyle(.glass)
                Spacer(minLength: 0)
                Button { actions.reveal(item) } label: { Image(systemName: "folder") }
                    .buttonStyle(.glass)
                    .help("在 Finder 中显示")
                    .accessibilityLabel("在 Finder 中显示")
            } else {
                let saved = item.savedFileName != nil
                Button(saved ? "再存一份 md" : "存成 md", systemImage: saved ? "checkmark.circle" : "arrow.down.doc") { actions.save(item) }
                    .buttonStyle(.glass)
                    .help(item.savedFileName.map { "已存成 \($0)" } ?? "存成 Markdown 文件")
                Spacer(minLength: 0)
                Text("Esc 关闭")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
            }
        }
        .controlSize(.regular)
        .padding(.horizontal, 14)
        .frame(height: Self.footerHeight)
    }
}

/// 用 Core Animation 图层显示图片：换图或关掉预览时直接放掉，内存可控
/// （SwiftUI 的 Image 会把每次显示过的大图留在缓存里）
private struct LayerImage: NSViewRepresentable {
    let image: CGImage

    func makeNSView(context: Context) -> ImageLayerView { ImageLayerView() }

    func updateNSView(_ view: ImageLayerView, context: Context) {
        if view.image !== image { view.image = image }
    }

    static func dismantleNSView(_ view: ImageLayerView, coordinator: ()) {
        view.image = nil
    }
}

/// 图片放在图层的 contents 里。必须走 updateLayer：直接改 layer.contents 的话，
/// 视图一改大小 AppKit 就会重画图层，把图片清掉（之前预览图片是空白就是这个原因）
final class ImageLayerView: NSView {
    var image: CGImage? {
        didSet { needsDisplay = true }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        guard let layer else { return }
        layer.contentsGravity = .resizeAspect
        layer.minificationFilter = .trilinear
        layer.contents = image
    }
}
