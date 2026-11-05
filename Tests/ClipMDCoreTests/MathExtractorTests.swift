import Foundation
import Testing
@testable import ClipMDCore

@Suite struct MathExtractorTests {
    private func latex(_ source: String) -> [String] { MathExtractor.extract(source).math.map(\.latex) }

    @Test func displayMathOnOneLine() {
        let (markdown, math) = MathExtractor.extract("前文\n\n$$ E = mc^2 $$\n\n后文")
        #expect(math == [MathSnippet(latex: "E = mc^2", display: true)])
        #expect(!markdown.contains("$"))
    }

    @Test func displayMathAcrossLines() {
        #expect(latex("$$\n\\frac{a}{b}\n+ c\n$$") == ["\\frac{a}{b}\n+ c"])
        #expect(latex("\\[\nx_1 + x_2\n\\]") == ["x_1 + x_2"])
    }

    @Test func inlineMath() {
        let (markdown, math) = MathExtractor.extract("面积是 $\\pi r^2$，周长是 \\(2\\pi r\\)。")
        #expect(math.map(\.latex) == ["\\pi r^2", "2\\pi r"])
        #expect(math.allSatisfy { !$0.display })
        #expect(markdown.hasPrefix("面积是 \u{E000}0\u{E001}"))
    }

    @Test func currencyIsNotMath() {
        #expect(latex("苹果 $5，香蕉 $10。").isEmpty)
        #expect(latex("价格是 $ 5 到 $ 10").isEmpty)
    }

    @Test func escapedDollarIsNotMath() {
        #expect(latex("显示 \\$x\\$ 本身").isEmpty)
    }

    @Test func codeIsLeftAlone() {
        #expect(latex("```bash\necho $HOME $PATH\n$$\n```").isEmpty)
        #expect(latex("用 `$x$` 表示").isEmpty)
    }

    @Test func underscoresInsideMathSurviveMarkdown() {
        let document = MarkdownParser.parse("变量 $a_1 * b_2 * c_3$ 很多")
        #expect(document.math.map(\.latex) == ["a_1 * b_2 * c_3"])
        #expect(document.blocks.first?.hasInlineMath == true)
    }

    @Test func displayMathBecomesItsOwnBlock() {
        let document = MarkdownParser.parse("# 标题\n\n损失函数：\n$$\nL = \\sum_i (y_i - \\hat y_i)^2\n$$\n继续")
        #expect(document.blocks.map(\.kind) == [.heading(level: 1), .paragraph, .math(index: 0), .paragraph])
        #expect(document.math.first?.latex == "L = \\sum_i (y_i - \\hat y_i)^2")
    }

    @Test func displayMathInsideListKeepsDepth() {
        let document = MarkdownParser.parse("- 第一项\n\n  $$x^2$$\n- 第二项")
        let mathBlock = document.blocks.first { if case .math = $0.kind { true } else { false } }
        #expect(mathBlock?.listDepth == 1)
    }

    @Test func documentWithoutMathIsUnchanged() {
        let source = "# 普通文档\n\n- 一\n- 二"
        #expect(MarkdownParser.parse(source).blocks == MarkdownParser.blocks(from: source))
        #expect(MarkdownParser.parse(source).math.isEmpty)
    }
}
