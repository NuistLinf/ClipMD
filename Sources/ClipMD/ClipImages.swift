import AppKit
import CryptoKit
import ImageIO

/// 剪贴板记录里的图片：单独存成 PNG 文件，不塞进记录文件里。
/// 文件名用内容的指纹，同一张图只存一份。
enum ClipImages {
    /// 太大的不记
    static let maxBytes = 30_000_000

    struct Stored: Sendable {
        let file: String
        let width: Int
        let height: Int
        let hash: String
    }

    /// 存好图片（放到后台线程调用）。不是图片、太大时返回 nil
    nonisolated static func store(_ data: Data, in folder: URL) -> Stored? {
        guard data.count <= maxBytes, let rep = NSBitmapImageRep(data: data) else { return nil }
        let isPNG = data.starts(with: [0x89, 0x50, 0x4E, 0x47])
        guard let png = isPNG ? data : rep.representation(using: .png, properties: [:]) else { return nil }
        let hash = SHA256.hash(data: png).prefix(12).map { String(format: "%02x", $0) }.joined()
        let file = "\(hash).png"
        let url = folder.appending(path: file)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            if !FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) {
                try png.write(to: url, options: .atomic)
            }
        } catch {
            Log.app.error("image store failed: \(String(describing: error), privacy: .public)")
            return nil
        }
        return Stored(file: file, width: rep.pixelsWide, height: rep.pixelsHigh, hash: hash)
    }

    /// 删掉记录里已经不用的图片（启动时调用，放到后台）
    nonisolated static func cleanup(in folder: URL, keeping files: Set<String>) {
        guard let urls = try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil) else { return }
        for url in urls where url.pathExtension == "png" && !files.contains(url.lastPathComponent) {
            try? FileManager.default.removeItem(at: url)
        }
    }

    // MARK: 缩略图

    private final class Box { let image: NSImage; init(_ image: NSImage) { self.image = image } }
    @MainActor private static let thumbnails: NSCache<NSString, Box> = {
        let cache = NSCache<NSString, Box>()
        cache.countLimit = 100
        return cache
    }()

    /// 列表里用的小图：用 ImageIO 直接解出小尺寸，不把整张大图读进内存
    @MainActor static func thumbnail(for url: URL, maxPixels: Int = 96) -> NSImage? {
        let key = url.lastPathComponent as NSString
        if let box = thumbnails.object(forKey: key) { return box.image }
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                  kCGImageSourceCreateThumbnailFromImageAlways: true,
                  kCGImageSourceCreateThumbnailWithTransform: true,
                  kCGImageSourceThumbnailMaxPixelSize: maxPixels,
              ] as CFDictionary) else { return nil }
        let image = NSImage(cgImage: cgImage, size: NSSize(width: cgImage.width / 2, height: cgImage.height / 2))
        thumbnails.setObject(Box(image), forKey: key)
        return image
    }
}
