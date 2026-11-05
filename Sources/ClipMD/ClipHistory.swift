import ClipMDCore
import Foundation
import Observation
import SwiftUI

/// 剪贴板记录：复制过的文字和图片先记在这里，不存成文件，需要时再一键存成 md（或识别图片里的文字）。
///
/// 密码管理器复制的内容、ClipMD 自己写进剪贴板的内容不记。
/// 存在本机 ~/Library/Application Support/ClipMD/history.json，图片在旁边的 images 文件夹；
/// 写盘放到后台，1 秒内没有新复制才写。
@MainActor
@Observable
final class ClipHistory {
    private static let enabledKey = "clipHistoryEnabled"

    private(set) var items: [ClipItem] = []
    private(set) var isEnabled: Bool

    @ObservationIgnored private var saveTask: Task<Void, Never>?

    /// 测试时可以用启动参数 `-historyFile <路径>` 换一个文件，免得混进真正的记录
    private static var fileURL: URL {
        if let path = UserDefaults.standard.string(forKey: "historyFile") { return URL(filePath: path) }
        return URL.applicationSupportDirectory.appending(path: "ClipMD/history.json")
    }

    /// 图片放在记录文件旁边的 images 文件夹
    static var imagesFolder: URL {
        fileURL.deletingLastPathComponent().appending(path: "images", directoryHint: .isDirectory)
    }

    init() {
        isEnabled = UserDefaults.standard.object(forKey: Self.enabledKey) as? Bool ?? true
        // 文件很小（最多 50 条），启动时直接读
        if let data = try? Data(contentsOf: Self.fileURL) { items = ClipHistoryList.decode(data) }
        // 记录里已经删掉的图片，启动时顺手清掉（删除和清空都能撤销，所以不当场删文件）
        let keep = ClipHistoryList.referencedImages(in: items)
        let folder = Self.imagesFolder
        Task.detached(priority: .utility) { ClipImages.cleanup(in: folder, keeping: keep) }
    }

    func imageURL(for item: ClipItem) -> URL? {
        item.imageFile.map { Self.imagesFolder.appending(path: $0) }
    }

    /// 记一张图片（存文件放在后台）。记录关着、或不是图片时返回 nil
    @discardableResult
    func recordImage(_ data: Data, sourceApp: String?) async -> ClipItem? {
        guard isEnabled else { return nil }
        let folder = Self.imagesFolder
        guard let stored = await Task.detached(priority: .userInitiated, operation: { ClipImages.store(data, in: folder) }).value else {
            return nil
        }
        let item = ClipItem(imageFile: stored.file, width: stored.width, height: stored.height, hash: stored.hash,
                            date: .now, sourceApp: sourceApp)
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            items = ClipHistoryList.adding(item, to: items)
        }
        scheduleSave()
        return item
    }

    func setEnabled(_ enabled: Bool) {
        isEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: Self.enabledKey)
    }

    func record(text: String, html: String?, sourceApp: String?) {
        guard isEnabled, let item = ClipHistoryList.makeItem(text: text, html: html, sourceApp: sourceApp) else { return }
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            items = ClipHistoryList.adding(item, to: items)
        }
        scheduleSave()
    }

    func remove(_ item: ClipItem) {
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            items.removeAll { $0.id == item.id }
        }
        scheduleSave()
    }

    func restore(_ item: ClipItem, at index: Int) {
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            items.insert(item, at: min(index, items.count))
        }
        scheduleSave()
    }

    func clear() {
        withAnimation(.smooth(duration: 0.3)) { items.removeAll() }
        scheduleSave()
    }

    func markSaved(_ item: ClipItem, as fileName: String) {
        guard let index = items.firstIndex(where: { $0.id == item.id }) else { return }
        items[index].savedFileName = fileName
        scheduleSave()
    }

    private func scheduleSave() {
        saveTask?.cancel()
        let items = items
        let url = Self.fileURL
        saveTask = Task {
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled else { return }
            await Task.detached(priority: .utility) { Self.write(items, to: url) }.value
        }
    }

    /// 退出时同步写一次
    func saveNow() {
        saveTask?.cancel()
        Self.write(items, to: Self.fileURL)
    }

    private nonisolated static func write(_ items: [ClipItem], to url: URL) {
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try ClipHistoryList.encode(items).write(to: url, options: .atomic)
        } catch {
            Log.app.error("history save failed: \(String(describing: error), privacy: .public)")
        }
    }
}
