import Foundation

/// 把网页复制出来的 HTML 转成 Markdown：标题、列表、代码块、表格、引用、粗体斜体、链接、图片，
/// 以及网页上的公式（KaTeX、维基百科的 MathML，取出里面的 LaTeX 原文）。
///
/// 用 Foundation 的 `XMLDocument` 的 HTML 整理模式解析，能容忍没闭合的标签这类不规范的网页代码。
public enum HTMLToMarkdown {
    /// 太大的 HTML 不转（一般是整页复制，内联样式很多），直接用纯文本
    static let maxInputLength = 2_000_000

    public static func convert(_ html: String) -> String? {
        guard html.count <= maxInputLength,
              let document = try? XMLDocument(xmlString: keepCodeBlocksIntact(preserveTeX(in: html)), options: [.documentTidyHTML]),
              let root = document.rootElement() else { return nil }
        let body = root.elements(forName: "body").first ?? root
        let markdown = Renderer().blocks(body.children ?? [], tight: false)
        let cleaned = markdown
            .replacingOccurrences(of: #"\n{3,}"#, with: "\n\n", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return cleaned.isEmpty ? nil : cleaned
    }

    /// 整理器按老的 HTML 规则，`<pre>` 里不能有 `<div>`，会把代码块拆开。
    /// ChatGPT 这类网页的代码块正好是 `<pre><div>语言名、复制按钮</div><div><code>…`，所以先把里面的 div 换成 span。
    static func keepCodeBlocksIntact(_ html: String) -> String {
        guard html.range(of: "<pre", options: .caseInsensitive) != nil,
              let pre = try? NSRegularExpression(pattern: "<pre\\b.*?</pre>", options: [.caseInsensitive, .dotMatchesLineSeparators]) else { return html }
        let source = html as NSString
        var result = ""
        var cursor = 0
        for match in pre.matches(in: html, range: NSRange(location: 0, length: source.length)) {
            result += source.substring(with: NSRange(location: cursor, length: match.range.location - cursor))
            result += source.substring(with: match.range)
                .replacingOccurrences(of: "<div\\b", with: "<span", options: [.regularExpression, .caseInsensitive])
                .replacingOccurrences(of: "</div>", with: "</span>", options: .caseInsensitive)
            cursor = match.range.location + match.range.length
        }
        return result + source.substring(from: cursor)
    }

    /// HTML 整理器不认识 MathML，会把 `<annotation>` 里的 LaTeX 原文和显示用的文字揉在一起。
    /// 先把 LaTeX 原文挪进它认识的 `<var class="clipmd-tex">`，解析完再取出来。
    static func preserveTeX(in html: String) -> String {
        guard html.contains("application/x-tex") else { return html }
        return html.replacingOccurrences(
            of: #"<annotation[^>]*encoding=["']application/x-tex["'][^>]*>(.*?)</annotation>"#,
            with: "<var class=\"clipmd-tex\">$1</var>",
            options: [.regularExpression, .caseInsensitive]
        )
    }
}

private struct Renderer {
    /// 不输出任何内容的标签
    static let skipped: Set<String> = [
        "script", "style", "head", "title", "meta", "link", "noscript", "button", "svg", "iframe",
        "form", "select", "textarea", "template", "object", "embed", "canvas", "audio", "video",
    ]
    /// 块级标签：前后要断开
    static let blockTags: Set<String> = [
        "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "pre", "blockquote", "table",
        "hr", "section", "article", "main", "header", "footer", "aside", "nav", "figure", "figcaption",
        "dl", "dt", "dd", "center", "address", "details", "summary", "body", "html",
    ]
    /// `<br>` 先用这个字符占位，合并空白之后再换成换行
    static let lineBreak: Character = "\u{E010}"

    // MARK: 块

    /// 一组节点渲染成块级 Markdown。`tight` 为 true 时块之间只换一行（用在列表项里）
    func blocks(_ nodes: [XMLNode], tight: Bool) -> String {
        var output: [String] = []
        var inline = ""
        func flushInline() {
            let text = Self.finishInline(inline)
            if !text.isEmpty { output.append(text) }
            inline = ""
        }
        for node in nodes {
            if let element = node as? XMLElement, isBlock(element) {
                flushInline()
                if let block = block(element), !block.isEmpty { output.append(block) }
            } else {
                inline += self.inline(node)
            }
        }
        flushInline()
        return output.joined(separator: tight ? "\n" : "\n\n")
    }

    private func isBlock(_ element: XMLElement) -> Bool {
        let name = element.localName?.lowercased() ?? ""
        return Self.blockTags.contains(name) || Self.skipped.contains(name) || element.isDisplayMath
    }

    private func block(_ element: XMLElement) -> String? {
        let name = element.localName?.lowercased() ?? ""
        if Self.skipped.contains(name) || element.isDecoration { return nil }
        if element.isMathContainer, let tex = element.tex {
            return element.isDisplayMath ? "$$\n\(tex)\n$$" : "$\(tex)$"
        }

        switch name {
        case "h1", "h2", "h3", "h4", "h5", "h6":
            let level = Int(name.dropFirst()) ?? 1
            let text = Self.finishInline(inlineChildren(element)).replacingOccurrences(of: "\n", with: " ")
            return text.isEmpty ? nil : String(repeating: "#", count: level) + " " + text
        case "p", "dt", "dd", "figcaption", "summary", "address":
            return Self.finishInline(inlineChildren(element))
        case "ul", "ol":
            return list(element, ordered: name == "ol")
        case "li":
            return listItem(element, marker: "-")
        case "pre":
            return codeBlock(element)
        case "blockquote":
            let inner = blocks(element.children ?? [], tight: false)
            guard !inner.isEmpty else { return nil }
            return inner.components(separatedBy: "\n").map { $0.isEmpty ? ">" : "> " + $0 }.joined(separator: "\n")
        case "table":
            return table(element)
        case "hr":
            return "---"
        default:
            return blocks(element.children ?? [], tight: false)
        }
    }

    private func list(_ element: XMLElement, ordered: Bool) -> String? {
        var number = Int(element.attribute(forName: "start")?.stringValue ?? "") ?? 1
        var items: [String] = []
        for child in element.children ?? [] {
            guard let item = child as? XMLElement else { continue }
            let name = item.localName?.lowercased()
            if name == "li" {
                let marker = ordered ? "\(number)." : "-"
                number += 1
                if let text = listItem(item, marker: marker) { items.append(text) }
            } else if name == "ul" || name == "ol", let nested = list(item, ordered: name == "ol") {
                // 不规范的嵌套：列表直接套列表
                items.append(nested.components(separatedBy: "\n").map { "  " + $0 }.joined(separator: "\n"))
            }
        }
        return items.isEmpty ? nil : items.joined(separator: "\n")
    }

    /// 第一行带列表符号，后面的行缩进对齐
    private func listItem(_ item: XMLElement, marker: String) -> String? {
        let content = blocks(item.children ?? [], tight: true)
        let indent = String(repeating: " ", count: marker.count + 1)
        let lines = content.components(separatedBy: "\n")
        guard let first = lines.first else { return nil }
        let rest = lines.dropFirst().map { $0.isEmpty ? "" : indent + $0 }
        return ([marker + " " + first] + rest).joined(separator: "\n")
    }

    private func codeBlock(_ pre: XMLElement) -> String? {
        // ChatGPT 之类的网页会在 <pre> 里放语言名、"复制"按钮，真正的代码在 <code> 里
        let code = pre.firstDescendant { $0.localName?.lowercased() == "code" }
        var text = (code ?? pre).stringValue ?? ""
        text = text.replacingOccurrences(of: "\u{00A0}", with: " ")
        while text.hasPrefix("\n") { text.removeFirst() }
        while text.hasSuffix("\n") || text.hasSuffix(" ") { text.removeLast() }
        guard !text.isEmpty else { return nil }
        let language = Self.language(from: code) ?? Self.language(from: pre) ?? ""
        let fence = String(repeating: "`", count: max(3, Self.longestRun(of: "`", in: text) + 1))
        return "\(fence)\(language)\n\(text)\n\(fence)"
    }

    private func table(_ element: XMLElement) -> String? {
        let rows = element.descendants { $0.localName?.lowercased() == "tr" }
        let cells: [[String]] = rows.map { row in
            (row.children ?? []).compactMap { $0 as? XMLElement }
                .filter { ["td", "th"].contains($0.localName?.lowercased() ?? "") }
                .map {
                    Self.finishInline(inlineChildren($0))
                        .replacingOccurrences(of: "\n", with: " ")
                        .replacingOccurrences(of: "|", with: "\\|")
                }
        }.filter { !$0.isEmpty }
        guard let header = cells.first else { return nil }
        let width = cells.map(\.count).max() ?? header.count
        func line(_ row: [String]) -> String {
            "| " + (row + Array(repeating: "", count: width - row.count)).joined(separator: " | ") + " |"
        }
        let separator = "|" + Array(repeating: " --- |", count: width).joined()
        return ([line(header), separator] + cells.dropFirst().map(line)).joined(separator: "\n")
    }

    // MARK: 行内

    private func inlineChildren(_ element: XMLElement) -> String {
        (element.children ?? []).map(inline).joined()
    }

    private func inline(_ node: XMLNode) -> String {
        if node.kind == .text {
            // 文字里的 * 和 ` 会被当成格式，转义掉
            return (node.stringValue ?? "")
                .replacingOccurrences(of: "*", with: "\\*")
                .replacingOccurrences(of: "`", with: "\\`")
        }
        guard let element = node as? XMLElement else { return "" }
        let name = element.localName?.lowercased() ?? ""
        if Self.skipped.contains(name) || element.isDecoration { return "" }

        // 公式：只取 LaTeX 原文，跳过给人看的那份和备用图片。写在段落里的独立公式单独占一行
        if element.isMathContainer, let tex = element.tex {
            let lineBreak = String(Self.lineBreak)
            return element.isDisplayMath ? "\(lineBreak)$$\(tex)$$\(lineBreak)" : "$\(tex)$"
        }
        if element.hasClass("clipmd-tex"), let tex = element.stringValue { return "$\(tex)$" }

        switch name {
        case "br":
            return String(Self.lineBreak)
        case "strong", "b":
            return Self.wrap(inlineChildren(element), with: "**")
        case "em", "i":
            return Self.wrap(inlineChildren(element), with: "*")
        case "del", "s", "strike":
            return Self.wrap(inlineChildren(element), with: "~~")
        case "code", "kbd", "samp", "tt":
            return Self.inlineCode(element.stringValue ?? "")
        case "a":
            let text = inlineChildren(element)
            guard let href = element.attribute(forName: "href")?.stringValue, Self.isExternal(href),
                  !text.trimmingCharacters(in: .whitespaces).isEmpty else { return text }
            return Self.wrap(text) { "[\($0)](\(href))" }
        case "img":
            guard let src = element.attribute(forName: "src")?.stringValue, Self.isExternal(src) else { return "" }
            let alt = element.attribute(forName: "alt")?.stringValue ?? ""
            return "![\(alt)](\(src))"
        case "input":
            // 任务列表的勾选框
            guard element.attribute(forName: "type")?.stringValue?.lowercased() == "checkbox" else { return "" }
            return element.attribute(forName: "checked") != nil ? "[x] " : "[ ] "
        default:
            return inlineChildren(element)
        }
    }

    /// 合并空白（和浏览器显示时一样），再把 `<br>` 的占位符换成换行
    static func finishInline(_ text: String) -> String {
        let collapsed = text
            // 零宽字符（维基百科在公式两边放了 U+2060），看不见但会留在文件里
            .replacingOccurrences(of: "[\u{200B}\u{200C}\u{200D}\u{2060}\u{FEFF}]", with: "", options: .regularExpression)
            .replacingOccurrences(of: "[ \t\n\r\u{00A0}]+", with: " ", options: .regularExpression)
            .replacingOccurrences(of: " ?\(lineBreak) ?", with: "\n", options: .regularExpression)
        return collapsed.trimmingCharacters(in: .whitespaces)
            .trimmingCharacters(in: CharacterSet(charactersIn: "\n"))
    }

    /// 标记要紧贴文字：`** 重点 **` 不算粗体，所以把首尾空白挪到标记外面
    static func wrap(_ text: String, with marker: String) -> String {
        wrap(text) { "\(marker)\($0)\(marker)" }
    }

    static func wrap(_ text: String, _ build: (String) -> String) -> String {
        let core = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !core.isEmpty else { return text }
        let leading = text.prefix { $0.isWhitespace }
        let trailing = text.reversed().prefix { $0.isWhitespace }
        return String(leading) + build(core) + String(trailing.reversed())
    }

    static func inlineCode(_ raw: String) -> String {
        let code = raw.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespaces)
        guard !code.isEmpty else { return "" }
        let fence = String(repeating: "`", count: longestRun(of: "`", in: code) + 1)
        let pad = code.hasPrefix("`") || code.hasSuffix("`") ? " " : ""
        return fence + pad + code + pad + fence
    }

    static func isExternal(_ url: String) -> Bool {
        let lower = url.lowercased()
        return lower.hasPrefix("http://") || lower.hasPrefix("https://") || lower.hasPrefix("mailto:")
    }

    static func language(from element: XMLElement?) -> String? {
        guard let classes = element?.attribute(forName: "class")?.stringValue else { return nil }
        for name in classes.split(separator: " ") {
            for prefix in ["language-", "lang-"] where name.hasPrefix(prefix) {
                let language = String(name.dropFirst(prefix.count))
                if !language.isEmpty { return language }
            }
        }
        return nil
    }

    static func longestRun(of character: Character, in text: String) -> Int {
        var longest = 0, current = 0
        for c in text {
            current = c == character ? current + 1 : 0
            longest = max(longest, current)
        }
        return longest
    }
}

private extension XMLElement {
    func hasClass(_ name: String) -> Bool {
        guard let classes = attribute(forName: "class")?.stringValue else { return false }
        return classes.split(separator: " ").contains { $0 == name }
    }

    /// 页面装饰，不是正文：标题旁的 ¶ 和锚点图标、维基百科的 [编辑] 链接和 [1] 引用编号、读屏器以外隐藏的元素
    var isDecoration: Bool {
        if attribute(forName: "aria-hidden")?.stringValue == "true", !hasClass("katex-html") { return true }
        if hasClass("headerlink") || hasClass("mw-editsection") || hasClass("anchor") { return true }
        return localName?.lowercased() == "sup" && hasClass("reference")
    }

    /// KaTeX（`katex`、`katex-display`）和维基百科（`mwe-math-element`）的公式外壳
    var isMathContainer: Bool {
        hasClass("katex") || hasClass("katex-display") || hasClass("mwe-math-element")
    }

    var isDisplayMath: Bool {
        hasClass("katex-display") || hasClass("mwe-math-element-block")
    }

    /// 自己遍历子节点找后代。不用 XPath：整理过的 HTML 上，`.//code` 这类相对路径会找到别处的节点
    func descendants(where match: (XMLElement) -> Bool) -> [XMLElement] {
        var result: [XMLElement] = []
        for case let child as XMLElement in children ?? [] {
            if match(child) { result.append(child) }
            result += child.descendants(where: match)
        }
        return result
    }

    func firstDescendant(where match: (XMLElement) -> Bool) -> XMLElement? {
        for case let child as XMLElement in children ?? [] {
            if match(child) { return child }
            if let found = child.firstDescendant(where: match) { return found }
        }
        return nil
    }

    /// 公式里的 LaTeX 原文（见 `HTMLToMarkdown.preserveTeX`）
    var tex: String? {
        let node = firstDescendant { $0.hasClass("clipmd-tex") }
        guard var tex = node?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines), !tex.isEmpty else { return nil }
        // 维基百科会再包一层 {\displaystyle …} 或 {\textstyle …}
        for wrapper in ["{\\displaystyle", "{\\textstyle"] where tex.hasPrefix(wrapper) && tex.hasSuffix("}") {
            tex = String(tex.dropFirst(wrapper.count).dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        // 去掉了外壳后，开头可能还剩一个 \textstyle
        for command in ["\\displaystyle", "\\textstyle"] where tex.hasPrefix(command + " ") {
            tex = String(tex.dropFirst(command.count)).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return tex.isEmpty ? nil : tex
    }
}
