import ClipMDCore
import Foundation
import Observation
import SwiftUI

/// 「已保存」列表里的最近记录（最多 50 条）。文件读写都放到后台线程，主线程只更新数组。
@MainActor
@Observable
final class NoteStore {
    /// 「已保存」列表最多列出最近这么多个文件（露出 5 条，其余滚动查看）
    static let listLimit = 50
    private static let folderKey = "saveFolder"
    private static let hiddenKey = "hiddenNotes"

    private(set) var notes: [SavedNote] = []
    private(set) var folder: URL
    /// 相对时间（"3 分钟前"）的参照点。只在悬浮窗被看到时刷新，不跑定时器。
    private(set) var now = Date.now
    /// 从列表里移除、但文件还留着的文件名
    private var hidden: Set<String>

    init() {
        // 也接受启动参数 `-saveFolder <路径>`（UserDefaults 的参数域）
        let folder = UserDefaults.standard.string(forKey: Self.folderKey)
            .map { URL(filePath: $0, directoryHint: .isDirectory) }
            ?? URL.documentsDirectory.appending(path: "ClipMD", directoryHint: .isDirectory)
        self.folder = folder
        // 文件已经不在了的记录顺手清掉，免得越积越多
        hidden = Set(UserDefaults.standard.stringArray(forKey: Self.hiddenKey) ?? []).filter {
            FileManager.default.fileExists(atPath: folder.appending(path: $0).path(percentEncoded: false))
        }
    }

    // MARK: 删除

    /// 只从列表里移除，文件不动
    func hide(_ note: SavedNote) {
        hidden.insert(note.url.lastPathComponent)
        saveHidden()
        removeFromList(note)
    }

    func unhide(_ note: SavedNote) {
        hidden.remove(note.url.lastPathComponent)
        saveHidden()
        reload()
    }

    /// 移到废纸篓（可以从废纸篓里找回），返回它在废纸篓里的位置
    func trash(_ note: SavedNote) throws -> URL {
        var trashed: NSURL?
        try FileManager.default.trashItem(at: note.url, resultingItemURL: &trashed)
        removeFromList(note)
        return (trashed as URL?) ?? note.url
    }

    func restore(_ note: SavedNote, from trashed: URL) throws {
        try FileManager.default.moveItem(at: trashed, to: note.url)
        reload()
    }

    private func removeFromList(_ note: SavedNote) {
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            notes.removeAll { $0.id == note.id }
        }
        // 补上下一条
        reload()
    }

    private func saveHidden() {
        UserDefaults.standard.set(Array(hidden).sorted(), forKey: Self.hiddenKey)
    }

    func setFolder(_ url: URL) {
        folder = url
        UserDefaults.standard.set(url.path(percentEncoded: false), forKey: Self.folderKey)
        reload()
    }

    func reload() {
        let folder = folder
        let limit = Self.listLimit
        let hidden = hidden
        Task {
            let fresh = await Task.detached(priority: .utility) {
                NoteWriter.recentNotes(in: folder, limit: limit, hidden: hidden)
            }.value
            guard fresh != notes else { return }
            withAnimation(.spring(response: 0.45, dampingFraction: 0.85)) { notes = fresh }
        }
    }

    /// `html` 是剪贴板里的网页格式；有的话在后台判断要不要转成 Markdown，见 `ClipboardMarkdown`
    func save(_ text: String, html: String? = nil) async throws -> (SavedNote, ClipboardMarkdown.Source) {
        let folder = folder
        let (note, source) = try await Task.detached(priority: .userInitiated) {
            let best = ClipboardMarkdown.best(plain: text, html: html)
            return (try NoteWriter.save(text: best.text, to: folder), best.source)
        }.value

        now = .now
        withAnimation(.spring(response: 0.45, dampingFraction: 0.8)) {
            notes.insert(note, at: 0)
            if notes.count > Self.listLimit { notes.removeLast(notes.count - Self.listLimit) }
        }
        return (note, source)
    }

    /// 文件在外面被删掉或改名时调用
    func noteMissing() { reload() }

    @ObservationIgnored private var lastReload = Date.distantPast

    /// 鼠标移进悬浮窗时调用：在 Finder 里删掉、改名的文件也能及时反映出来。3 秒内只扫一次。
    func reloadIfStale() {
        guard Date.now.timeIntervalSince(lastReload) > 3 else { return }
        lastReload = .now
        reload()
    }

    func refreshClock() {
        // 半分钟以内不刷新，避免鼠标进出就重绘
        if Date.now.timeIntervalSince(now) > 30 { now = .now }
    }
}
