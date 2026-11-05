import Foundation
import Testing
@testable import ClipMDCore

@Suite struct ClipHistoryListTests {
    private func item(_ text: String, html: String? = nil) -> ClipItem {
        ClipHistoryList.makeItem(text: text, html: html, sourceApp: "微信")!
    }

    @Test func newestFirst() {
        var list: [ClipItem] = []
        list = ClipHistoryList.adding(item("一"), to: list)
        list = ClipHistoryList.adding(item("二"), to: list)
        #expect(list.map(\.text) == ["二", "一"])
    }

    @Test func copyingAgainMovesToTop() {
        var list: [ClipItem] = []
        for text in ["一", "二", "三", "一"] { list = ClipHistoryList.adding(item(text), to: list) }
        #expect(list.map(\.text) == ["一", "三", "二"])
    }

    @Test func savedStateSurvivesRecopy() {
        var first = item("要存的")
        first.savedFileName = "2026-10-24 要存的.md"
        let list = ClipHistoryList.adding(item("要存的"), to: [first])
        #expect(list.count == 1)
        #expect(list[0].savedFileName == "2026-10-24 要存的.md")
    }

    @Test func keepsAtMostFiftyItems() {
        var list: [ClipItem] = []
        for index in 0..<60 { list = ClipHistoryList.adding(item("第 \(index) 条"), to: list) }
        #expect(list.count == ClipHistoryList.maxItems)
        #expect(list.first?.text == "第 59 条")
    }

    @Test func blankAndHugeTextsAreNotRecorded() {
        #expect(ClipHistoryList.makeItem(text: "  \n ", html: nil, sourceApp: nil) == nil)
        #expect(ClipHistoryList.makeItem(text: String(repeating: "字", count: ClipHistoryList.maxTextLength + 1), html: nil, sourceApp: nil) == nil)
    }

    @Test func hugeHTMLIsDroppedButTextKept() {
        let huge = String(repeating: "<p>x</p>", count: ClipHistoryList.maxHTMLLength)
        let recorded = ClipHistoryList.makeItem(text: "正文", html: huge, sourceApp: nil)
        #expect(recorded?.text == "正文")
        #expect(recorded?.html == nil)
    }

    @Test func titleDropsMarkdownHeadingMarks() {
        #expect(item("## 训练结果总结\n正文").title == "训练结果总结")
        #expect(item("#hashtag 不是标题").title == "#hashtag 不是标题")
        #expect(item("###   ").title == "###")
    }

    @Test func previewIsFirstNonEmptyLine() {
        #expect(item("\n\n   第一行内容  \n第二行").preview == "第一行内容")
    }

    @Test func roundTripsThroughJSON() throws {
        let list = [item("带格式的", html: "<b>带格式的</b>"), item("纯文字")]
        let decoded = ClipHistoryList.decode(try ClipHistoryList.encode(list))
        #expect(decoded.map(\.text) == list.map(\.text))
        #expect(decoded.first?.html == "<b>带格式的</b>")
        #expect(decoded.first?.sourceApp == "微信")
    }

    @Test func imagesDedupeByHashNotByEmptyText() {
        let a = ClipItem(imageFile: "a.png", width: 100, height: 50, hash: "aaa", date: .now, sourceApp: "截图")
        let b = ClipItem(imageFile: "b.png", width: 200, height: 80, hash: "bbb", date: .now, sourceApp: "截图")
        let again = ClipItem(imageFile: "a2.png", width: 100, height: 50, hash: "aaa", date: .now, sourceApp: "截图")
        var list = ClipHistoryList.adding(a, to: [])
        list = ClipHistoryList.adding(item("文字"), to: list)
        list = ClipHistoryList.adding(b, to: list)
        #expect(list.count == 3)
        list = ClipHistoryList.adding(again, to: list)
        #expect(list.count == 3)
        #expect(list.first?.imageFile == "a2.png")
        #expect(list.first?.preview == "图片 100 × 50")
        #expect(ClipHistoryList.referencedImages(in: list) == ["a2.png", "b.png"])
    }

    @Test func oldHistoryFilesWithoutImageFieldsStillLoad() {
        let old = #"[{"id":"\#(UUID().uuidString)","text":"旧记录","date":"2026-10-24T08:00:00Z","characterCount":3,"preview":"旧记录"}]"#
        let decoded = ClipHistoryList.decode(Data(old.utf8))
        #expect(decoded.first?.text == "旧记录")
        #expect(decoded.first?.isImage == false)
    }

    @Test func brokenFileMeansEmptyHistory() {
        #expect(ClipHistoryList.decode(Data("不是 JSON".utf8)).isEmpty)
    }
}
