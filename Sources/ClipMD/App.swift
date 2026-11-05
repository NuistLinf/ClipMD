import AppKit
import Carbon.HIToolbox
import ClipMDCore

@main
@MainActor
enum ClipMDMain {
    static func main() {
        // 识别文字的子进程（见 TextRecognizer.recognizeInHelper）：不启动界面
        let arguments = CommandLine.arguments
        if let index = arguments.firstIndex(of: "--ocr"), index + 1 < arguments.count {
            TextRecognizer.runAsHelper(path: arguments[index + 1])
        }
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        // 不出现在 Dock 和 ⌘Tab 里
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let store = NoteStore()
    private let hud = HUDController()
    private lazy var previews = PreviewWindows(store: store)
    private let doubleCopy = DoubleCopyDetector()
    private let history = ClipHistory()
    private let pins = PinBoard()
    private lazy var clipPreview = ClipPreviewController(history: history, actions: .init(
        copy: { [weak self] in self?.copyClip($0) },
        save: { [weak self] in self?.saveClip($0) },
        recognize: { [weak self] in self?.recognizeClip($0) },
        pin: { [weak self] in self?.pinClip($0) },
        reveal: { [weak self] in
            guard let url = self?.history.imageURL(for: $0) else { return }
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    ))
    private var capture: CaptureSession?
    private lazy var settings = SettingsWindow { [unowned self] in
        SettingsView(store: store, detector: doubleCopy, history: history, panel: floating, actions: SettingsActions(
            chooseFolder: { [weak self] in self?.chooseFolder() },
            openFolder: { [weak self] in self?.openFolder() },
            openPasteSettings: { [weak self] in self?.apply(.pasteSettings) },
            togglePin: { [weak self] in self?.floating.togglePin() },
            moveTo: { [weak self] in self?.floating.move(to: $0) }
        ))
    }
    private var floating: PanelController!
    private var statusItem: NSStatusItem!
    private var panelItem: NSMenuItem!
    private var tuckItem: NSMenuItem!
    private var pinItem: NSMenuItem!
    private var doubleCopyItem: NSMenuItem!
    private var saveItem: NSMenuItem!
    private var screenshotItem: NSMenuItem!
    private var ocrItem: NSMenuItem!

    func applicationDidFinishLaunching(_ notification: Notification) {
        Preferences.shared.applyAppearance()
        floating = PanelController(
            store: store,
            history: history,
            onSave: { [weak self] in self?.saveClipboard(source: "button") },
            onPreview: { [weak self] in self?.preview($0) },
            onTrashed: { [weak self] in self?.previews.discard($0) },
            onOpenFolder: { [weak self] in self?.openFolder() },
            onFix: { [weak self] in self?.apply($0) },
            onOpenSettings: { [weak self] in self?.settings.show() },
            onDrop: { [weak self] in self?.handleDrop($0, action: $1) },
            onSaveClip: { [weak self] in self?.saveClip($0) },
            onCopyClip: { [weak self] in self?.copyClip($0) },
            onPreviewClip: { [weak self] in self?.previewClip($0) },
            onRecognizeClip: { [weak self] in self?.recognizeClip($0) },
            onRevealClip: { [weak self] in
                guard let url = self?.history.imageURL(for: $0) else { return }
                NSWorkspace.shared.activateFileViewerSelecting([url])
            },
            onScreenshot: { [weak self] in self?.takeScreenshot(recognizeText: $0) }
        )
        setUpMainMenu()
        setUpStatusItem()

        let hotKeys = HotKeyCenter.shared
        hotKeys.onPress = { [weak self] action in
            switch action {
            case .save: self?.saveClipboard(source: "hotkey")
            case .screenshot: self?.takeScreenshot(recognizeText: false)
            case .screenshotOCR: self?.takeScreenshot(recognizeText: true)
            }
        }
        let failed = hotKeys.registerAll()
        if !failed.isEmpty {
            let names = failed.map { "\($0.title)（\(hotKeys.shortcuts[$0]?.display ?? "")）" }.joined(separator: "、")
            show(.failed(Failure(title: "有快捷键没能注册", detail: "\(names) 可能被别的应用占用了，可以在设置里换一个",
                                 fix: .openSettings)))
        }

        floating.onCollapse = { [weak self] in self?.clipPreview.close() }
        pins.onCopy = { [weak self] in self?.copyCapturedImage($0, record: false) }
        pins.onRecognize = { [weak self] in self?.recognizeCapturedImage($0) }

        doubleCopy.onDoubleCopy = { [weak self] in self?.saveClipboard(source: "double-copy") }
        doubleCopy.onTextCopied = { [weak self] in self?.floating.noteTextCopied() }
        doubleCopy.onTextRead = { [weak self] text, html, source in
            self?.history.record(text: text, html: html, sourceApp: source)
        }
        doubleCopy.onImageRead = { [weak self] data, source in
            Task { await self?.history.recordImage(data, sourceApp: source) }
        }
        doubleCopy.startIfEnabled()

        Log.app.notice("launched; folder=\(self.store.folder.path(percentEncoded: false), privacy: .public) pasteAccess=\(Pasteboard.accessBehavior.rawValue)")
        store.reload()
        floating.show()
    }

    func applicationWillTerminate(_ notification: Notification) {
        previews.saveAllSynchronously()
        history.saveNow()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        floating.show()
        return false
    }

    /// `clipmd://save`：可以从快捷指令、Raycast 等工具触发保存
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls where url.scheme == "clipmd" {
            switch url.host() {
            case "save": saveClipboard(source: "url")
            case "test":
                TestURLs.handle(url, store: store, panel: floating, previews: previews, history: history,
                                openSettings: { settings.show() }, eyeButton: { preview($0) }, saveClip: { saveClip($0) },
                                recognizeClip: { recognizeClip($0) }, capture: { handleCaptureTest($0, $1) })
            default: break
            }
        }
    }

    /// 测试截图层：`shot?<图片>` 用一张图片假装是屏幕画面开始截图，其余命令交给截图层
    private func handleCaptureTest(_ command: String, _ argument: String) {
        switch command {
        case "/shot":
            pins.hiddenForTesting = true
            beginCapture(Screenshot.freezeForTesting(URL(filePath: argument)), recognizeOnly: false, hiddenForTesting: true)
        case "/shot-ocr": beginCapture(Screenshot.freezeForTesting(URL(filePath: argument)), recognizeOnly: true, hiddenForTesting: true)
        case "/shot-visible": beginCapture(Screenshot.freezeForTesting(URL(filePath: argument)), recognizeOnly: false)
        case "/shot-real": takeScreenshot(recognizeText: false)
        case "/shot-pins": Log.app.notice("pins=\(self.pins.count) \(self.pins.debugState, privacy: .public)")
        case "/shot-pin-zoom": pins.zoomLatest(CGFloat(Double(argument) ?? 1))
        case "/shot-close-pins": pins.closeAll()
        case "/clip-preview":
            guard let index = Int(argument), history.items.indices.contains(index) else { return }
            clipPreview.hiddenForTesting = true
            pins.hiddenForTesting = true
            previewClip(history.items[index])
            Log.app.notice("clip preview shown=\(self.clipPreview.shownItemID != nil) frame=\(String(describing: self.clipPreview.frame), privacy: .public) mode=\(self.clipPreview.model.mode.rawValue, privacy: .public)")
        case "/clip-preview-debug": Log.app.notice("clip preview debug: \(self.clipPreview.debugImageState, privacy: .public)")
        case "/clip-preview-mode": clipPreview.model.mode = argument == "plain" ? .plain : .formatted
        case "/clip-preview-snapshot": clipPreview.snapshot(to: URL(filePath: argument, directoryHint: .isDirectory))
        case "/clip-preview-close": clipPreview.close()
        case "/clip-preview-pin": if let item = history.items.first(where: { $0.id == clipPreview.shownItemID }) { pinClip(item) }
        default:
            guard let canvas = capture?.activeCanvas else {
                Log.app.notice("capture test \(command, privacy: .public): no session")
                return
            }
            canvas.handleTest(command, argument)
        }
    }

    // MARK: 保存

    private func saveClipboard(source: String) {
        let byDoubleCopy = source == "double-copy"
        let content = Pasteboard.read()
        Log.app.notice("save from \(source, privacy: .public): \(content.logDescription, privacy: .public) pasteAccess=\(Pasteboard.accessBehavior.rawValue)")

        // 连点两次复制的不是文字（比如在 Finder 里复制文件），安静地忽略
        if byDoubleCopy, !content.isSavable, !content.isDenied { return }

        switch content {
        case .text(let text):
            let html = Preferences.shared.convertHTML ? Pasteboard.html() : nil
            Task {
                do {
                    let (note, source) = try await store.save(text, html: html)
                    Pasteboard.writeFile(note.url)
                    Log.app.notice("saved \(note.url.lastPathComponent, privacy: .public) source=\(source.rawValue, privacy: .public) html=\(html?.count ?? 0)")
                    show(.saved(note, fromHTML: source == .html, onClipboard: true))
                    if byDoubleCopy { offerUndoForDoubleCopy(note, text: text) }
                } catch {
                    Log.app.error("save failed: \(String(describing: error), privacy: .public)")
                    show(.failed(Self.failure(for: error)))
                }
            }
        case .image:
            show(.failed(Failure(title: "剪贴板里是图片", detail: "到悬浮窗的「剪贴板」里点「识别文字」，或者用「截图并识别文字」")))
        case .empty:
            show(.failed(Failure(title: "剪贴板里没有文字", detail: "先复制一段文字，再保存")))
        case .file:
            show(.failed(Failure(title: "剪贴板里是文件，不是文字", detail: "先复制一段文字，再保存")))
        case .concealed:
            show(.failed(Failure(title: "已跳过密码类内容", detail: "密码管理器复制的内容不会被保存")))
        case .denied:
            show(.failed(Failure(title: "macOS 没有允许读取剪贴板",
                                 detail: "在「隐私与安全性 → 粘贴」里把 ClipMD 设成「允许」",
                                 fix: .pasteSettings)))
        }
    }

    // MARK: 截图和识别文字

    /// 截图：先冻结整块屏幕，在 ClipMD 自己的截图层上选区、标注，再放进剪贴板（也留在剪贴板记录里）。
    /// `recognizeText` 时选好区域就直接识别里面的文字，把文字放进剪贴板
    private func takeScreenshot(recognizeText: Bool) {
        guard capture == nil else { return }
        guard Screenshot.hasPermission else {
            Screenshot.requestPermission()
            show(.failed(Failure(title: "需要允许 ClipMD 录制屏幕",
                                 detail: "截图要用到「屏幕录制」权限。打开后需要重新启动 ClipMD 才生效",
                                 fix: .screenSettings)))
            return
        }
        Task {
            let started = Date.now
            let screens = await Screenshot.freezeScreens()
            Log.app.notice("froze \(screens.count) screens in \(Int(Date.now.timeIntervalSince(started) * 1000))ms")
            guard !screens.isEmpty else {
                show(.failed(Failure(title: "截图失败", detail: "没能读取屏幕画面，可以检查「录制屏幕」权限", fix: .screenSettings)))
                return
            }
            beginCapture(screens, recognizeOnly: recognizeText)
        }
    }

    private func beginCapture(_ screens: [FrozenScreen], recognizeOnly: Bool, hiddenForTesting: Bool = false) {
        guard capture == nil, !screens.isEmpty else { return }
        let session = CaptureSession(screens: screens, recognizeOnly: recognizeOnly, hiddenForTesting: hiddenForTesting) { [weak self] in
            self?.finishCapture($0)
        }
        capture = session
        session.begin()
    }

    private func finishCapture(_ result: CaptureResult) {
        capture = nil
        switch result {
        case .cancelled:
            Log.app.notice("capture cancelled")
        case .color(let text):
            Pasteboard.writeString(text)
            Log.app.notice("capture color \(text, privacy: .public)")
            show(.copied(title: "已复制色号 \(text)", detail: "可以直接粘贴"))
        case .copy(let image):
            copyCapturedImage(image, record: true)
        case .pin(let image, let frame):
            pins.pin(image, frame: frame)
            Log.app.notice("pinned \(image.width)x\(image.height) pins=\(self.pins.count)")
        case .recognize(let image):
            recognizeCapturedImage(image)
        }
    }

    private func copyCapturedImage(_ image: CGImage, record: Bool) {
        let size = "\(image.width) × \(image.height)"
        Task {
            guard let png = await Self.encodePNG(image) else { return }
            Pasteboard.writeImage(png)
            let recorded = record ? await history.recordImage(png, sourceApp: "截图") : nil
            Log.app.notice("capture copied \(size, privacy: .public) png=\(png.count)")
            show(.copied(title: "截图已放到剪贴板", detail: recorded == nil ? "\(size)，可以直接粘贴" : "\(size)，也留在了剪贴板记录里"))
        }
    }

    private func recognizeCapturedImage(_ image: CGImage) {
        Task {
            guard let png = await Self.encodePNG(image) else { return }
            let url = FileManager.default.temporaryDirectory.appending(path: "ClipMD-ocr-\(UUID().uuidString).png")
            defer { try? FileManager.default.removeItem(at: url) }
            guard (try? png.write(to: url)) != nil else { return }
            await history.recordImage(png, sourceApp: "截图")
            await recognizeText(in: url, source: "截图识别")
        }
    }

    /// 大图编码要几百毫秒，放到后台
    private static func encodePNG(_ image: CGImage) async -> Data? {
        await Task.detached(priority: .userInitiated) { CaptureRenderer.pngData(image) }.value
    }

    /// 识别图片里的文字，放进剪贴板，并作为一条文字记录留下来
    private func recognizeText(in url: URL, source: String) async {
        let started = Date.now
        // 一般 0.3 秒就认完，提示一闪而过反而晃眼；超过 0.4 秒（比如第一次用，系统要准备识别模型）才提示在识别
        let progress = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            self?.show(.working(title: "正在识别文字…", detail: "在本机识别，不联网"))
        }
        defer { progress.cancel() }
        do {
            let text = try await TextRecognizer.recognizeInHelper(url)
            Log.app.notice("ocr \(text.count) chars in \(Int(Date.now.timeIntervalSince(started) * 1000))ms")
            guard text.contains(where: { !$0.isWhitespace }) else {
                show(.failed(Failure(title: "没识别出文字", detail: "图片里可能没有文字，或者字太小、太模糊")))
                return
            }
            Pasteboard.writeText(text, html: nil)
            history.record(text: text, html: nil, sourceApp: source)
            show(.copied(title: "已识别文字，放到剪贴板了", detail: "\(text.count.formatted()) 字，也留在了剪贴板记录里"))
        } catch {
            Log.app.error("ocr failed: \(String(describing: error), privacy: .public)")
            show(.failed(Failure(title: "识别文字失败", detail: error.localizedDescription)))
        }
    }

    /// 剪贴板记录里的一张图片：识别文字
    private func recognizeClip(_ item: ClipItem) {
        guard let url = history.imageURL(for: item) else { return }
        Task { await recognizeText(in: url, source: "识别自图片") }
    }

    /// 预览剪贴板记录里的一条：在悬浮窗旁边弹出，同一条再点一次就关掉
    private func previewClip(_ item: ClipItem) {
        clipPreview.toggle(item, beside: floating.cardFrame)
    }

    /// 剪贴板记录里的图片钉在桌面：按实际大小（太大就缩到屏幕的七成），放在预览窗口的位置
    private func pinClip(_ item: ClipItem) {
        guard let url = history.imageURL(for: item),
              let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return }
        let center = clipPreview.frame.map { NSPoint(x: $0.midX, y: $0.midY) } ?? NSPoint(x: floating.cardFrame.midX, y: floating.cardFrame.midY)
        let screen = NSScreen.screens.first { NSMouseInRect(center, $0.frame, false) } ?? NSScreen.main ?? NSScreen.screens[0]
        let visible = screen.visibleFrame
        var size = NSSize(width: CGFloat(image.width) / screen.backingScaleFactor, height: CGFloat(image.height) / screen.backingScaleFactor)
        let fit = min(1, visible.width * 0.7 / size.width, visible.height * 0.7 / size.height)
        size = NSSize(width: (size.width * fit).rounded(), height: (size.height * fit).rounded())
        var frame = NSRect(x: center.x - size.width / 2, y: center.y - size.height / 2, width: size.width, height: size.height)
        frame.origin.x = min(max(frame.minX, visible.minX), visible.maxX - frame.width)
        frame.origin.y = min(max(frame.minY, visible.minY), visible.maxY - frame.height)
        clipPreview.close()
        pins.pin(image, frame: frame)
    }

    /// 剪贴板记录里的一条放回剪贴板：文字带上网页格式，图片放 PNG
    private func copyClip(_ item: ClipItem) {
        if let url = history.imageURL(for: item), let data = try? Data(contentsOf: url) {
            Pasteboard.writeImage(data)
        } else {
            Pasteboard.writeText(item.text, html: item.html)
        }
    }

    /// 剪贴板记录里的一条存成 md：不动系统剪贴板，存好后单击"已保存"里的那条就能复制文件
    private func saveClip(_ item: ClipItem) {
        Task {
            do {
                let (note, source) = try await store.save(item.text, html: Preferences.shared.convertHTML ? item.html : nil)
                history.markSaved(item, as: note.url.lastPathComponent)
                Log.app.notice("saved \(note.url.lastPathComponent, privacy: .public) source=\(source.rawValue, privacy: .public) via=history")
                show(.saved(note, fromHTML: source == .html, onClipboard: false))
            } catch {
                Log.app.error("save failed: \(String(describing: error), privacy: .public)")
                show(.failed(Self.failure(for: error)))
            }
        }
    }

    /// 拖进悬浮窗：上半块存成 Markdown（不动系统剪贴板），下半块只放到剪贴板（不存文件）
    private func handleDrop(_ content: DroppedContent, action: DropAction) {
        switch action {
        case .save: saveDropped(content)
        case .clipboard: copyDropped(content)
        }
    }

    private func copyDropped(_ content: DroppedContent) {
        Task {
            let items: [(text: String, html: String?)]
            switch content {
            case .text(let text, let html): items = [(text, html)]
            case .files(let urls):
                items = await Task.detached(priority: .userInitiated) { urls.prefix(20).compactMap(DroppedText.load) }.value
            }
            // 网页文件只有 html：纯文本那份用转出来的 Markdown
            let texts = items.map { $0.text.isEmpty ? HTMLToMarkdown.convert($0.html ?? "") ?? "" : $0.text }
                .filter { $0.contains(where: { !$0.isWhitespace }) }
            guard !texts.isEmpty else {
                show(.failed(Failure(title: "没读出文字", detail: TrackingContainerView.rejectMessage)))
                return
            }
            let text = texts.joined(separator: "\n\n")
            let html = items.count == 1 ? items[0].html : nil
            Pasteboard.writeText(text, html: html)
            Log.app.notice("drop copied to clipboard: \(text.count) chars html=\(html?.count ?? 0)")
            show(.copied(title: "已放到剪贴板", detail: "\(text.count.formatted()) 字" + (html != nil ? " · 带网页格式" : "") + "，没有存文件"))
        }
    }

    private func saveDropped(_ content: DroppedContent) {
        Task {
            let items: [(text: String, html: String?)]
            switch content {
            case .text(let text, let html):
                Log.app.notice("save from drop: text(\(text.count) chars) html=\(html?.count ?? 0)")
                items = [(text, html)]
            case .files(let urls):
                Log.app.notice("save from drop: \(urls.count) file(s) \(urls.map(\.pathExtension).joined(separator: ","), privacy: .public)")
                // 读文件放到后台，最多一次 20 个
                items = await Task.detached(priority: .userInitiated) {
                    urls.prefix(20).compactMap(DroppedText.load)
                }.value
            }
            guard !items.isEmpty else {
                show(.failed(Failure(title: "没读出文字", detail: TrackingContainerView.rejectMessage)))
                return
            }
            do {
                var last: (SavedNote, ClipboardMarkdown.Source)?
                for item in items {
                    let html = Preferences.shared.convertHTML ? item.html : nil
                    // 网页文件只有 html：关掉了转换时，退回去掉标签的文字
                    let text = item.text.isEmpty && html == nil ? HTMLToMarkdown.convert(item.html ?? "") ?? "" : item.text
                    guard text.contains(where: { !$0.isWhitespace }) || html != nil else { continue }
                    last = try await store.save(text, html: html)
                }
                guard let (note, source) = last else { return }
                Log.app.notice("saved \(note.url.lastPathComponent, privacy: .public) source=\(source.rawValue, privacy: .public) via=drop count=\(items.count)")
                show(.saved(note, fromHTML: source == .html, onClipboard: false))
            } catch {
                Log.app.error("save failed: \(String(describing: error), privacy: .public)")
                show(.failed(Self.failure(for: error)))
            }
        }
    }

    private func show(_ feedback: Feedback) {
        hud.show(feedback)
        floating.show(feedback)
    }

    /// 连点两次是自动触发的，万一误触：撤销会把文件移到废纸篓，并把原来的文字放回剪贴板
    private func offerUndoForDoubleCopy(_ note: SavedNote, text: String) {
        floating.offerUndo("连点两次复制，已存成 Markdown") { [weak self] in
            guard let self else { return }
            self.previews.discard(note.url)
            _ = try? self.store.trash(note)
            Pasteboard.writeString(text)
            Log.app.notice("undid double-copy save \(note.url.lastPathComponent, privacy: .public)")
        }
    }

    private static func failure(for error: Error) -> Failure {
        let error = error as NSError
        let noPermission = [NSFileWriteNoPermissionError, NSFileReadNoPermissionError]
        if error.domain == NSCocoaErrorDomain, noPermission.contains(error.code) {
            return Failure(title: "没有权限写入保存文件夹",
                           detail: "在「隐私与安全性 → 文件与文件夹」里允许 ClipMD 访问「文稿」，或者换个保存位置",
                           fix: .filesSettings)
        }
        return Failure(title: "保存失败", detail: error.localizedDescription, fix: .chooseFolder)
    }

    private func apply(_ fix: FailureFix) {
        if let url = fix.settingsURL {
            NSWorkspace.shared.open(url)
        } else if fix == .openSettings {
            settings.show()
        } else {
            chooseFolder()
        }
    }

    // MARK: 主菜单

    /// 菜单栏应用不显示主菜单，但 ⌘C/⌘V/⌘Z/⌘W 这些快捷键要靠它才能在预览窗口里生效
    private func setUpMainMenu() {
        let main = NSMenu()
        func addMenu(_ title: String, _ items: [NSMenuItem]) {
            let menu = NSMenu(title: title)
            items.forEach(menu.addItem)
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            item.submenu = menu
            main.addItem(item)
        }
        func item(_ title: String, _ action: Selector, _ key: String,
                  _ modifiers: NSEvent.ModifierFlags = .command, tag: Int = 0) -> NSMenuItem {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
            item.keyEquivalentModifierMask = modifiers
            item.tag = tag
            return item
        }

        let settingsItem = item("设置…", #selector(showSettings), ",")
        settingsItem.target = self
        addMenu("ClipMD", [settingsItem, .separator(), item("退出 ClipMD", #selector(NSApplication.terminate(_:)), "q")])
        addMenu("文件", [
            item("保存", #selector(PreviewWindow.saveDocument(_:)), "s"),
            item("关闭", #selector(NSWindow.performClose(_:)), "w"),
        ])
        addMenu("编辑", [
            item("撤销", Selector(("undo:")), "z"),
            item("重做", Selector(("redo:")), "z", [.command, .shift]),
            .separator(),
            item("剪切", #selector(NSText.cut(_:)), "x"),
            item("拷贝", #selector(NSText.copy(_:)), "c"),
            item("粘贴", #selector(NSText.paste(_:)), "v"),
            item("全选", #selector(NSText.selectAll(_:)), "a"),
        ])
        addMenu("显示", NoteDocument.Mode.allCases.enumerated().map { index, mode in
            item(mode.title, #selector(PreviewWindow.showMode(_:)), "\(index + 1)", tag: index)
        })
        NSApp.mainMenu = main
    }

    // MARK: 菜单栏

    private func setUpStatusItem() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = MenuBarGlyph.templateImage() ?? NSImage(systemSymbolName: "doc.on.clipboard", accessibilityDescription: "ClipMD")

        let menu = NSMenu()
        menu.delegate = self
        menu.autoenablesItems = false

        saveItem = NSMenuItem(title: "保存剪贴板为 Markdown", action: #selector(saveFromMenu), keyEquivalent: "")
        menu.addItem(saveItem)
        screenshotItem = NSMenuItem(title: "截图", action: #selector(screenshotFromMenu), keyEquivalent: "")
        menu.addItem(screenshotItem)
        ocrItem = NSMenuItem(title: "截图并识别文字", action: #selector(ocrFromMenu), keyEquivalent: "")
        menu.addItem(ocrItem)
        doubleCopyItem = NSMenuItem(title: "监听复制（连点两次自动保存）", action: #selector(toggleDoubleCopy), keyEquivalent: "")
        menu.addItem(doubleCopyItem)
        menu.addItem(.separator())

        panelItem = NSMenuItem(title: "隐藏悬浮窗", action: #selector(togglePanel), keyEquivalent: "")
        menu.addItem(panelItem)
        pinItem = NSMenuItem(title: "固定位置", action: #selector(togglePin), keyEquivalent: "")
        menu.addItem(pinItem)
        tuckItem = NSMenuItem(title: "贴边收起", action: #selector(toggleTuck), keyEquivalent: "")
        menu.addItem(tuckItem)

        let moveMenu = NSMenu()
        for (index, corner) in PanelController.Corner.allCases.enumerated() {
            let item = NSMenuItem(title: corner.title, action: #selector(moveToCorner(_:)), keyEquivalent: "")
            item.tag = index
            item.target = self
            moveMenu.addItem(item)
        }
        let moveItem = NSMenuItem(title: "把悬浮窗移到", action: nil, keyEquivalent: "")
        moveItem.submenu = moveMenu
        menu.addItem(moveItem)
        menu.addItem(.separator())

        menu.addItem(NSMenuItem(title: "打开保存文件夹", action: #selector(openFolderFromMenu), keyEquivalent: ""))
        menu.addItem(NSMenuItem(title: "更改保存位置…", action: #selector(chooseFolder), keyEquivalent: ""))
        menu.addItem(NSMenuItem(title: "设置…", action: #selector(showSettings), keyEquivalent: ","))
        for item in menu.items where item.action != nil { item.target = self }
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "退出 ClipMD", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))

        statusItem.menu = menu
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        guard menu === statusItem.menu else { return }
        doubleCopyItem.state = doubleCopy.isEnabled ? .on : .off
        let shortcuts = HotKeyCenter.shared.shortcuts
        for (item, action) in [(saveItem, HotKeyAction.save), (screenshotItem, .screenshot), (ocrItem, .screenshotOCR)] {
            item?.title = action.title + (shortcuts[action].map { "  \($0.display)" } ?? "")
        }
        panelItem.title = floating.isVisible ? "隐藏悬浮窗" : "显示悬浮窗"
        pinItem.state = floating.isPinned ? .on : .off
        pinItem.isEnabled = floating.isVisible
        tuckItem.title = floating.isTucked ? "展开悬浮窗" : "贴边收起"
        tuckItem.isEnabled = floating.isVisible
    }

    @objc private func saveFromMenu() { saveClipboard(source: "menu") }

    @objc private func screenshotFromMenu() { takeScreenshot(recognizeText: false) }

    @objc private func ocrFromMenu() { takeScreenshot(recognizeText: true) }

    @objc private func toggleDoubleCopy() { doubleCopy.setEnabled(!doubleCopy.isEnabled) }

    @objc private func togglePanel() {
        floating.isVisible ? floating.hide() : floating.show()
    }

    @objc private func togglePin() { floating.togglePin() }

    @objc private func toggleTuck() { floating.toggleTuck() }

    @objc private func moveToCorner(_ sender: NSMenuItem) {
        floating.move(to: PanelController.Corner.allCases[sender.tag])
    }

    @objc private func openFolderFromMenu() { openFolder() }

    @objc private func showSettings() { settings.show() }

    /// 眼睛按钮：按设置用内置预览，或者用选好的应用（Typora、VS Code……）打开
    private func preview(_ note: SavedNote) {
        guard let app = Preferences.shared.previewApp else {
            previews.show(note)
            return
        }
        NSWorkspace.shared.open([note.url], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration()) { _, error in
            guard let error else { return }
            Log.app.error("open with \(app.lastPathComponent, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    private func openFolder() {
        try? FileManager.default.createDirectory(at: store.folder, withIntermediateDirectories: true)
        NSWorkspace.shared.open(store.folder)
    }

    @objc private func chooseFolder() {
        let picker = NSOpenPanel()
        picker.canChooseDirectories = true
        picker.canChooseFiles = false
        picker.canCreateDirectories = true
        picker.allowsMultipleSelection = false
        picker.directoryURL = store.folder
        picker.prompt = "存到这里"
        picker.message = "选择 md 文件的保存位置，比如你的 Obsidian 库"
        NSApp.activate()
        if picker.runModal() == .OK, let url = picker.url {
            store.setFolder(url)
        }
    }
}
