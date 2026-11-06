//! 根据内容给 md 文件起名：优先第一个 Markdown 标题，没有就用第一行非空文本。

use crate::text::{is_hspace, prefix_chars};
use crate::time::LocalDate;

pub const FALLBACK_TITLE: &str = "未命名";

/// 只看开头这么多字符，避免长文本整篇扫描
const SCAN_LIMIT: usize = 4000;

pub fn title(text: &str) -> String {
    title_with_max(text, 30)
}

pub fn title_with_max(text: &str, max_length: usize) -> String {
    let head = prefix_chars(text, SCAN_LIMIT);
    let mut first_line: Option<&str> = None;
    let mut heading: Option<&str> = None;
    let mut in_fence = false;

    for raw in head.split(['\n', '\r', '\u{2028}', '\u{2029}']) {
        let line = raw.trim_matches(is_hspace);
        if line.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        // 代码块里的 `# 注释` 不算标题
        if line.starts_with('#') && !in_fence {
            heading = Some(line);
            break;
        } else if first_line.is_none() {
            first_line = Some(line);
        }
    }

    let raw = heading.or(first_line).unwrap_or("");
    let cleaned = sanitize(&strip_markdown(raw));
    if cleaned.is_empty() {
        return FALLBACK_TITLE.into();
    }
    truncate(&cleaned, max_length)
}

/// `2026-10-24 标题.md`
pub fn file_name(title: &str, date: LocalDate, suffix: Option<u32>) -> String {
    match suffix {
        Some(n) => format!("{} {} {}.md", date.iso(), title, n),
        None => format!("{} {}.md", date.iso(), title),
    }
}

fn strip_markdown(line: &str) -> String {
    // 标题、引用、列表前缀
    let mut s = line.trim_start_matches(['#', '>']).trim_matches(is_hspace).to_string();
    if let Some(len) = list_marker_len(&s) {
        s.drain(..len);
    }
    // 强调和行内代码标记
    for mark in ["**", "__", "`", "*", "~~"] {
        s = s.replace(mark, "");
    }
    // [文字](链接) 只留文字
    replace_links(&s)
}

/// `- `、`* `、`1. `、`2) ` 这类列表符号（含后面的空白）的长度
fn list_marker_len(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    match bytes.first()? {
        b'-' | b'*' | b'+' => i = 1,
        b'0'..=b'9' => {
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') {
                i += 1;
            } else {
                return None;
            }
        }
        _ => return None,
    }
    let rest = &s[i..];
    let spaces = rest.len() - rest.trim_start().len();
    if spaces == 0 { None } else { Some(i + spaces) }
}

fn replace_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let parsed = after.find(']').and_then(|close| {
            let text = &after[..close];
            let tail = &after[close + 1..];
            let inner = tail.strip_prefix('(')?;
            let end = inner.find(')')?;
            Some((text, &inner[end + 1..]))
        });
        match parsed {
            Some((text, remaining)) => {
                out.push_str(&rest[..open]);
                out.push_str(text);
                rest = remaining;
            }
            None => {
                out.push_str(&rest[..open + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 去掉文件名里不能用的字符，空白合并成一个空格
fn sanitize(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| if "/\\:?*\"<>|".contains(c) || c.is_control() || crate::text::is_newline(c) { ' ' } else { c })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    // 文件名以点开头会被隐藏，Windows 上结尾的点和空格也会被系统去掉
    collapsed.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string()
}

/// 英文在单词中间截断不好看（"completing the s"），往回退到最近的空格；中文直接截
fn truncate(s: &str, max_length: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_length {
        return s.to_string();
    }
    let cut = &chars[..max_length];
    let next = chars[max_length];
    let word = |c: char| c.is_ascii_alphanumeric();
    if let Some(&last) = cut.last() {
        if word(last) && word(next) {
            if let Some(space) = cut.iter().rposition(|&c| c == ' ') {
                if space >= max_length / 2 {
                    return cut[..space].iter().collect::<String>().trim_matches(is_hspace).to_string();
                }
            }
        }
    }
    cut.iter().collect::<String>().trim_matches(is_hspace).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_first_heading() {
        let text = "好的，下面是方案。\n\n## 如何设计剪贴板工具\n\n正文";
        assert_eq!(title(text), "如何设计剪贴板工具");
    }

    #[test]
    fn falls_back_to_first_line() {
        assert_eq!(title("\n\n  **核心思路**：先触发再保存\n第二行"), "核心思路：先触发再保存");
    }

    #[test]
    fn skips_code_fence_line() {
        assert_eq!(title("```swift\nlet a = 1\n```"), "let a = 1");
    }

    #[test]
    fn ignores_comments_inside_code_blocks() {
        let text = "示例代码：\n```python\n# 这是注释\nprint(1)\n```\n## 真正的标题";
        assert_eq!(title(text), "真正的标题");
    }

    #[test]
    fn strips_list_markers_and_links() {
        assert_eq!(title("- 看 [官方文档](https://example.com) 的说明"), "看 官方文档 的说明");
    }

    #[test]
    fn removes_illegal_file_name_characters() {
        assert_eq!(title("# A/B: 对比?"), "A B 对比");
    }

    #[test]
    fn truncates_long_titles() {
        assert_eq!(title(&"长".repeat(80)).chars().count(), 30);
    }

    #[test]
    fn english_titles_break_at_words() {
        assert_eq!(title("## Derivation by completing the square"), "Derivation by completing the");
        assert_eq!(title(&"中".repeat(40)).chars().count(), 30);
    }

    #[test]
    fn empty_text_gets_fallback() {
        assert_eq!(title("   \n\n"), FALLBACK_TITLE);
        assert_eq!(title("..."), FALLBACK_TITLE);
    }

    #[test]
    fn file_name_has_date_prefix() {
        let date = LocalDate { year: 2026, month: 10, day: 24 };
        assert_eq!(file_name("标题", date, None), "2026-10-24 标题.md");
        assert_eq!(file_name("标题", date, Some(2)), "2026-10-24 标题 2.md");
    }

    #[test]
    fn windows_line_endings() {
        assert_eq!(title("\r\n# 标题\r\n正文"), "标题");
    }
}
