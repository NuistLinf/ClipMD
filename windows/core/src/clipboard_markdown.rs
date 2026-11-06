//! 剪贴板里同时有纯文本和网页格式（HTML）时，决定存哪一份。

use std::sync::LazyLock;

use regex_lite::Regex;

use crate::html_to_markdown;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// 用纯文本：本来就是 Markdown，或者没有网页格式
    Plain,
    /// 用网页格式转出来的 Markdown
    Html,
}

pub fn best(plain: &str, html: Option<&str>) -> (String, Source) {
    // AI 的复制按钮给的一般已经是 Markdown，直接用
    let Some(html) = html else { return (plain.to_string(), Source::Plain) };
    if looks_like_markdown(plain) {
        return (plain.to_string(), Source::Plain);
    }
    let Some(converted) = html_to_markdown::convert(html) else { return (plain.to_string(), Source::Plain) };
    // 网页本来就没什么格式，转出来除了空白都一样，保持原样
    if without_whitespace(&converted) == without_whitespace(plain) {
        return (plain.to_string(), Source::Plain);
    }
    // 转换后明显少了内容（比如文字在不认识的标签里被跳过了），宁可不转也不能丢字
    if content_length(&converted) < content_length(plain) * 7 / 10 {
        return (plain.to_string(), Source::Plain);
    }
    (converted, Source::Html)
}

/// 有这些明确的 Markdown 写法之一，就当它已经是 Markdown。
/// 不看 `# 标题`：网页里的 Python、Shell 代码有很多 `#` 开头的注释，复制成纯文本后会误判。
pub fn looks_like_markdown(text: &str) -> bool {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"(?m)^\s*(```|~~~)",       // 代码块
            r"\*\*[^*\n]+\*\*",         // 粗体
            r"\[[^\]\n]+\]\([^)\s]+\)", // 链接
            r"(?m)^\s*\|?\s*:?-{3,}",   // 表格分隔线
        ]
        .iter()
        .map(|p| Regex::new(p).expect("正则写错了"))
        .collect()
    });
    PATTERNS.iter().any(|p| p.is_match(text))
}

fn without_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 去掉所有空白和 Markdown 符号，只比较文字本身
fn content_length(text: &str) -> usize {
    text.chars().filter(|&c| !c.is_whitespace() && !"#*_`>-|[]()!~\\".contains(c)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_markdown_wins() {
        let plain = "## 标题\n\n- 一\n- 二";
        let (text, source) = best(plain, Some("<h2>标题</h2><ul><li>一</li><li>二</li></ul>"));
        // "## 标题" 里没有明确的 Markdown 写法，但转出来和原文只差空白 → 保持原样
        assert_eq!(source, Source::Plain);
        assert_eq!(text, plain);
    }

    #[test]
    fn web_selection_uses_html() {
        let plain = "核心思路\n\n第一项\n第二项";
        let (text, source) = best(plain, Some("<h2>核心思路</h2><ul><li>第一项</li><li>第二项</li></ul>"));
        assert_eq!(source, Source::Html);
        assert_eq!(text, "## 核心思路\n\n- 第一项\n- 第二项");
    }

    #[test]
    fn formatless_html_keeps_plain_text() {
        let plain = "第一行\n第二行";
        let (text, source) = best(plain, Some("<div>第一行</div><div>第二行</div>"));
        assert_eq!(source, Source::Plain);
        assert_eq!(text, plain);
    }

    #[test]
    fn lossy_conversion_falls_back_to_plain() {
        let plain = "很长的一段正文，内容很多很多很多很多很多很多";
        // 正文在 <textarea> 里会被跳过，转出来几乎没东西
        let (_, source) = best(plain, Some("<h1>标题</h1><textarea>很长的一段正文，内容很多很多很多很多很多很多</textarea>"));
        assert_eq!(source, Source::Plain);
    }

    #[test]
    fn code_comments_in_plain_text_do_not_look_like_markdown() {
        let plain = "if 语句\n\n# Create a sample collection\nusers = {}";
        let html = "<h2>if 语句</h2><pre><code class=\"language-python\"># Create a sample collection\nusers = {}</code></pre>";
        let (text, source) = best(plain, Some(html));
        assert_eq!(source, Source::Html);
        assert_eq!(text, "## if 语句\n\n```python\n# Create a sample collection\nusers = {}\n```");
    }

    #[test]
    fn no_html_uses_plain() {
        assert_eq!(best("文字", None).1, Source::Plain);
    }

    #[test]
    fn markdown_detection() {
        assert!(looks_like_markdown("看 **这里**"));
        assert!(looks_like_markdown("```\ncode\n```"));
        assert!(looks_like_markdown("[链接](https://a.b)"));
        assert!(looks_like_markdown("| a | b |\n|---|---|"));
        assert!(!looks_like_markdown("# 注释\nprint(1)"));
    }
}
