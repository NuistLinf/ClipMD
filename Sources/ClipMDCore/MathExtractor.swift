import Foundation

/// 一段 LaTeX 公式
public struct MathSnippet: Equatable, Sendable {
    public let latex: String
    /// `$$…$$` / `\[…\]` 是独立成行的大号公式，`$…$` / `\(…\)` 是行内公式
    public let display: Bool
}

/// Markdown 解析器不认识公式，而且会把公式里的 `_`、`*` 当成斜体。
/// 所以在解析前把公式换成占位符，排版时再换回渲染好的公式。
///
/// 占位符用 Unicode 私有区字符包住序号，Markdown 不会动它们：
/// - 行内：`\u{E000}序号\u{E001}`
/// - 独立成段：`\u{E002}序号\u{E003}`，单独一段
public enum MathExtractor {
    public static let inlineOpen: Character = "\u{E000}"
    public static let inlineClose: Character = "\u{E001}"
    public static let blockOpen: Character = "\u{E002}"
    public static let blockClose: Character = "\u{E003}"

    public static func extract(_ source: String) -> (markdown: String, math: [MathSnippet]) {
        // 没有 $ 和 \ 就不可能有公式，大多数文档直接跳过
        guard source.contains("$") || source.contains("\\[") || source.contains("\\(") else { return (source, []) }

        var math: [MathSnippet] = []
        var output: [String] = []
        let lines = source.components(separatedBy: "\n")
        var fence: String?
        var index = 0

        while index < lines.count {
            let line = lines[index]
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            // 代码块里的内容原样保留
            if let open = fence {
                output.append(line)
                if trimmed.hasPrefix(open) { fence = nil }
                index += 1
                continue
            }
            if let marker = fenceMarker(trimmed) {
                fence = marker
                output.append(line)
                index += 1
                continue
            }

            let (prefix, body) = splitPrefix(line)
            if let (latex, consumed) = displayMath(startingAt: index, lines: lines, prefix: prefix, body: body) {
                let token = "\(blockOpen)\(math.count)\(blockClose)"
                math.append(MathSnippet(latex: latex, display: true))
                // 前后空一行，保证它单独成段；保留列表缩进和引用符号
                let blank = prefix.trimmingCharacters(in: .whitespaces)
                output.append(contentsOf: [blank, prefix + token, blank])
                index += consumed
                continue
            }

            output.append(replaceInline(in: line, math: &math))
            index += 1
        }
        return (output.joined(separator: "\n"), math)
    }

    // MARK: 独立成段的公式

    /// 以 `$$` 或 `\[` 开头的行。返回公式内容和用掉的行数。
    private static func displayMath(startingAt start: Int, lines: [String], prefix: String, body: String)
        -> (String, Int)? {
        let body = body.trimmingCharacters(in: .whitespaces)
        let delimiters = [("$$", "$$"), ("\\[", "\\]")]
        guard let (open, close) = delimiters.first(where: { body.hasPrefix($0.0) }) else { return nil }

        let afterOpen = String(body.dropFirst(open.count))
        // 同一行就结束：$$ x^2 $$
        if let end = afterOpen.range(of: close) {
            let rest = afterOpen[end.upperBound...].trimmingCharacters(in: .whitespaces)
            // 后面还有别的文字，说明是写在句子里的，交给行内处理
            guard rest.isEmpty else { return nil }
            let latex = String(afterOpen[..<end.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
            return latex.isEmpty ? nil : (latex, 1)
        }

        // 跨多行，往下找结束符；最多看 200 行，找不到就当普通文字
        var parts = [afterOpen]
        var index = start + 1
        while index < lines.count, index - start <= 200 {
            let (_, lineBody) = splitPrefix(lines[index])
            if let end = lineBody.range(of: close) {
                let rest = lineBody[end.upperBound...].trimmingCharacters(in: .whitespaces)
                guard rest.isEmpty else { return nil }
                parts.append(String(lineBody[..<end.lowerBound]))
                let latex = parts.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
                return latex.isEmpty ? nil : (latex, index - start + 1)
            }
            parts.append(lineBody)
            index += 1
        }
        return nil
    }

    /// 把行首的缩进和引用符号（`> `）分出来
    private static func splitPrefix(_ line: String) -> (prefix: String, body: String) {
        var prefixEnd = line.startIndex
        for index in line.indices {
            let character = line[index]
            if character == " " || character == "\t" || character == ">" {
                prefixEnd = line.index(after: index)
            } else {
                break
            }
        }
        return (String(line[..<prefixEnd]), String(line[prefixEnd...]))
    }

    private static func fenceMarker(_ trimmed: String) -> String? {
        if trimmed.hasPrefix("```") { return "```" }
        if trimmed.hasPrefix("~~~") { return "~~~" }
        return nil
    }

    // MARK: 行内公式

    private static func replaceInline(in line: String, math: inout [MathSnippet]) -> String {
        guard line.contains("$") || line.contains("\\(") else { return line }
        let characters = Array(line)
        var result = ""
        var i = 0

        func token(_ latex: String, display: Bool) -> String {
            defer { math.append(MathSnippet(latex: latex, display: display)) }
            return "\(inlineOpen)\(math.count)\(inlineClose)"
        }

        while i < characters.count {
            let character = characters[i]

            // 行内代码原样保留
            if character == "`" {
                var run = 0
                while i + run < characters.count, characters[i + run] == "`" { run += 1 }
                if let end = findBacktickRun(characters, from: i + run, length: run) {
                    result += String(characters[i..<(end + run)])
                    i = end + run
                } else {
                    result += String(characters[i..<(i + run)])
                    i += run
                }
                continue
            }

            // \$ 是转义，\( … \) 是行内公式
            if character == "\\", i + 1 < characters.count {
                let next = characters[i + 1]
                if next == "(", let end = find(characters, "\\)", from: i + 2) {
                    let latex = String(characters[(i + 2)..<end]).trimmingCharacters(in: .whitespaces)
                    if !latex.isEmpty {
                        result += token(latex, display: false)
                        i = end + 2
                        continue
                    }
                }
                result.append(character)
                result.append(next)
                i += 2
                continue
            }

            if character == "$" {
                // 句子中间的 $$ … $$，按大号公式画在行内
                if i + 1 < characters.count, characters[i + 1] == "$" {
                    if let end = find(characters, "$$", from: i + 2) {
                        let latex = String(characters[(i + 2)..<end]).trimmingCharacters(in: .whitespaces)
                        if !latex.isEmpty {
                            result += token(latex, display: true)
                            i = end + 2
                            continue
                        }
                    }
                    result += "$$"
                    i += 2
                    continue
                }
                if let end = closingDollar(characters, openAt: i) {
                    result += token(String(characters[(i + 1)..<end]), display: false)
                    i = end + 1
                    continue
                }
            }

            result.append(character)
            i += 1
        }
        return result
    }

    /// pandoc 的规则：开头的 $ 后面不能是空白；结尾的 $ 前面不能是空白，后面不能紧跟数字。
    /// 这样 "$5 和 $10" 不会被当成公式。
    private static func closingDollar(_ characters: [Character], openAt open: Int) -> Int? {
        let start = open + 1
        guard start < characters.count, !characters[start].isWhitespace, characters[start] != "$" else { return nil }
        var j = start
        while j < characters.count {
            if characters[j] == "\\" {
                j += 2
                continue
            }
            if characters[j] == "$" {
                let before = characters[j - 1]
                let after = j + 1 < characters.count ? characters[j + 1] : " "
                if !before.isWhitespace, !after.isNumber, j > start { return j }
                return nil
            }
            j += 1
        }
        return nil
    }

    private static func find(_ characters: [Character], _ needle: String, from start: Int) -> Int? {
        let pattern = Array(needle)
        guard start <= characters.count - pattern.count else { return nil }
        for j in start...(characters.count - pattern.count) where Array(characters[j..<(j + pattern.count)]) == pattern {
            return j
        }
        return nil
    }

    private static func findBacktickRun(_ characters: [Character], from start: Int, length: Int) -> Int? {
        var j = start
        while j < characters.count {
            if characters[j] == "`" {
                var run = 0
                while j + run < characters.count, characters[j + run] == "`" { run += 1 }
                if run == length { return j }
                j += run
            } else {
                j += 1
            }
        }
        return nil
    }
}
