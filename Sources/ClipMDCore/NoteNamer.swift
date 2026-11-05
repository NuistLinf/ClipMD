import Foundation

/// 根据内容给 md 文件起名：优先第一个 Markdown 标题，没有就用第一行非空文本。
public enum NoteNamer {
    public static let fallbackTitle = "未命名"

    /// 只看开头这么多字符，避免长文本整篇扫描
    static let scanLimit = 4000

    public static func title(for text: String, maxLength: Int = 30) -> String {
        let head = String(text.prefix(scanLimit))
        var firstLine: Substring?
        var heading: Substring?
        var inFence = false

        head.enumerateSubstrings(in: head.startIndex..., options: [.byLines, .substringNotRequired]) { _, range, _, stop in
            let line = head[range].trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                inFence.toggle()
                return
            }
            guard !line.isEmpty else { return }
            // 代码块里的 `# 注释` 不算标题
            if line.hasPrefix("#"), !inFence {
                heading = Substring(line)
                stop = true
            } else if firstLine == nil {
                firstLine = Substring(line)
            }
        }

        let raw = heading ?? firstLine ?? ""
        let cleaned = sanitize(stripMarkdown(raw))
        guard !cleaned.isEmpty else { return fallbackTitle }
        return truncate(cleaned, to: maxLength)
    }

    /// `2026-10-24 标题.md`
    public static func fileName(title: String, date: Date, suffix: Int? = nil) -> String {
        let day = date.formatted(Date.ISO8601FormatStyle(timeZone: .current).year().month().day().dateSeparator(.dash))
        let tail = suffix.map { " \($0)" } ?? ""
        return "\(day) \(title)\(tail).md"
    }

    static func stripMarkdown(_ line: Substring) -> String {
        var s = String(line)
        // 标题、引用、列表前缀
        while let first = s.first, "#>".contains(first) { s.removeFirst() }
        s = s.trimmingCharacters(in: .whitespaces)
        if let range = s.range(of: #"^([-*+]|\d+[.)])\s+"#, options: .regularExpression) {
            s.removeSubrange(range)
        }
        // 强调和行内代码标记
        for mark in ["**", "__", "`", "*", "~~"] {
            s = s.replacingOccurrences(of: mark, with: "")
        }
        // [文字](链接) 只留文字
        s = s.replacingOccurrences(of: #"\[([^\]]*)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
        return s
    }

    static func sanitize(_ s: String) -> String {
        let illegal = CharacterSet(charactersIn: "/\\:?*\"<>|").union(.newlines).union(.controlCharacters)
        let parts = s.components(separatedBy: illegal).joined(separator: " ")
        let collapsed = parts.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        // 文件名以点开头会被隐藏
        return collapsed.trimmingCharacters(in: CharacterSet(charactersIn: ". ").union(.whitespaces))
    }

    /// 英文在单词中间截断不好看（"completing the s"），往回退到最近的空格；中文直接截
    static func truncate(_ s: String, to maxLength: Int) -> String {
        guard s.count > maxLength else { return s }
        let cut = s.prefix(maxLength)
        let next = s[cut.endIndex]
        if let last = cut.last, last.isASCII, last.isLetter || last.isNumber, next.isASCII, next.isLetter || next.isNumber,
           let space = cut.lastIndex(of: " "), cut.distance(from: cut.startIndex, to: space) >= maxLength / 2 {
            return String(cut[..<space]).trimmingCharacters(in: .whitespaces)
        }
        return String(cut).trimmingCharacters(in: .whitespaces)
    }
}
