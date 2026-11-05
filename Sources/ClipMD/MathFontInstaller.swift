import CryptoKit
import Foundation
import Observation
import SwiftMath

/// 公式字体不随安装包带，预览里遇到公式时由用户点了再下载（约 0.8 MB）。
///
/// 地址固定指向 SwiftMath 1.7.3 那次提交里的文件，下载后校验 SHA-256，内容对不上就不用。
@MainActor
@Observable
final class MathFontInstaller {
    enum State: Equatable {
        case missing, downloading, installed
        case failed(String)
    }

    static let shared = MathFontInstaller()

    private nonisolated static let baseURL = "https://raw.githubusercontent.com/mgriebling/SwiftMath/fa8244ed032f4a1ade4cb0571bf87d2f1a9fd2d7/Sources/SwiftMath/mathFonts.bundle/"
    private nonisolated static let files: [(name: String, sha256: String)] = [
        ("latinmodern-math.otf", "6075562b771f8b82f0c179e363389684f2dd09de30038269e2628e504bd7be0f"),
        ("latinmodern-math.plist", "201e6a483783415f335328f2d02b356fe55cd478eb0cd052898236589c8cb946"),
        // 字体的许可（GUST Font License），跟字体放在一起
        ("GUST-FONT-LICENSE.txt", "2bd69affc3da00715116f713f57eab9707e96daf3562ad0215987b15b9c16f73"),
    ]
    static let downloadSize = "约 0.8 MB"

    private(set) var state: State

    private init() {
        state = MathFontResources.isAvailable ? .installed : .missing
    }

    var isReady: Bool { state == .installed }

    func install() async {
        guard state != .downloading, state != .installed else { return }
        state = .downloading
        let destination = MathFontResources.downloadedBundleURL
        do {
            try await Task.detached(priority: .userInitiated) {
                try await Self.download(to: destination)
            }.value
            Log.app.notice("math fonts installed")
            state = .installed
        } catch {
            Log.app.error("math font download failed: \(String(describing: error), privacy: .public)")
            state = .failed(error.localizedDescription)
        }
    }

    /// 删除下载的字体。已经加载进内存的字体要等下次启动才会释放，但预览会立刻改回显示原文。
    func remove() {
        try? FileManager.default.removeItem(at: MathFontResources.downloadedBundleURL)
        state = MathFontResources.isAvailable ? .installed : .missing
        Log.app.notice("math fonts removed")
    }

    private enum DownloadError: LocalizedError {
        case badResponse(String)
        case checksumMismatch(String)

        var errorDescription: String? {
            switch self {
            case .badResponse(let name): "\(name) 下载失败"
            case .checksumMismatch(let name): "\(name) 校验没通过，已丢弃"
            }
        }
    }

    /// 先下载到临时文件夹，全部校验通过才挪到正式位置
    private nonisolated static func download(to destination: URL) async throws {
        let fm = FileManager.default
        let staging = fm.temporaryDirectory.appending(path: "ClipMD-fonts-\(UUID().uuidString)", directoryHint: .isDirectory)
        try fm.createDirectory(at: staging, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: staging) }

        for file in files {
            guard let url = URL(string: baseURL + file.name) else { throw DownloadError.badResponse(file.name) }
            let (data, response) = try await URLSession.shared.data(from: url)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw DownloadError.badResponse(file.name) }
            let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            guard digest == file.sha256 else { throw DownloadError.checksumMismatch(file.name) }
            try data.write(to: staging.appending(path: file.name))
        }

        try fm.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
        if fm.fileExists(atPath: destination.path(percentEncoded: false)) {
            try fm.removeItem(at: destination)
        }
        try fm.moveItem(at: staging, to: destination)
    }
}
