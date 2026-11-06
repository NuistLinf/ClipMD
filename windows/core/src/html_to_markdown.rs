//! 把网页复制出来的 HTML 转成 Markdown：标题、列表、代码块、表格、引用、粗体斜体、链接、图片，
//! 以及网页上的公式（KaTeX、维基百科的 MathML，取出里面的 LaTeX 原文）。
//!
//! macOS 版用 Foundation 的 XMLDocument 整理 HTML；这里用 html5ever（和浏览器同一套 HTML5 解析规则），
//! 没闭合的标签、`<pre>` 里套 `<div>`、MathML 都能按浏览器的方式处理，所以不需要 macOS 版那两个预处理。

use std::cell::Ref;

use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::text::{is_hspace, longest_run};

/// 太大的 HTML 不转（一般是整页复制，内联样式很多），直接用纯文本
const MAX_INPUT_LENGTH: usize = 2_000_000;

/// `<br>` 先用这个字符占位，合并空白之后再换成换行
const LINE_BREAK: char = '\u{E010}';

/// 不输出任何内容的标签
const SKIPPED: &[&str] = &[
    "script", "style", "head", "title", "meta", "link", "noscript", "button", "svg", "iframe", "form", "select",
    "textarea", "template", "object", "embed", "canvas", "audio", "video",
];

/// 块级标签：前后要断开
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "pre", "blockquote", "table", "hr", "section",
    "article", "main", "header", "footer", "aside", "nav", "figure", "figcaption", "dl", "dt", "dd", "center",
    "address", "details", "summary", "body", "html",
];

pub fn convert(html: &str) -> Option<String> {
    if html.len() > MAX_INPUT_LENGTH * 4 || html.chars().count() > MAX_INPUT_LENGTH {
        return None;
    }
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(html);
    let body = find_body(&dom.document).unwrap_or_else(|| dom.document.clone());
    let markdown = blocks(&children(&body), false);
    let cleaned = collapse_blank_lines(&markdown);
    let cleaned = cleaned.trim();
    if cleaned.is_empty() { None } else { Some(cleaned.to_string()) }
}

fn find_body(node: &Handle) -> Option<Handle> {
    for child in node.children.borrow().iter() {
        if tag(child).as_deref() == Some("body") {
            return Some(child.clone());
        }
        if let Some(found) = find_body(child) {
            return Some(found);
        }
    }
    None
}

/// 连续三个以上的换行合并成两个
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for c in text.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    out
}

// MARK: 节点工具

fn children(node: &Handle) -> Vec<Handle> {
    node.children.borrow().clone()
}

/// 元素的小写标签名；不是元素返回 None
fn tag(node: &Handle) -> Option<String> {
    match &node.data {
        NodeData::Element { name, .. } => Some(str::to_ascii_lowercase(&name.local)),
        _ => None,
    }
}

fn attr(node: &Handle, key: &str) -> Option<String> {
    match &node.data {
        NodeData::Element { attrs, .. } => {
            let attrs: Ref<_> = attrs.borrow();
            attrs.iter().find(|a| str::eq_ignore_ascii_case(&a.name.local, key)).map(|a| a.value.to_string())
        }
        _ => None,
    }
}

fn has_class(node: &Handle, class: &str) -> bool {
    attr(node, "class").is_some_and(|classes| classes.split_whitespace().any(|c| c == class))
}

/// 所有文字后代拼起来（相当于 XMLNode.stringValue）
fn text_content(node: &Handle) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out
}

fn collect_text(node: &Handle, out: &mut String) {
    match &node.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        _ => {
            for child in node.children.borrow().iter() {
                collect_text(child, out);
            }
        }
    }
}

fn descendants(node: &Handle, matches: &dyn Fn(&Handle) -> bool, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if matches(child) {
            out.push(child.clone());
        }
        descendants(child, matches, out);
    }
}

fn first_descendant(node: &Handle, matches: &dyn Fn(&Handle) -> bool) -> Option<Handle> {
    for child in node.children.borrow().iter() {
        if matches(child) {
            return Some(child.clone());
        }
        if let Some(found) = first_descendant(child, matches) {
            return Some(found);
        }
    }
    None
}

/// 页面装饰，不是正文：标题旁的 ¶ 和锚点图标、维基百科的 [编辑] 链接和 [1] 引用编号、读屏器以外隐藏的元素
fn is_decoration(node: &Handle) -> bool {
    if attr(node, "aria-hidden").as_deref() == Some("true") && !has_class(node, "katex-html") {
        return true;
    }
    if has_class(node, "headerlink") || has_class(node, "mw-editsection") || has_class(node, "anchor") {
        return true;
    }
    tag(node).as_deref() == Some("sup") && has_class(node, "reference")
}

/// KaTeX（`katex`、`katex-display`）和维基百科（`mwe-math-element`）的公式外壳
fn is_math_container(node: &Handle) -> bool {
    has_class(node, "katex") || has_class(node, "katex-display") || has_class(node, "mwe-math-element")
}

fn is_display_math(node: &Handle) -> bool {
    has_class(node, "katex-display") || has_class(node, "mwe-math-element-block")
}

/// 公式里的 LaTeX 原文：MathML 的 `<annotation encoding="application/x-tex">`
fn tex(node: &Handle) -> Option<String> {
    let annotation = first_descendant(node, &|n| {
        tag(n).as_deref() == Some("annotation")
            && attr(n, "encoding").is_some_and(|e| e.eq_ignore_ascii_case("application/x-tex"))
    })?;
    let mut tex = text_content(&annotation).trim().to_string();
    if tex.is_empty() {
        return None;
    }
    // 维基百科会再包一层 {\displaystyle …} 或 {\textstyle …}
    for wrapper in ["{\\displaystyle", "{\\textstyle"] {
        if tex.starts_with(wrapper) && tex.ends_with('}') {
            tex = tex[wrapper.len()..tex.len() - 1].trim().to_string();
        }
    }
    // 去掉了外壳后，开头可能还剩一个 \textstyle
    for command in ["\\displaystyle", "\\textstyle"] {
        if tex.starts_with(&format!("{command} ")) {
            tex = tex[command.len()..].trim().to_string();
        }
    }
    if tex.is_empty() { None } else { Some(tex) }
}

// MARK: 块

/// 一组节点渲染成块级 Markdown。`tight` 为 true 时块之间只换一行（用在列表项里）
fn blocks(nodes: &[Handle], tight: bool) -> String {
    let mut output: Vec<String> = Vec::new();
    let mut inline_text = String::new();
    let flush = |inline_text: &mut String, output: &mut Vec<String>| {
        let text = finish_inline(inline_text);
        if !text.is_empty() {
            output.push(text);
        }
        inline_text.clear();
    };
    for node in nodes {
        if is_block(node) {
            flush(&mut inline_text, &mut output);
            if let Some(block) = block(node).filter(|b| !b.is_empty()) {
                output.push(block);
            }
        } else {
            inline_text.push_str(&inline(node));
        }
    }
    flush(&mut inline_text, &mut output);
    output.join(if tight { "\n" } else { "\n\n" })
}

fn is_block(node: &Handle) -> bool {
    let Some(name) = tag(node) else { return false };
    BLOCK_TAGS.contains(&name.as_str()) || SKIPPED.contains(&name.as_str()) || is_display_math(node)
}

fn block(node: &Handle) -> Option<String> {
    let name = tag(node)?;
    if SKIPPED.contains(&name.as_str()) || is_decoration(node) {
        return None;
    }
    if is_math_container(node) {
        if let Some(tex) = tex(node) {
            return Some(if is_display_math(node) { format!("$$\n{tex}\n$$") } else { format!("${tex}$") });
        }
    }

    match name.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name[1..].parse::<usize>().unwrap_or(1);
            let text = finish_inline(&inline_children(node)).replace('\n', " ");
            if text.is_empty() { None } else { Some(format!("{} {}", "#".repeat(level), text)) }
        }
        "p" | "dt" | "dd" | "figcaption" | "summary" | "address" => Some(finish_inline(&inline_children(node))),
        "ul" | "ol" => list(node, name == "ol"),
        "li" => list_item(node, "-"),
        "pre" => code_block(node),
        "blockquote" => {
            let inner = blocks(&children(node), false);
            if inner.is_empty() {
                return None;
            }
            Some(inner.split('\n').map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") }).collect::<Vec<_>>().join("\n"))
        }
        "table" => table(node),
        "hr" => Some("---".into()),
        _ => Some(blocks(&children(node), false)),
    }
}

fn list(node: &Handle, ordered: bool) -> Option<String> {
    let mut number: i64 = attr(node, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1);
    let mut items: Vec<String> = Vec::new();
    for child in children(node) {
        let Some(name) = tag(&child) else { continue };
        if name == "li" {
            let marker = if ordered { format!("{number}.") } else { "-".to_string() };
            number += 1;
            if let Some(text) = list_item(&child, &marker) {
                items.push(text);
            }
        } else if name == "ul" || name == "ol" {
            // 不规范的嵌套：列表直接套列表
            if let Some(nested) = list(&child, name == "ol") {
                items.push(nested.split('\n').map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n"));
            }
        }
    }
    if items.is_empty() { None } else { Some(items.join("\n")) }
}

/// 第一行带列表符号，后面的行缩进对齐
fn list_item(item: &Handle, marker: &str) -> Option<String> {
    let content = blocks(&children(item), true);
    let indent = " ".repeat(marker.chars().count() + 1);
    let mut lines = content.split('\n');
    let first = lines.next()?;
    let mut out = vec![format!("{marker} {first}")];
    out.extend(lines.map(|l| if l.is_empty() { String::new() } else { format!("{indent}{l}") }));
    Some(out.join("\n"))
}

fn code_block(pre: &Handle) -> Option<String> {
    // ChatGPT 之类的网页会在 <pre> 里放语言名、"复制"按钮，真正的代码在 <code> 里
    let code = first_descendant(pre, &|n| tag(n).as_deref() == Some("code"));
    let mut text = text_content(code.as_ref().unwrap_or(pre)).replace('\u{00A0}', " ");
    while text.starts_with('\n') {
        text.remove(0);
    }
    while text.ends_with('\n') || text.ends_with(' ') {
        text.pop();
    }
    if text.is_empty() {
        return None;
    }
    let language = language(code.as_ref()).or_else(|| language(Some(pre))).unwrap_or_default();
    let fence = "`".repeat((longest_run('`', &text) + 1).max(3));
    Some(format!("{fence}{language}\n{text}\n{fence}"))
}

fn table(node: &Handle) -> Option<String> {
    let mut rows = Vec::new();
    descendants(node, &|n| tag(n).as_deref() == Some("tr"), &mut rows);
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            children(row)
                .iter()
                .filter(|c| matches!(tag(c).as_deref(), Some("td" | "th")))
                .map(|c| finish_inline(&inline_children(c)).replace('\n', " ").replace('|', "\\|"))
                .collect::<Vec<_>>()
        })
        .filter(|r| !r.is_empty())
        .collect();
    let header = cells.first()?;
    let width = cells.iter().map(Vec::len).max().unwrap_or(header.len());
    let line = |row: &Vec<String>| {
        let mut padded = row.clone();
        padded.resize(width, String::new());
        format!("| {} |", padded.join(" | "))
    };
    let separator = format!("|{}", " --- |".repeat(width));
    let mut out = vec![line(header), separator];
    out.extend(cells.iter().skip(1).map(line));
    Some(out.join("\n"))
}

// MARK: 行内

fn inline_children(node: &Handle) -> String {
    node.children.borrow().iter().map(inline).collect()
}

fn inline(node: &Handle) -> String {
    match &node.data {
        // 文字里的 * 和 ` 会被当成格式，转义掉
        NodeData::Text { contents } => return contents.borrow().replace('*', "\\*").replace('`', "\\`"),
        NodeData::Element { .. } => {}
        _ => return String::new(),
    }
    let name = tag(node).unwrap_or_default();
    if SKIPPED.contains(&name.as_str()) || is_decoration(node) {
        return String::new();
    }

    // 公式：只取 LaTeX 原文，跳过给人看的那份和备用图片。写在段落里的独立公式单独占一行
    if is_math_container(node) {
        if let Some(tex) = tex(node) {
            return if is_display_math(node) { format!("{LINE_BREAK}$${tex}$${LINE_BREAK}") } else { format!("${tex}$") };
        }
    }

    match name.as_str() {
        "br" => LINE_BREAK.to_string(),
        "strong" | "b" => wrap_marker(&inline_children(node), "**"),
        "em" | "i" => wrap_marker(&inline_children(node), "*"),
        "del" | "s" | "strike" => wrap_marker(&inline_children(node), "~~"),
        "code" | "kbd" | "samp" | "tt" => inline_code(&text_content(node)),
        "a" => {
            let text = inline_children(node);
            match attr(node, "href") {
                Some(href) if is_external(&href) && !text.trim_matches(is_hspace).is_empty() => {
                    wrap(&text, |core| format!("[{core}]({href})"))
                }
                _ => text,
            }
        }
        "img" => match attr(node, "src") {
            Some(src) if is_external(&src) => format!("![{}]({src})", attr(node, "alt").unwrap_or_default()),
            _ => String::new(),
        },
        "input" => {
            // 任务列表的勾选框
            if attr(node, "type").is_some_and(|t| t.eq_ignore_ascii_case("checkbox")) {
                if attr(node, "checked").is_some() { "[x] ".into() } else { "[ ] ".into() }
            } else {
                String::new()
            }
        }
        _ => inline_children(node),
    }
}

/// 合并空白（和浏览器显示时一样），再把 `<br>` 的占位符换成换行
fn finish_inline(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        match c {
            // 零宽字符（维基百科在公式两边放了 U+2060），看不见但会留在文件里
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}
            ' ' | '\t' | '\n' | '\r' | '\u{00A0}' => {
                if !in_space {
                    collapsed.push(' ');
                    in_space = true;
                }
            }
            _ => {
                collapsed.push(c);
                in_space = false;
            }
        }
    }
    // " ?<br> ?" → "\n"
    let mut out = String::with_capacity(collapsed.len());
    let chars: Vec<char> = collapsed.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' && chars.get(i + 1) == Some(&LINE_BREAK) {
            i += 1;
            continue;
        }
        if c == LINE_BREAK {
            out.push('\n');
            if chars.get(i + 1) == Some(&' ') {
                i += 1;
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out.trim_matches(is_hspace).trim_matches('\n').to_string()
}

/// 标记要紧贴文字：`** 重点 **` 不算粗体，所以把首尾空白挪到标记外面
fn wrap_marker(text: &str, marker: &str) -> String {
    wrap(text, |core| format!("{marker}{core}{marker}"))
}

fn wrap(text: &str, build: impl Fn(&str) -> String) -> String {
    let core = text.trim();
    if core.is_empty() {
        return text.to_string();
    }
    let leading = &text[..text.len() - text.trim_start().len()];
    let trailing = &text[text.trim_end().len()..];
    format!("{leading}{}{trailing}", build(core))
}

fn inline_code(raw: &str) -> String {
    let code = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if code.is_empty() {
        return String::new();
    }
    let fence = "`".repeat(longest_run('`', &code) + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') { " " } else { "" };
    format!("{fence}{pad}{code}{pad}{fence}")
}

fn is_external(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

fn language(node: Option<&Handle>) -> Option<String> {
    let classes = attr(node?, "class")?;
    for name in classes.split_whitespace() {
        for prefix in ["language-", "lang-"] {
            if let Some(language) = name.strip_prefix(prefix) {
                if !language.is_empty() {
                    return Some(language.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{BlockKind, parse};

    fn md(html: &str) -> String {
        convert(html).unwrap_or_else(|| "<nil>".into())
    }

    #[test]
    fn headings_and_paragraphs() {
        let html = "<meta charset='utf-8'><h2 style=\"font-size:20px\">一、核心思路</h2><p>这是<strong>重点</strong>&nbsp;内容。<p>没闭合的段落";
        assert_eq!(md(html), "## 一、核心思路\n\n这是**重点** 内容。\n\n没闭合的段落");
    }

    #[test]
    fn inline_formatting() {
        let html = "<p>有 <em>斜体</em>、<del>删除线</del>、<code>inline_code</code> 和 <a href=\"https://example.com\">链接</a>。</p>";
        assert_eq!(md(html), "有 *斜体*、~~删除线~~、`inline_code` 和 [链接](https://example.com)。");
    }

    #[test]
    fn markers_hug_the_text() {
        assert_eq!(md("<p>前<strong> 粗体 </strong>后</p>"), "前 **粗体** 后");
    }

    #[test]
    fn nested_lists() {
        let html = "<ul><li>第一项<ul><li>嵌套</li></ul></li><li>第二项</ul><ol start=\"3\"><li>三</li><li>四</li></ol>";
        assert_eq!(md(html), "- 第一项\n  - 嵌套\n- 第二项\n\n3. 三\n4. 四");
    }

    #[test]
    fn list_item_with_paragraphs() {
        let html = "<ol><li><p>步骤一</p><p>补充说明</p></li><li><p>步骤二</p></li></ol>";
        assert_eq!(md(html), "1. 步骤一\n   补充说明\n2. 步骤二");
    }

    #[test]
    fn code_block_keeps_indentation_and_language() {
        let html = "<pre><code class=\"language-swift\">let a = 1\n  缩进保留\n</code></pre>";
        assert_eq!(md(html), "```swift\nlet a = 1\n  缩进保留\n```");
    }

    #[test]
    fn chatgpt_style_code_block_skips_header_and_button() {
        let html = r#"<pre><div><div>python</div><button>复制代码</button></div><div><code class="hljs language-python"><span class="hljs-keyword">print</span>(<span>"hi"</span>)</code></div></pre>"#;
        assert_eq!(md(html), "```python\nprint(\"hi\")\n```");
    }

    /// 回归：Python 文档的代码块里没有 <code>，后面段落里的行内代码不能被当成代码块内容
    #[test]
    fn several_code_blocks_with_inline_code_between() {
        let html = "<p>例如：</p><div class=\"highlight\"><pre><span></span><span class=\"k\">if</span> x:\n    <span class=\"n\">print</span>(1)</pre></div><p>可以有 <code>elif</code> 部分。</p>\n<div class=\"highlight\"><pre><span class=\"k\">for</span> w <span class=\"ow\">in</span> words:</pre></div><p>还有 <code>range()</code>。</p>";
        assert_eq!(
            md(html),
            "例如：\n\n```\nif x:\n    print(1)\n```\n\n可以有 `elif` 部分。\n\n```\nfor w in words:\n```\n\n还有 `range()`。"
        );
    }

    #[test]
    fn each_formula_keeps_its_own_tex() {
        let katex = |tex: &str| {
            format!("<span class=\"katex\"><span class=\"katex-mathml\"><math><semantics><annotation encoding=\"application/x-tex\">{tex}</annotation></semantics></math></span><span class=\"katex-html\">x</span></span>")
        };
        let html = format!("<p>先 {} 再 {} 最后 {}</p>", katex("a^2"), katex("b_1"), katex("c"));
        assert_eq!(md(&html), "先 $a^2$ 再 $b_1$ 最后 $c$");
    }

    #[test]
    fn code_containing_backticks_gets_longer_fence() {
        assert_eq!(md("<pre><code>用 ``` 包住</code></pre>"), "````\n用 ``` 包住\n````");
    }

    #[test]
    fn blockquote() {
        assert_eq!(md("<blockquote><p>第一段</p><p>第二段</p></blockquote>"), "> 第一段\n>\n> 第二段");
    }

    #[test]
    fn table() {
        let html = "<table><thead><tr><th>名称</th><th>说明</th></tr></thead><tbody><tr><td>a|b</td><td><strong>粗</strong></td></tr></tbody></table>";
        assert_eq!(md(html), "| 名称 | 说明 |\n| --- | --- |\n| a\\|b | **粗** |");
    }

    #[test]
    fn katex_formulas() {
        let inline = r#"<p>质能方程 <span class="katex"><span class="katex-mathml"><math><semantics><mrow><mi>E</mi></mrow><annotation encoding="application/x-tex">E=mc^2</annotation></semantics></math></span><span class="katex-html" aria-hidden="true">E=mc2</span></span> 很有名</p>"#;
        assert_eq!(md(inline), "质能方程 $E=mc^2$ 很有名");

        let display = r#"<span class="katex-display"><span class="katex"><span class="katex-mathml"><math display="block"><semantics><annotation encoding="application/x-tex">\frac{a}{b} &lt; 1</annotation></semantics></math></span><span class="katex-html">ab</span></span></span>"#;
        assert_eq!(md(display), "$$\n\\frac{a}{b} < 1\n$$");
    }

    #[test]
    fn wikipedia_formulas() {
        let html = r#"<p>公式 <span class="mwe-math-element mwe-math-element-inline"><span class="mwe-math-mathml-a11y" style="display: none;"><math><semantics><mrow><mi>x</mi></mrow><annotation encoding="application/x-tex">{\displaystyle x^{2}}</annotation></semantics></math></span><img src="https://wikimedia.org/x.svg" class="mwe-math-fallback-image-inline" alt="x^2"></span> 和<span class="mwe-math-element mwe-math-element-block"><span class="mwe-math-mathml-a11y"><math><semantics><annotation encoding="application/x-tex">{\displaystyle x={\frac {-b}{2a}}}</annotation></semantics></math></span><img src="https://wikimedia.org/y.svg"></span>结束</p>"#;
        let markdown = md(html);
        assert_eq!(markdown, "公式 $x^{2}$ 和\n$$x={\\frac {-b}{2a}}$$\n结束");
        // 接到预览那边：独立公式要单独成块
        let document = parse(&markdown);
        assert_eq!(document.math.iter().map(|m| m.display).collect::<Vec<_>>(), [false, true]);
        assert!(document.blocks.iter().any(|b| b.kind == BlockKind::Math(1)));
    }

    #[test]
    fn zero_width_characters_are_removed() {
        assert_eq!(md("<p>前\u{2060}后\u{200B}文</p>"), "前后文");
    }

    #[test]
    fn page_decorations_are_dropped() {
        let html = "<h2>4.1. if 语句<a class=\"headerlink\" href=\"#if\">¶</a></h2>\n<h2><span class=\"mw-headline\">推导</span><span class=\"mw-editsection\">[<a href=\"/w/index.php?action=edit\">编辑</a>]</span></h2>\n<p>正文<sup class=\"reference\"><a href=\"#cite-1\">[1]</a></sup>结束<span aria-hidden=\"true\">🔗</span></p>";
        assert_eq!(md(html), "## 4.1. if 语句\n\n## 推导\n\n正文结束");
    }

    #[test]
    fn leftover_textstyle_is_stripped() {
        let html = r#"<span class="mwe-math-element mwe-math-element-inline"><math><semantics><annotation encoding="application/x-tex">{\displaystyle \textstyle ax^{2}}</annotation></semantics></math></span>"#;
        assert_eq!(md(html), "$ax^{2}$");
    }

    #[test]
    fn scripts_styles_and_buttons_are_dropped() {
        assert_eq!(md("<style>p{}</style><script>alert(1)</script><p>正文</p><button>点我</button>"), "正文");
    }

    #[test]
    fn line_breaks_and_whitespace() {
        assert_eq!(md("<p>第一行<br>第二行</p><p>  多个   空格\n换行  </p>"), "第一行\n第二行\n\n多个 空格 换行");
    }

    #[test]
    fn literal_asterisks_are_escaped() {
        assert_eq!(md("<p>a * b * c</p>"), "a \\* b \\* c");
    }

    #[test]
    fn relative_links_and_data_images_keep_only_text() {
        assert_eq!(md("<p><a href=\"/docs\">文档</a> <img src=\"data:image/png;base64,AAA\"></p>"), "文档");
    }

    #[test]
    fn task_list() {
        assert_eq!(
            md("<ul><li><input type=\"checkbox\" checked> 完成</li><li><input type=\"checkbox\"> 待办</li></ul>"),
            "- [x] 完成\n- [ ] 待办"
        );
    }

    /// Windows 剪贴板的 "HTML Format" 带 StartFragment 注释和外层 html/body
    #[test]
    fn windows_clipboard_fragment() {
        let html = "<html><body>\r\n<!--StartFragment--><h3>标题</h3><p>正文</p><!--EndFragment-->\r\n</body>\r\n</html>";
        assert_eq!(md(html), "### 标题\n\n正文");
    }
}
