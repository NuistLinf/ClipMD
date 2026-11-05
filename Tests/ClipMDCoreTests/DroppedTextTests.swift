import AppKit
import Foundation
import Testing
@testable import ClipMDCore

@Suite struct DroppedTextTests {
    let folder = FileManager.default.temporaryDirectory.appending(path: "ClipMDDropTests-\(UUID().uuidString)")

    private func write(_ data: Data, as name: String) throws -> URL {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appending(path: name)
        try data.write(to: url)
        return url
    }

    @Test func utf8TextAndMarkdownFiles() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let txt = try write(Data("微信里的一段话".utf8), as: "消息.txt")
        let md = try write(Data("# 标题\n\n- 一".utf8), as: "笔记.md")
        #expect(DroppedText.load(txt)?.text == "微信里的一段话")
        #expect(DroppedText.load(md)?.text == "# 标题\n\n- 一")
    }

    @Test func gbkTextFile() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let gb18030 = String.Encoding(rawValue: CFStringConvertEncodingToNSStringEncoding(CFStringEncoding(CFStringEncodings.GB_18030_2000.rawValue)))
        let url = try write("中文 Windows 上存的文本".data(using: gb18030)!, as: "旧文件.txt")
        #expect(DroppedText.load(url)?.text == "中文 Windows 上存的文本")
    }

    @Test func rtfFile() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let attributed = NSAttributedString(string: "带格式的文字")
        let data = try #require(attributed.rtf(from: NSRange(location: 0, length: attributed.length), documentAttributes: [:]))
        let url = try write(data, as: "格式.rtf")
        #expect(DroppedText.load(url)?.text.trimmingCharacters(in: .whitespacesAndNewlines) == "带格式的文字")
    }

    @Test func htmlFileIsPassedAsHTML() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = try write(Data("<h1>网页</h1><p>正文</p>".utf8), as: "页面.html")
        let loaded = try #require(DroppedText.load(url))
        #expect(loaded.text.isEmpty)
        #expect(ClipboardMarkdown.best(plain: loaded.text, html: loaded.html).text == "# 网页\n\n正文")
    }

    @Test func nonTextFilesAreRejected() {
        #expect(!DroppedText.isTextFile(URL(filePath: "/tmp/图片.png")))
        #expect(!DroppedText.isTextFile(URL(filePath: "/tmp/压缩包.zip")))
        #expect(DroppedText.isTextFile(URL(filePath: "/tmp/脚本.py")))
        #expect(DroppedText.load(URL(filePath: "/tmp/图片.png")) == nil)
    }
}
