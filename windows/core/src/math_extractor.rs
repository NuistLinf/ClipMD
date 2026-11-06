//! Markdown 解析器不认识公式，而且会把公式里的 `_`、`*` 当成斜体。
//! 所以在解析前把公式换成占位符，排版时再换回渲染好的公式。
//!
//! 占位符用 Unicode 私有区字符包住序号，Markdown 不会动它们：
//! - 行内：`\u{E000}序号\u{E001}`
//! - 独立成段：`\u{E002}序号\u{E003}`，单独一段

use crate::text::{is_hspace, trim_h};

pub const INLINE_OPEN: char = '\u{E000}';
pub const INLINE_CLOSE: char = '\u{E001}';
pub const BLOCK_OPEN: char = '\u{E002}';
pub const BLOCK_CLOSE: char = '\u{E003}';

/// 一段 LaTeX 公式
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MathSnippet {
    pub latex: String,
    /// `$$…$$` / `\[…\]` 是独立成行的大号公式，`$…$` / `\(…\)` 是行内公式
    pub display: bool,
}

pub fn extract(source: &str) -> (String, Vec<MathSnippet>) {
    // 没有 $ 和 \ 就不可能有公式，大多数文档直接跳过
    if !(source.contains('$') || source.contains("\\[") || source.contains("\\(")) {
        return (source.to_string(), Vec::new());
    }

    let mut math = Vec::new();
    let mut output: Vec<String> = Vec::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let mut fence: Option<&str> = None;
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = trim_h(line);

        // 代码块里的内容原样保留
        if let Some(open) = fence {
            output.push(line.to_string());
            if trimmed.starts_with(open) {
                fence = None;
            }
            index += 1;
            continue;
        }
        if let Some(marker) = fence_marker(trimmed) {
            fence = Some(marker);
            output.push(line.to_string());
            index += 1;
            continue;
        }

        let (prefix, body) = split_prefix(line);
        if let Some((latex, consumed)) = display_math(index, &lines, body) {
            let token = format!("{BLOCK_OPEN}{}{BLOCK_CLOSE}", math.len());
            math.push(MathSnippet { latex, display: true });
            // 前后空一行，保证它单独成段；保留列表缩进和引用符号
            let blank = trim_h(prefix).to_string();
            output.push(blank.clone());
            output.push(format!("{prefix}{token}"));
            output.push(blank);
            index += consumed;
            continue;
        }

        output.push(replace_inline(line, &mut math));
        index += 1;
    }
    (output.join("\n"), math)
}

// MARK: 独立成段的公式

/// 以 `$$` 或 `\[` 开头的行。返回公式内容和用掉的行数。
fn display_math(start: usize, lines: &[&str], body: &str) -> Option<(String, usize)> {
    let body = trim_h(body);
    let (open, close) = [("$$", "$$"), ("\\[", "\\]")].into_iter().find(|(open, _)| body.starts_with(open))?;

    let after_open = &body[open.len()..];
    // 同一行就结束：$$ x^2 $$
    if let Some(end) = after_open.find(close) {
        let rest = trim_h(&after_open[end + close.len()..]);
        // 后面还有别的文字，说明是写在句子里的，交给行内处理
        if !rest.is_empty() {
            return None;
        }
        let latex = after_open[..end].trim();
        return if latex.is_empty() { None } else { Some((latex.to_string(), 1)) };
    }

    // 跨多行，往下找结束符；最多看 200 行，找不到就当普通文字
    let mut parts = vec![after_open.to_string()];
    let mut index = start + 1;
    while index < lines.len() && index - start <= 200 {
        let (_, line_body) = split_prefix(lines[index]);
        if let Some(end) = line_body.find(close) {
            let rest = trim_h(&line_body[end + close.len()..]);
            if !rest.is_empty() {
                return None;
            }
            parts.push(line_body[..end].to_string());
            let latex = parts.join("\n");
            let latex = latex.trim();
            return if latex.is_empty() { None } else { Some((latex.to_string(), index - start + 1)) };
        }
        parts.push(line_body.to_string());
        index += 1;
    }
    None
}

/// 把行首的缩进和引用符号（`> `）分出来
fn split_prefix(line: &str) -> (&str, &str) {
    let end = line.find(|c: char| !(c == ' ' || c == '\t' || c == '>')).unwrap_or(line.len());
    (&line[..end], &line[end..])
}

fn fence_marker(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

// MARK: 行内公式

fn replace_inline(line: &str, math: &mut Vec<MathSnippet>) -> String {
    if !(line.contains('$') || line.contains("\\(")) {
        return line.to_string();
    }
    let chars: Vec<char> = line.chars().collect();
    let mut result = String::with_capacity(line.len());
    let mut i = 0;

    let token = |latex: String, display: bool, math: &mut Vec<MathSnippet>| {
        let t = format!("{INLINE_OPEN}{}{INLINE_CLOSE}", math.len());
        math.push(MathSnippet { latex, display });
        t
    };
    let slice = |from: usize, to: usize| chars[from..to].iter().collect::<String>();

    while i < chars.len() {
        let c = chars[i];

        // 行内代码原样保留
        if c == '`' {
            let mut run = 0;
            while i + run < chars.len() && chars[i + run] == '`' {
                run += 1;
            }
            if let Some(end) = find_backtick_run(&chars, i + run, run) {
                result.push_str(&slice(i, end + run));
                i = end + run;
            } else {
                result.push_str(&slice(i, i + run));
                i += run;
            }
            continue;
        }

        // \$ 是转义，\( … \) 是行内公式
        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == '(' {
                if let Some(end) = find(&chars, &['\\', ')'], i + 2) {
                    let latex = slice(i + 2, end).trim_matches(is_hspace).to_string();
                    if !latex.is_empty() {
                        result.push_str(&token(latex, false, math));
                        i = end + 2;
                        continue;
                    }
                }
            }
            result.push(c);
            result.push(next);
            i += 2;
            continue;
        }

        if c == '$' {
            // 句子中间的 $$ … $$，按大号公式画在行内
            if i + 1 < chars.len() && chars[i + 1] == '$' {
                if let Some(end) = find(&chars, &['$', '$'], i + 2) {
                    let latex = slice(i + 2, end).trim_matches(is_hspace).to_string();
                    if !latex.is_empty() {
                        result.push_str(&token(latex, true, math));
                        i = end + 2;
                        continue;
                    }
                }
                result.push_str("$$");
                i += 2;
                continue;
            }
            if let Some(end) = closing_dollar(&chars, i) {
                result.push_str(&token(slice(i + 1, end), false, math));
                i = end + 1;
                continue;
            }
        }

        result.push(c);
        i += 1;
    }
    result
}

/// pandoc 的规则：开头的 $ 后面不能是空白；结尾的 $ 前面不能是空白，后面不能紧跟数字。
/// 这样 "$5 和 $10" 不会被当成公式。
fn closing_dollar(chars: &[char], open: usize) -> Option<usize> {
    let start = open + 1;
    if start >= chars.len() || chars[start].is_whitespace() || chars[start] == '$' {
        return None;
    }
    let mut j = start;
    while j < chars.len() {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j] == '$' {
            let before = chars[j - 1];
            let after = chars.get(j + 1).copied().unwrap_or(' ');
            if !before.is_whitespace() && !after.is_numeric() && j > start {
                return Some(j);
            }
            return None;
        }
        j += 1;
    }
    None
}

fn find(chars: &[char], needle: &[char], start: usize) -> Option<usize> {
    if chars.len() < needle.len() || start > chars.len() - needle.len() {
        return None;
    }
    (start..=chars.len() - needle.len()).find(|&j| &chars[j..j + needle.len()] == needle)
}

fn find_backtick_run(chars: &[char], start: usize, length: usize) -> Option<usize> {
    let mut j = start;
    while j < chars.len() {
        if chars[j] == '`' {
            let mut run = 0;
            while j + run < chars.len() && chars[j + run] == '`' {
                run += 1;
            }
            if run == length {
                return Some(j);
            }
            j += run;
        } else {
            j += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{BlockKind, blocks_from, parse};

    fn latex(source: &str) -> Vec<String> {
        extract(source).1.into_iter().map(|m| m.latex).collect()
    }

    #[test]
    fn display_math_on_one_line() {
        let (markdown, math) = extract("前文\n\n$$ E = mc^2 $$\n\n后文");
        assert_eq!(math, [MathSnippet { latex: "E = mc^2".into(), display: true }]);
        assert!(!markdown.contains('$'));
    }

    #[test]
    fn display_math_across_lines() {
        assert_eq!(latex("$$\n\\frac{a}{b}\n+ c\n$$"), ["\\frac{a}{b}\n+ c"]);
        assert_eq!(latex("\\[\nx_1 + x_2\n\\]"), ["x_1 + x_2"]);
    }

    #[test]
    fn inline_math() {
        let (markdown, math) = extract("面积是 $\\pi r^2$，周长是 \\(2\\pi r\\)。");
        assert_eq!(math.iter().map(|m| m.latex.as_str()).collect::<Vec<_>>(), ["\\pi r^2", "2\\pi r"]);
        assert!(math.iter().all(|m| !m.display));
        assert!(markdown.starts_with("面积是 \u{E000}0\u{E001}"));
    }

    #[test]
    fn currency_is_not_math() {
        assert!(latex("苹果 $5，香蕉 $10。").is_empty());
        assert!(latex("价格是 $ 5 到 $ 10").is_empty());
    }

    #[test]
    fn escaped_dollar_is_not_math() {
        assert!(latex("显示 \\$x\\$ 本身").is_empty());
    }

    #[test]
    fn code_is_left_alone() {
        assert!(latex("```bash\necho $HOME $PATH\n$$\n```").is_empty());
        assert!(latex("用 `$x$` 表示").is_empty());
    }

    #[test]
    fn underscores_inside_math_survive_markdown() {
        let document = parse("变量 $a_1 * b_2 * c_3$ 很多");
        assert_eq!(document.math.iter().map(|m| m.latex.as_str()).collect::<Vec<_>>(), ["a_1 * b_2 * c_3"]);
        assert!(document.blocks[0].has_inline_math);
    }

    #[test]
    fn display_math_becomes_its_own_block() {
        let document = parse("# 标题\n\n损失函数：\n$$\nL = \\sum_i (y_i - \\hat y_i)^2\n$$\n继续");
        let kinds: Vec<_> = document.blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(kinds, [BlockKind::Heading(1), BlockKind::Paragraph, BlockKind::Math(0), BlockKind::Paragraph]);
        assert_eq!(document.math[0].latex, "L = \\sum_i (y_i - \\hat y_i)^2");
    }

    #[test]
    fn display_math_inside_list_keeps_depth() {
        let document = parse("- 第一项\n\n  $$x^2$$\n- 第二项");
        let block = document.blocks.iter().find(|b| matches!(b.kind, BlockKind::Math(_))).unwrap();
        assert_eq!(block.list_depth, 1);
    }

    #[test]
    fn document_without_math_is_unchanged() {
        let source = "# 普通文档\n\n- 一\n- 二";
        assert_eq!(parse(source).blocks, blocks_from(source));
        assert!(parse(source).math.is_empty());
    }
}
