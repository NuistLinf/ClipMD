import Foundation

/// 预览用的一个块：标题、段落、列表项、代码块、分隔线或表格。
public struct MarkdownBlock: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case paragraph
        case heading(level: Int)
        case codeBlock(language: String?)
        case thematicBreak
        case table
        /// 独立成段的公式，序号对应 `MarkdownDocument.math`
        case math(index: Int)
    }

    public let id: Int
    public var kind: Kind
    /// 段落、标题的内容，保留粗体、行内代码、链接等行内格式
    public var text: AttributedString
    /// 列表嵌套层数，0 表示不在列表里
    public var listDepth = 0
    /// 列表项第一段前面的符号（"•"、"1."）；同一项的后续段落为 nil
    public var listMarker: String?
    public var quoteDepth = 0
    /// 表格的单元格，第一行是表头
    public var tableRows: [[AttributedString]] = []
    /// 文字里有行内公式的占位符，排版时要换成公式图片
    public var hasInlineMath = false

    public var code: String {
        var s = String(text.characters)
        if s.hasSuffix("\n") { s.removeLast() }
        return s
    }
}

/// 解析结果：块，加上文档里所有的公式
public struct MarkdownDocument: Equatable, Sendable {
    public var blocks: [MarkdownBlock]
    public var math: [MathSnippet]

    public init(blocks: [MarkdownBlock], math: [MathSnippet]) {
        self.blocks = blocks
        self.math = math
    }
}

/// 用 Foundation 自带的 Markdown 解析器，把块结构拆出来交给 SwiftUI 排版。
public enum MarkdownParser {
    /// 先把公式换成占位符再解析，见 `MathExtractor`
    public static func parse(_ source: String) -> MarkdownDocument {
        let (markdown, math) = MathExtractor.extract(source)
        var blocks = blocks(from: markdown)
        guard !math.isEmpty else { return MarkdownDocument(blocks: blocks, math: []) }

        for index in blocks.indices {
            if blocks[index].kind == .paragraph,
               let mathIndex = blockMathIndex(blocks[index].text), math.indices.contains(mathIndex) {
                blocks[index].kind = .math(index: mathIndex)
                continue
            }
            let texts = blocks[index].kind == .table ? blocks[index].tableRows.flatMap { $0 } : [blocks[index].text]
            blocks[index].hasInlineMath = texts.contains { $0.characters.contains(MathExtractor.inlineOpen) }
        }
        return MarkdownDocument(blocks: blocks, math: math)
    }

    /// 整段只有一个 `\u{E002}序号\u{E003}`
    private static func blockMathIndex(_ text: AttributedString) -> Int? {
        let plain = String(text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
        guard plain.first == MathExtractor.blockOpen, plain.last == MathExtractor.blockClose else { return nil }
        return Int(plain.dropFirst().dropLast())
    }

    /// 只解析 Markdown，不处理公式
    public static func blocks(from source: String) -> [MarkdownBlock] {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: false,
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        guard let document = try? AttributedString(markdown: source, options: options) else {
            return [MarkdownBlock(id: 0, kind: .paragraph, text: AttributedString(source))]
        }

        var blocks: [MarkdownBlock] = []
        var lastListItem: Int?
        var tableBlockIndex: [Int: Int] = [:]

        for (intent, range) in document.runs[\.presentationIntent] {
            let content = AttributedString(document[range])
            guard let intent else {
                blocks.append(MarkdownBlock(id: blocks.count, kind: .paragraph, text: content))
                continue
            }

            var kind = MarkdownBlock.Kind.paragraph
            var listDepth = 0, quoteDepth = 0
            var innermostItem: (ordinal: Int, identity: Int)?
            var innermostListOrdered: Bool?
            var table: Int?
            var row = 0, column = 0

            // components 从最内层排到最外层
            for component in intent.components {
                switch component.kind {
                case .header(let level): kind = .heading(level: level)
                case .codeBlock(let language): kind = .codeBlock(language: language)
                case .thematicBreak: kind = .thematicBreak
                case .blockQuote: quoteDepth += 1
                case .orderedList:
                    listDepth += 1
                    if innermostListOrdered == nil { innermostListOrdered = true }
                case .unorderedList:
                    listDepth += 1
                    if innermostListOrdered == nil { innermostListOrdered = false }
                case .listItem(let ordinal):
                    if innermostItem == nil { innermostItem = (ordinal, component.identity) }
                case .table: table = component.identity
                case .tableHeaderRow: row = 0
                case .tableRow(let index): row = index
                case .tableCell(let index): column = index
                default: break
                }
            }

            if let table {
                let index: Int
                if let existing = tableBlockIndex[table] {
                    index = existing
                } else {
                    index = blocks.count
                    tableBlockIndex[table] = index
                    blocks.append(MarkdownBlock(id: index, kind: .table, text: AttributedString(),
                                                quoteDepth: quoteDepth))
                }
                while blocks[index].tableRows.count <= row { blocks[index].tableRows.append([]) }
                while blocks[index].tableRows[row].count <= column { blocks[index].tableRows[row].append(AttributedString()) }
                blocks[index].tableRows[row][column] = content
                continue
            }

            var marker: String?
            if let item = innermostItem, item.identity != lastListItem {
                lastListItem = item.identity
                marker = innermostListOrdered == true ? "\(item.ordinal)." : bullet(forDepth: listDepth)
            }

            blocks.append(MarkdownBlock(id: blocks.count, kind: kind, text: content,
                                        listDepth: listDepth, listMarker: marker, quoteDepth: quoteDepth))
        }
        return blocks
    }

    static func bullet(forDepth depth: Int) -> String {
        switch depth {
        case ...1: "•"
        case 2: "◦"
        default: "▪"
        }
    }
}
