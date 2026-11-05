import Foundation
import Testing
@testable import ClipMDCore

@Suite struct HTMLToMarkdownTests {
    private func md(_ html: String) -> String { HTMLToMarkdown.convert(html) ?? "<nil>" }

    @Test func headingsAndParagraphs() {
        let html = "<meta charset='utf-8'><h2 style=\"font-size:20px\">一、核心思路</h2><p>这是<strong>重点</strong>&nbsp;内容。<p>没闭合的段落"
        #expect(md(html) == "## 一、核心思路\n\n这是**重点** 内容。\n\n没闭合的段落")
    }

    @Test func inlineFormatting() {
        let html = "<p>有 <em>斜体</em>、<del>删除线</del>、<code>inline_code</code> 和 <a href=\"https://example.com\">链接</a>。</p>"
        #expect(md(html) == "有 *斜体*、~~删除线~~、`inline_code` 和 [链接](https://example.com)。")
    }

    @Test func markersHugTheText() {
        #expect(md("<p>前<strong> 粗体 </strong>后</p>") == "前 **粗体** 后")
    }

    @Test func nestedLists() {
        let html = "<ul><li>第一项<ul><li>嵌套</li></ul></li><li>第二项</ul><ol start=\"3\"><li>三</li><li>四</li></ol>"
        #expect(md(html) == "- 第一项\n  - 嵌套\n- 第二项\n\n3. 三\n4. 四")
    }

    @Test func listItemWithParagraphs() {
        let html = "<ol><li><p>步骤一</p><p>补充说明</p></li><li><p>步骤二</p></li></ol>"
        #expect(md(html) == "1. 步骤一\n   补充说明\n2. 步骤二")
    }

    @Test func codeBlockKeepsIndentationAndLanguage() {
        let html = "<pre><code class=\"language-swift\">let a = 1\n  缩进保留\n</code></pre>"
        #expect(md(html) == "```swift\nlet a = 1\n  缩进保留\n```")
    }

    @Test func chatGPTStyleCodeBlockSkipsHeaderAndButton() {
        let html = """
        <pre><div><div>python</div><button>复制代码</button></div><div><code class="hljs language-python"><span class="hljs-keyword">print</span>(<span>"hi"</span>)</code></div></pre>
        """
        #expect(md(html) == "```python\nprint(\"hi\")\n```")
    }

    /// 回归：Python 文档的代码块里没有 <code>，后面段落里的行内代码不能被当成代码块内容
    @Test func severalCodeBlocksWithInlineCodeBetween() {
        let html = """
        <p>例如：</p><div class="highlight"><pre><span></span><span class="k">if</span> x:
            <span class="n">print</span>(1)</pre></div><p>可以有 <code>elif</code> 部分。</p>
        <div class="highlight"><pre><span class="k">for</span> w <span class="ow">in</span> words:</pre></div><p>还有 <code>range()</code>。</p>
        """
        #expect(md(html) == "例如：\n\n```\nif x:\n    print(1)\n```\n\n可以有 `elif` 部分。\n\n```\nfor w in words:\n```\n\n还有 `range()`。")
    }

    @Test func eachFormulaKeepsItsOwnTeX() {
        func katex(_ tex: String) -> String {
            "<span class=\"katex\"><span class=\"katex-mathml\"><math><semantics><annotation encoding=\"application/x-tex\">\(tex)</annotation></semantics></math></span><span class=\"katex-html\">x</span></span>"
        }
        #expect(md("<p>先 \(katex("a^2")) 再 \(katex("b_1")) 最后 \(katex("c"))</p>") == "先 $a^2$ 再 $b_1$ 最后 $c$")
    }

    @Test func codeContainingBackticksGetsLongerFence() {
        #expect(md("<pre><code>用 ``` 包住</code></pre>") == "````\n用 ``` 包住\n````")
    }

    @Test func blockquote() {
        #expect(md("<blockquote><p>第一段</p><p>第二段</p></blockquote>") == "> 第一段\n>\n> 第二段")
    }

    @Test func table() {
        let html = "<table><thead><tr><th>名称</th><th>说明</th></tr></thead><tbody><tr><td>a|b</td><td><strong>粗</strong></td></tr></tbody></table>"
        #expect(md(html) == "| 名称 | 说明 |\n| --- | --- |\n| a\\|b | **粗** |")
    }

    @Test func katexFormulas() {
        let inline = """
        <p>质能方程 <span class="katex"><span class="katex-mathml"><math><semantics><mrow><mi>E</mi></mrow><annotation encoding="application/x-tex">E=mc^2</annotation></semantics></math></span><span class="katex-html" aria-hidden="true">E=mc2</span></span> 很有名</p>
        """
        #expect(md(inline) == "质能方程 $E=mc^2$ 很有名")

        let display = """
        <span class="katex-display"><span class="katex"><span class="katex-mathml"><math display="block"><semantics><annotation encoding="application/x-tex">\\frac{a}{b} &lt; 1</annotation></semantics></math></span><span class="katex-html">ab</span></span></span>
        """
        #expect(md(display) == "$$\n\\frac{a}{b} < 1\n$$")
    }

    @Test func wikipediaFormulas() {
        let html = """
        <p>公式 <span class="mwe-math-element mwe-math-element-inline"><span class="mwe-math-mathml-a11y" style="display: none;"><math><semantics><mrow><mi>x</mi></mrow><annotation encoding="application/x-tex">{\\displaystyle x^{2}}</annotation></semantics></math></span><img src="https://wikimedia.org/x.svg" class="mwe-math-fallback-image-inline" alt="x^2"></span> 和<span class="mwe-math-element mwe-math-element-block"><span class="mwe-math-mathml-a11y"><math><semantics><annotation encoding="application/x-tex">{\\displaystyle x={\\frac {-b}{2a}}}</annotation></semantics></math></span><img src="https://wikimedia.org/y.svg"></span>结束</p>
        """
        let markdown = md(html)
        #expect(markdown == "公式 $x^{2}$ 和\n$$x={\\frac {-b}{2a}}$$\n结束")
        // 接到预览那边：独立公式要单独成块
        let document = MarkdownParser.parse(markdown)
        #expect(document.math.map(\.display) == [false, true])
        #expect(document.blocks.contains { $0.kind == .math(index: 1) })
    }

    @Test func zeroWidthCharactersAreRemoved() {
        #expect(md("<p>前\u{2060}后\u{200B}文</p>") == "前后文")
    }

    @Test func pageDecorationsAreDropped() {
        let html = """
        <h2>4.1. if 语句<a class="headerlink" href="#if">¶</a></h2>
        <h2><span class="mw-headline">推导</span><span class="mw-editsection">[<a href="/w/index.php?action=edit">编辑</a>]</span></h2>
        <p>正文<sup class="reference"><a href="#cite-1">[1]</a></sup>结束<span aria-hidden="true">🔗</span></p>
        """
        #expect(md(html) == "## 4.1. if 语句\n\n## 推导\n\n正文结束")
    }

    @Test func leftoverTextstyleIsStripped() {
        let html = "<span class=\"mwe-math-element mwe-math-element-inline\"><math><semantics><annotation encoding=\"application/x-tex\">{\\displaystyle \\textstyle ax^{2}}</annotation></semantics></math></span>"
        #expect(md(html) == "$ax^{2}$")
    }

    @Test func scriptsStylesAndButtonsAreDropped() {
        #expect(md("<style>p{}</style><script>alert(1)</script><p>正文</p><button>点我</button>") == "正文")
    }

    @Test func lineBreaksAndWhitespace() {
        #expect(md("<p>第一行<br>第二行</p><p>  多个   空格\n换行  </p>") == "第一行\n第二行\n\n多个 空格 换行")
    }

    @Test func literalAsterisksAreEscaped() {
        #expect(md("<p>a * b * c</p>") == "a \\* b \\* c")
    }

    @Test func relativeLinksAndDataImagesKeepOnlyText() {
        #expect(md("<p><a href=\"/docs\">文档</a> <img src=\"data:image/png;base64,AAA\"></p>") == "文档")
    }

    @Test func taskList() {
        #expect(md("<ul><li><input type=\"checkbox\" checked> 完成</li><li><input type=\"checkbox\"> 待办</li></ul>") == "- [x] 完成\n- [ ] 待办")
    }
}

@Suite struct ClipboardMarkdownTests {
    @Test func plainMarkdownWins() {
        let plain = "## 标题\n\n- 一\n- 二"
        let result = ClipboardMarkdown.best(plain: plain, html: "<h2>标题</h2><ul><li>一</li><li>二</li></ul>")
        #expect(result.source == .plain)
        #expect(result.text == plain)
    }

    @Test func webSelectionUsesHTML() {
        let plain = "核心思路\n\n第一项\n第二项"
        let result = ClipboardMarkdown.best(plain: plain, html: "<h2>核心思路</h2><ul><li>第一项</li><li>第二项</li></ul>")
        #expect(result.source == .html)
        #expect(result.text == "## 核心思路\n\n- 第一项\n- 第二项")
    }

    @Test func formatlessHTMLKeepsPlainText() {
        let plain = "第一行\n第二行"
        let result = ClipboardMarkdown.best(plain: plain, html: "<div>第一行</div><div>第二行</div>")
        #expect(result.source == .plain)
        #expect(result.text == plain)
    }

    @Test func lossyConversionFallsBackToPlain() {
        let plain = "很长的一段正文，内容很多很多很多很多很多很多"
        // 正文在 <textarea> 里会被跳过，转出来几乎没东西
        let result = ClipboardMarkdown.best(plain: plain, html: "<h1>标题</h1><textarea>很长的一段正文，内容很多很多很多很多很多很多</textarea>")
        #expect(result.source == .plain)
    }

    @Test func codeCommentsInPlainTextDoNotLookLikeMarkdown() {
        // 网页里的 Python 代码复制成纯文本后，# 注释在行首
        let plain = "if 语句\n\n# Create a sample collection\nusers = {}"
        let html = "<h2>if 语句</h2><pre><code class=\"language-python\"># Create a sample collection\nusers = {}</code></pre>"
        let result = ClipboardMarkdown.best(plain: plain, html: html)
        #expect(result.source == .html)
        #expect(result.text == "## if 语句\n\n```python\n# Create a sample collection\nusers = {}\n```")
    }

    @Test func noHTMLUsesPlain() {
        #expect(ClipboardMarkdown.best(plain: "文字", html: nil).source == .plain)
    }
}
