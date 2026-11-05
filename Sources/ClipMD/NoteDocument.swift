import ClipMDCore
import Foundation
import Observation

/// 预览窗口里打开的一个 md 文件：内容、渲染好的块、保存状态。
@MainActor
@Observable
final class NoteDocument {
    enum Mode: String, CaseIterable, Identifiable {
        case preview, edit, split

        var id: Self { self }

        var title: String {
            switch self {
            case .preview: "预览"
            case .edit: "编辑"
            case .split: "并排"
            }
        }
    }

    enum SaveState: Equatable {
        case saved, editing, saving
        case failed(String)
    }

    private static let modeKey = "previewMode"

    let url: URL
    let title: String
    private(set) var text = ""
    /// 渲染好的块和公式
    private(set) var parsed = MarkdownDocument(blocks: [], math: [])
    var blocks: [MarkdownBlock] { parsed.blocks }
    private(set) var characterCount = 0
    private(set) var saveState = SaveState.saved
    private(set) var isLoaded = false
    private(set) var mode: Mode
    /// 保存成功后通知列表刷新字数
    var onSaved: (@MainActor () -> Void)?

    private var parseTask: Task<Void, Never>?
    private var saveTask: Task<Void, Never>?
    /// 文件被移到废纸篓后不再写回，免得把它重新建出来
    private var discarded = false

    init(note: SavedNote) {
        url = note.url
        title = note.title
        mode = UserDefaults.standard.string(forKey: Self.modeKey).flatMap(Mode.init(rawValue:)) ?? .preview
    }

    func setMode(_ mode: Mode) {
        self.mode = mode
        UserDefaults.standard.set(mode.rawValue, forKey: Self.modeKey)
    }

    func load() async {
        let url = url
        do {
            let (text, parsed, count) = try await Task.detached(priority: .userInitiated) {
                let text = try String(contentsOf: url, encoding: .utf8)
                return (text, MarkdownParser.parse(text), text.count)
            }.value
            self.text = text
            self.parsed = parsed
            characterCount = count
        } catch {
            saveState = .failed("读取失败：\(error.localizedDescription)")
        }
        isLoaded = true
    }

    func update(text newText: String) {
        guard newText != text, !discarded else { return }
        text = newText
        saveState = .editing
        scheduleParse()
        scheduleSave()
    }

    /// 停止输入 150 毫秒后在后台重新渲染
    private func scheduleParse() {
        parseTask?.cancel()
        let text = text
        parseTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }
            let (parsed, count) = await Task.detached(priority: .userInitiated) {
                (MarkdownParser.parse(text), text.count)
            }.value
            guard let self, !Task.isCancelled else { return }
            self.parsed = parsed
            self.characterCount = count
        }
    }

    /// 停止输入 0.8 秒后自动保存
    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(800))
            guard let self, !Task.isCancelled else { return }
            await self.saveNow()
        }
    }

    func saveNow() async {
        guard !discarded, needsSave else { return }
        saveTask?.cancel()
        let text = text, url = url
        saveState = .saving
        do {
            try await Task.detached(priority: .userInitiated) {
                try NoteWriter.overwrite(text: text, at: url)
            }.value
            if self.text == text {
                saveState = .saved
            } else {
                // 保存期间又改了
                saveState = .editing
                scheduleSave()
            }
            onSaved?()
        } catch {
            Log.app.error("preview save failed: \(String(describing: error), privacy: .public)")
            saveState = .failed(error.localizedDescription)
        }
    }

    /// 退出应用时同步写盘
    func saveSynchronously() {
        guard !discarded, needsSave else { return }
        try? NoteWriter.overwrite(text: text, at: url)
        saveState = .saved
    }

    func discard() {
        discarded = true
        saveTask?.cancel()
        parseTask?.cancel()
    }

    private var needsSave: Bool {
        switch saveState {
        case .editing, .failed: true
        case .saved, .saving: false
        }
    }

    var statusText: String {
        let count = "\(characterCount.formatted()) 字"
        switch saveState {
        case .saved: return "\(count) · 已保存"
        case .editing: return "\(count) · 正在编辑"
        case .saving: return "\(count) · 保存中…"
        case .failed(let message): return "保存失败：\(message)"
        }
    }
}
