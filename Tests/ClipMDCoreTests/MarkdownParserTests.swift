import Foundation
import Testing
@testable import ClipMDCore

@Suite struct MarkdownParserTests {
    private func plain(_ text: AttributedString) -> String { String(text.characters) }

    @Test func headingsAndParagraphs() {
        let blocks = MarkdownParser.blocks(from: "# 标题\n\n第一段 **粗体**\n第二行")
        #expect(blocks.map(\.kind) == [.heading(level: 1), .paragraph])
        #expect(plain(blocks[1].text) == "第一段 粗体 第二行")
    }

    @Test func nestedListsGetMarkersAndDepth() {
        let blocks = MarkdownParser.blocks(from: "- 一\n  - 嵌套\n- 二\n\n1. 甲\n2. 乙")
        #expect(blocks.map(\.listDepth) == [1, 2, 1, 1, 1])
        #expect(blocks.map(\.listMarker) == ["•", "◦", "•", "1.", "2."])
    }

    @Test func secondParagraphInListItemHasNoMarker() {
        let blocks = MarkdownParser.blocks(from: "- 第一段\n\n  同一项的第二段\n- 下一项")
        #expect(blocks.map(\.listMarker) == ["•", nil, "•"])
    }

    @Test func codeBlockKeepsLanguageAndDropsTrailingNewline() {
        let blocks = MarkdownParser.blocks(from: "```swift\nlet a = 1\n```")
        #expect(blocks.first?.kind == .codeBlock(language: "swift"))
        #expect(blocks.first?.code == "let a = 1")
    }

    @Test func quoteAndRule() {
        let blocks = MarkdownParser.blocks(from: "> 引用\n\n---\n\n正文")
        #expect(blocks.map(\.kind) == [.paragraph, .thematicBreak, .paragraph])
        #expect(blocks[0].quoteDepth == 1)
    }

    @Test func tableCellsAreCollected() {
        let blocks = MarkdownParser.blocks(from: "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |")
        #expect(blocks.count == 1)
        #expect(blocks[0].kind == .table)
        #expect(blocks[0].tableRows.map { $0.map(plain) } == [["A", "B"], ["1", "2"], ["3", "4"]])
    }

    @Test func idsAreUniqueAndOrdered() {
        let blocks = MarkdownParser.blocks(from: "# A\n\n| x |\n|---|\n| y |\n\n段落")
        #expect(blocks.map(\.id) == Array(0..<blocks.count))
    }
}

@Suite struct NoteWriterEditingTests {
    let folder = FileManager.default.temporaryDirectory.appending(path: "ClipMDEditTests-\(UUID().uuidString)")

    @Test func hiddenNotesAreSkippedAndBackfilled() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let a = try NoteWriter.save(text: "# 甲", to: folder)
        _ = try NoteWriter.save(text: "# 乙", to: folder)
        let recent = NoteWriter.recentNotes(in: folder, limit: 5, hidden: [a.url.lastPathComponent])
        #expect(recent.map(\.title) == ["乙"])
    }

    @Test func overwriteKeepsCreationDate() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let note = try NoteWriter.save(text: "# 原文", to: folder)
        let path = note.url.path(percentEncoded: false)
        let before = try FileManager.default.attributesOfItem(atPath: path)[.creationDate] as? Date
        Thread.sleep(forTimeInterval: 1.1)
        try NoteWriter.overwrite(text: "# 改过了\n", at: note.url)
        let after = try FileManager.default.attributesOfItem(atPath: path)[.creationDate] as? Date
        #expect(before != nil)
        #expect(before == after)
        #expect(try String(contentsOf: note.url, encoding: .utf8) == "# 改过了\n")
    }
}
