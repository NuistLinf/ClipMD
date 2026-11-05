import Foundation
import Testing
@testable import ClipMDCore

@Suite struct NoteNamerTests {
    @Test func usesFirstHeading() {
        let text = "好的，下面是方案。\n\n## 如何设计剪贴板工具\n\n正文"
        #expect(NoteNamer.title(for: text) == "如何设计剪贴板工具")
    }

    @Test func fallsBackToFirstLine() {
        #expect(NoteNamer.title(for: "\n\n  **核心思路**：先触发再保存\n第二行") == "核心思路：先触发再保存")
    }

    @Test func skipsCodeFenceLine() {
        #expect(NoteNamer.title(for: "```swift\nlet a = 1\n```") == "let a = 1")
    }

    @Test func ignoresCommentsInsideCodeBlocks() {
        let text = "示例代码：\n```python\n# 这是注释\nprint(1)\n```\n## 真正的标题"
        #expect(NoteNamer.title(for: text) == "真正的标题")
    }

    @Test func stripsListMarkersAndLinks() {
        #expect(NoteNamer.title(for: "- 看 [官方文档](https://example.com) 的说明") == "看 官方文档 的说明")
    }

    @Test func removesIllegalFileNameCharacters() {
        #expect(NoteNamer.title(for: "# A/B: 对比?") == "A B 对比")
    }

    @Test func truncatesLongTitles() {
        let title = NoteNamer.title(for: String(repeating: "长", count: 80))
        #expect(title.count == 30)
    }

    @Test func englishTitlesBreakAtWords() {
        #expect(NoteNamer.title(for: "## Derivation by completing the square") == "Derivation by completing the")
        // 中文照常按字数截
        #expect(NoteNamer.title(for: String(repeating: "中", count: 40)).count == 30)
    }

    @Test func emptyTextGetsFallback() {
        #expect(NoteNamer.title(for: "   \n\n") == NoteNamer.fallbackTitle)
        #expect(NoteNamer.title(for: "...") == NoteNamer.fallbackTitle)
    }

    @Test func fileNameHasDatePrefix() {
        let date = Date(timeIntervalSince1970: 1_793_000_000)
        let name = NoteNamer.fileName(title: "标题", date: date)
        #expect(name.hasSuffix(" 标题.md"))
        #expect(name.range(of: #"^\d{4}-\d{2}-\d{2} "#, options: .regularExpression) != nil)
        #expect(NoteNamer.fileName(title: "标题", date: date, suffix: 2).hasSuffix(" 标题 2.md"))
    }
}

@Suite struct NoteWriterTests {
    let folder = FileManager.default.temporaryDirectory.appending(path: "ClipMDTests-\(UUID().uuidString)")

    @Test func savesUniqueFilesAndListsNewestFirst() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let first = try NoteWriter.save(text: "# 同名\n一", to: folder)
        let second = try NoteWriter.save(text: "# 同名\n二二", to: folder)

        #expect(first.url != second.url)
        #expect(second.url.lastPathComponent.hasSuffix("同名 2.md"))
        #expect(try String(contentsOf: first.url, encoding: .utf8) == "# 同名\n一\n")

        let recent = NoteWriter.recentNotes(in: folder, limit: 5)
        #expect(recent.count == 2)
        #expect(recent.allSatisfy { $0.title.hasPrefix("同名") })
    }

    @Test func missingFolderListsNothing() {
        #expect(NoteWriter.recentNotes(in: folder, limit: 5).isEmpty)
    }
}
