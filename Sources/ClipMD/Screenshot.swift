import AppKit
import ScreenCaptureKit
import Vision

/// 截图开始时冻结下来的一块屏幕：之后的选区、放大镜、取色、标注都在这张图上做
struct FrozenScreen {
    let screen: NSScreen
    /// 屏幕在全局坐标里的位置（AppKit 坐标，原点在主屏左下角，单位是点）
    let frame: CGRect
    /// 原始像素，不带鼠标指针
    let image: CGImage
    /// 这块屏幕上的窗口，从前到后；坐标相对屏幕左上角，单位是点
    let windows: [CGRect]

    /// 每个点对应几个像素（Retina 屏是 2）
    var scale: CGFloat { CGFloat(image.width) / frame.width }
}

/// 截图要用「屏幕录制」权限；临时签名每次重新编译后，系统可能会再问一次。
enum Screenshot {
    static var hasPermission: Bool { CGPreflightScreenCaptureAccess() }

    /// 弹系统的授权询问（只会弹一次，之后要到系统设置里改）
    static func requestPermission() {
        CGRequestScreenCaptureAccess()
    }

    /// 把每块屏幕当前的样子截下来。窗口列表在后台同时取（要 0.1 秒左右）
    static func freezeScreens() async -> [FrozenScreen] {
        let screens = NSScreen.screens
        let frames = screens.map { globalRect(of: $0) }
        let windowsTask = Task.detached(priority: .userInitiated) { windowFrames(screens: frames) }
        var images: [CGImage?] = []
        for (screen, frame) in zip(screens, frames) {
            let config = SCScreenshotConfiguration()
            config.showsCursor = false
            config.dynamicRange = .sdr
            config.width = Int(screen.frame.width * screen.backingScaleFactor)
            config.height = Int(screen.frame.height * screen.backingScaleFactor)
            do {
                images.append(try await SCScreenshotManager.captureScreenshot(rect: frame, configuration: config).sdrImage)
            } catch {
                Log.app.error("capture failed: \(String(describing: error), privacy: .public)")
                images.append(nil)
            }
        }
        let windows = await windowsTask.value
        return zip(screens, zip(frames, images)).compactMap { screen, pair in
            guard let image = pair.1 else { return nil }
            return FrozenScreen(screen: screen, frame: screen.frame, image: image, windows: localWindows(windows, in: pair.0))
        }
    }

    /// 测试用：拿一张图片假装是主屏幕的画面
    static func freezeForTesting(_ url: URL) -> [FrozenScreen] {
        guard let screen = NSScreen.screens.first,
              let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let decoded = CGImageSourceCreateImageAtIndex(source, 0, nil),
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: decoded.width, height: decoded.height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: space, bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
        else { return [] }
        // 先解码成位图，和真实截图一样，取色、放大时不用反复解 PNG
        context.draw(decoded, in: CGRect(x: 0, y: 0, width: decoded.width, height: decoded.height))
        guard let image = context.makeImage() else { return [] }
        let frame = globalRect(of: screen)
        let windows = localWindows(windowFrames(screens: [frame]), in: frame)
        return [FrozenScreen(screen: screen, frame: screen.frame, image: image, windows: windows)]
    }

    /// 屏幕在「全局显示坐标」里的位置：原点在主屏左上角，y 向下（窗口列表和截屏接口用的都是这个）
    private static func globalRect(of screen: NSScreen) -> CGRect {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? screen.frame.height
        return CGRect(x: screen.frame.minX, y: primaryHeight - screen.frame.maxY, width: screen.frame.width, height: screen.frame.height)
    }

    /// 屏幕上的窗口，从前到后。程序坞这类盖满整块屏幕的系统透明层不算，不然鼠标在哪都会选中它
    nonisolated static func windowFrames(screens: [CGRect]) -> [CGRect] {
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { return [] }
        return list.compactMap { info in
            guard let bounds = info[kCGWindowBounds as String] as? NSDictionary,
                  let rect = CGRect(dictionaryRepresentation: bounds) else { return nil }
            let layer = info[kCGWindowLayer as String] as? Int ?? 0
            let alpha = info[kCGWindowAlpha as String] as? Double ?? 1
            guard alpha > 0.01, rect.width >= 8, rect.height >= 8 else { return nil }
            if layer != 0, screens.contains(where: { $0.insetBy(dx: -1, dy: -1).contains(rect) && rect.width >= $0.width - 1 && rect.height >= $0.height - 1 }) {
                return nil
            }
            return rect
        }
    }

    /// 换成相对这块屏幕左上角的坐标，只留和屏幕有交集的部分
    private static func localWindows(_ windows: [CGRect], in screen: CGRect) -> [CGRect] {
        let bounds = CGRect(origin: .zero, size: screen.size)
        return windows.compactMap { window in
            let local = window.offsetBy(dx: -screen.minX, dy: -screen.minY).intersection(bounds)
            return local.isNull || local.width < 4 || local.height < 4 ? nil : local
        }
    }
}

/// 识别图片里的文字：系统自带的 Vision，完全在本机，不联网。支持简体、繁体中文和英文。
///
/// 实测（同一批测试图）：
/// - 指定中文优先：中文、中英混排都准，纯英文偶尔认错单词（dog → qog）
/// - 自动识别语言：纯英文全对，但中英混排时可能把整行中文认成乱码
/// 所以先按中文优先认；结果里一个汉字都没有（纯英文图），再用自动识别认一遍。
enum TextRecognizer {
    nonisolated static func recognize(_ url: URL) throws -> String {
        let chinese = try run(url) { $0.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"] }
        guard !chinese.unicodeScalars.contains(where: { (0x4E00...0x9FFF).contains($0.value) }) else { return chinese }
        let automatic = try run(url) { $0.automaticallyDetectsLanguage = true }
        return automatic.isEmpty ? chinese : automatic
    }

    private nonisolated static func run(_ url: URL, configure: (VNRecognizeTextRequest) -> Void) throws -> String {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        configure(request)
        try VNImageRequestHandler(url: url).perform([request])
        return arrange(request.results ?? [])
    }

    /// 在一个用完就退出的子进程里识别：识别模型要占几十 MB，子进程退出后就还给系统，
    /// ClipMD 常驻时的内存不会因为用过一次识别就涨上去。子进程就是 ClipMD 自己，带上 `--ocr <图片路径>` 启动
    static func recognizeInHelper(_ url: URL) async throws -> String {
        guard let executable = Bundle.main.executableURL else { return try recognize(url) }
        return try await Task.detached(priority: .userInitiated) {
            let process = Process()
            process.executableURL = executable
            process.arguments = ["--ocr", url.path(percentEncoded: false)]
            let output = Pipe(), errors = Pipe()
            process.standardOutput = output
            process.standardError = errors
            try process.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            let errorData = errors.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            guard process.terminationStatus == 0 else {
                throw NSError(domain: "ClipMD.OCR", code: Int(process.terminationStatus),
                              userInfo: [NSLocalizedDescriptionKey: String(data: errorData, encoding: .utf8) ?? "识别进程出错"])
            }
            return String(data: data, encoding: .utf8) ?? ""
        }.value
    }

    /// 子进程的入口：识别完把文字写到标准输出就退出，不启动界面
    static func runAsHelper(path: String) -> Never {
        do {
            let text = try recognize(URL(filePath: path))
            FileHandle.standardOutput.write(Data(text.utf8))
            exit(0)
        } catch {
            FileHandle.standardError.write(Data(error.localizedDescription.utf8))
            exit(1)
        }
    }

    /// 按阅读顺序排：一行一行从上到下，同一行里从左到右（Vision 的坐标原点在左下角）
    nonisolated static func arrange(_ observations: [VNRecognizedTextObservation]) -> String {
        let boxes = observations.compactMap { observation -> (box: CGRect, text: String)? in
            guard let text = observation.topCandidates(1).first?.string else { return nil }
            return (observation.boundingBox, text)
        }.sorted { $0.box.midY > $1.box.midY }

        var lines: [[(box: CGRect, text: String)]] = []
        for item in boxes {
            // 和上一行的高度重叠一半以上，就算同一行
            if let last = lines.last?.last, abs(last.box.midY - item.box.midY) < min(last.box.height, item.box.height) * 0.5 {
                lines[lines.count - 1].append(item)
            } else {
                lines.append([item])
            }
        }
        return lines
            .map { $0.sorted { $0.box.minX < $1.box.minX }.map(\.text).joined(separator: " ") }
            .joined(separator: "\n")
    }
}
