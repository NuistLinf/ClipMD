import Foundation

/// 剪贴板里同时有纯文本和网页格式（HTML）时，决定存哪一份。
public enum ClipboardMarkdown {
    public enum Source: String, Sendable {
        /// 用纯文本：本来就是 Markdown，或者没有网页格式
        case plain
        /// 用网页格式转出来的 Markdown
        case html
    }

    public static func best(plain: String, html: String?) -> (text: String, source: Source) {
        // AI 的复制按钮给的一般已经是 Markdown，直接用
        guard let html, !looksLikeMarkdown(plain), let converted = HTMLToMarkdown.convert(html) else {
            return (plain, .plain)
        }
        // 网页本来就没什么格式，转出来除了空白都一样，保持原样
        if withoutWhitespace(converted) == withoutWhitespace(plain) { return (plain, .plain) }
        // 转换后明显少了内容（比如文字在不认识的标签里被跳过了），宁可不转也不能丢字
        if contentLength(converted) < contentLength(plain) * 7 / 10 { return (plain, .plain) }
        return (converted, .html)
    }

    /// 有这些明确的 Markdown 写法之一，就当它已经是 Markdown。
    /// 不看 `# 标题`：网页里的 Python、Shell 代码有很多 `#` 开头的注释，复制成纯文本后会误判。
    public static func looksLikeMarkdown(_ text: String) -> Bool {
        let patterns = [
            #"(?m)^\s*(```|~~~)"#,      // 代码块
            #"\*\*[^*\n]+\*\*"#,        // 粗体
            #"\[[^\]\n]+\]\([^)\s]+\)"#, // 链接
            #"(?m)^\s*\|?\s*:?-{3,}"#,  // 表格分隔线
        ]
        return patterns.contains { text.range(of: $0, options: .regularExpression) != nil }
    }

    private static func withoutWhitespace(_ text: String) -> String {
        text.unicodeScalars.filter { !CharacterSet.whitespacesAndNewlines.contains($0) }.map(String.init).joined()
    }

    /// 去掉所有空白和 Markdown 符号，只比较文字本身
    private static func normalized(_ text: String) -> String {
        text.unicodeScalars
            .filter { !CharacterSet.whitespacesAndNewlines.contains($0) && !"#*_`>-|[]()!~\\".unicodeScalars.contains($0) }
            .map(String.init).joined()
    }

    private static func contentLength(_ text: String) -> Int {
        normalized(text).count
    }
}
