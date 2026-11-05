import AppKit // RTF 转文字要用 AppKit 的 NSAttributedString 扩展
import UniformTypeIdentifiers

/// 拖进悬浮窗的文本文件（.md、.txt、.html、.rtf……）读成文字。
public enum DroppedText {
    /// 太大的文件不读
    static let maxFileSize = 5_000_000

    /// 能当文字读的文件：Markdown、纯文本（含代码文件）、网页、RTF
    public static func isTextFile(_ url: URL) -> Bool {
        let ext = url.pathExtension.lowercased()
        if ["md", "markdown", "mdown", "mkd", "txt", "text", "html", "htm", "rtf"].contains(ext) { return true }
        guard let type = UTType(filenameExtension: ext) else { return false }
        return type.conforms(to: .plainText) || type.conforms(to: .html) || type.conforms(to: .rtf)
    }

    /// 读出来的纯文本和网页格式（网页文件只有 html，交给 `ClipboardMarkdown` 去转）
    public static func load(_ url: URL) -> (text: String, html: String?)? {
        guard isTextFile(url),
              let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= maxFileSize,
              let data = try? Data(contentsOf: url) else { return nil }
        switch url.pathExtension.lowercased() {
        case "html", "htm":
            return decode(data).map { ("", $0) }
        case "rtf":
            guard let text = rtfText(data) else { return nil }
            return (text, nil)
        default:
            return decode(data).map { ($0, nil) }
        }
    }

    /// 先按 UTF-8 读；不行再试中文 Windows 上常见的 GB18030（兼容 GBK、GB2312），最后试 UTF-16
    public static func decode(_ data: Data) -> String? {
        if let text = String(data: data, encoding: .utf8) { return text }
        let gb18030 = String.Encoding(rawValue: CFStringConvertEncodingToNSStringEncoding(CFStringEncoding(CFStringEncodings.GB_18030_2000.rawValue)))
        if let text = String(data: data, encoding: gb18030) { return text }
        return String(data: data, encoding: .utf16)
    }

    /// RTF 只取文字
    public static func rtfText(_ data: Data) -> String? {
        let text = (try? NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtf],
                                            documentAttributes: nil))?.string
        return text?.contains(where: { !$0.isWhitespace }) == true ? text : nil
    }
}
