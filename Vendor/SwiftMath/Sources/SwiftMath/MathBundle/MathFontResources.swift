// ClipMD 新增的文件（不是 SwiftMath 原有的）。
//
// 原版用 SwiftPM 生成的 `Bundle.module` 找字体，打包成 .app 后要求资源放在 .app 的根目录，
// 但 macOS 的代码签名不允许根目录有额外内容。另外 ClipMD 不随安装包带字体，而是用到时再下载。
// 所以这里改成：先找下载好的字体，再找 App 自己的 Contents/Resources。

import Foundation

public enum MathFontResources {
    /// 字体下载后放在这里：~/Library/Application Support/ClipMD/mathFonts.bundle
    public static var downloadedBundleURL: URL {
        URL.applicationSupportDirectory.appending(path: "ClipMD/mathFonts.bundle", directoryHint: .isDirectory)
    }

    public static let fontFileName = "latinmodern-math.otf"

    public static var bundleURL: URL? {
        let downloaded = downloadedBundleURL
        if FileManager.default.fileExists(atPath: downloaded.appending(path: fontFileName).path(percentEncoded: false)) {
            return downloaded
        }
        return Bundle.main.url(forResource: "mathFonts", withExtension: "bundle")
    }

    /// 字体在不在。不在时调用方应该直接显示公式原文，因为 SwiftMath 找不到字体会 fatalError。
    public static var isAvailable: Bool { bundleURL != nil }
}
