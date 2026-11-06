//! RTF 只取文字。macOS 版用系统的 NSAttributedString；Windows 版自己解析，
//! 处理 `\par`、`\'hh`（按 `\ansicpg` 代码页解码，中文 Word/写字板是 936）、`\uN`，跳过字体表、图片这些。

use crate::codepage;

/// 这些组里的内容不是正文
const SKIPPED_DESTINATIONS: &[&str] = &[
    "fonttbl", "colortbl", "stylesheet", "info", "pict", "object", "themedata", "colorschememapping", "latentstyles",
    "datastore", "xmlnstbl", "listtable", "listoverridetable", "rsidtbl", "generator", "mmathPr", "header", "footer",
    "headerl", "headerr", "footerl", "footerr", "footnote", "fldinst", "filetbl", "revtbl", "expandedcolortbl",
    "bkmkstart", "bkmkend", "nonshppict", "shpinst", "userprops", "xe", "tc", "pgdsctbl",
];

#[derive(Clone, Copy)]
struct Group {
    skip: bool,
    /// `\ucN`：每个 `\u` 后面跟着几个给老程序看的替代字符
    uc: usize,
}

pub fn rtf_text(data: &[u8]) -> Option<String> {
    if !data.starts_with(b"{\\rtf") {
        return None;
    }
    let mut out = String::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut codepage = 1252u32;
    let mut stack: Vec<Group> = Vec::new();
    let mut group = Group { skip: false, uc: 1 };
    // \u 之后还要跳过几个替代字符
    let mut skip_chars = 0usize;
    // 刚进组，下一个控制字决定这个组是不是要跳过的目标
    let mut group_start = false;
    let mut i = 0;

    let flush = |pending: &mut Vec<u8>, out: &mut String, codepage: u32| {
        if !pending.is_empty() {
            match codepage::decode(codepage, pending) {
                Some(text) => out.push_str(&text),
                None => out.push_str(&String::from_utf8_lossy(pending)),
            }
            pending.clear();
        }
    };

    while i < data.len() {
        let c = data[i];
        match c {
            b'{' => {
                stack.push(group);
                group_start = true;
                i += 1;
            }
            b'}' => {
                group = stack.pop().unwrap_or(group);
                group_start = false;
                skip_chars = 0;
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&next) = data.get(i) else { break };
                if next.is_ascii_alphabetic() {
                    let start = i;
                    while i < data.len() && data[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = std::str::from_utf8(&data[start..i]).unwrap_or("");
                    let num_start = i;
                    if i < data.len() && (data[i] == b'-' || data[i].is_ascii_digit()) {
                        i += 1;
                        while i < data.len() && data[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let param: Option<i32> = std::str::from_utf8(&data[num_start..i]).ok().and_then(|s| s.parse().ok());
                    // 控制字后面的一个空格是分隔符
                    if i < data.len() && data[i] == b' ' {
                        i += 1;
                    }
                    let was_start = std::mem::replace(&mut group_start, false);
                    if was_start && SKIPPED_DESTINATIONS.contains(&word) {
                        group.skip = true;
                    }
                    if group.skip {
                        continue;
                    }
                    if skip_chars > 0 {
                        skip_chars -= 1;
                        continue;
                    }
                    match word {
                        "ansicpg" => codepage = param.unwrap_or(1252) as u32,
                        "uc" => group.uc = param.unwrap_or(1).max(0) as usize,
                        "u" => {
                            flush(&mut pending, &mut out, codepage);
                            let mut code = param.unwrap_or(0);
                            if code < 0 {
                                code += 65536;
                            }
                            push_utf16_unit(&mut out, code as u32);
                            skip_chars = group.uc;
                        }
                        "par" | "line" | "sect" | "page" | "row" => {
                            flush(&mut pending, &mut out, codepage);
                            out.push('\n');
                        }
                        "tab" | "cell" => {
                            flush(&mut pending, &mut out, codepage);
                            out.push('\t');
                        }
                        "emdash" => out.push('—'),
                        "endash" => out.push('–'),
                        "bullet" => out.push('•'),
                        "lquote" => out.push('‘'),
                        "rquote" => out.push('’'),
                        "ldblquote" => out.push('“'),
                        "rdblquote" => out.push('”'),
                        _ => {}
                    }
                } else {
                    i += 1;
                    match next {
                        b'*' => {
                            // {\* … } 是可选目标，不认识就整组跳过
                            group.skip = true;
                        }
                        b'\'' => {
                            let hex = data.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                            i += 2;
                            group_start = false;
                            if group.skip {
                                continue;
                            }
                            if skip_chars > 0 {
                                skip_chars -= 1;
                                continue;
                            }
                            if let Some(byte) = hex {
                                pending.push(byte);
                            }
                        }
                        b'\\' | b'{' | b'}' => {
                            group_start = false;
                            if !group.skip {
                                if skip_chars > 0 {
                                    skip_chars -= 1;
                                } else {
                                    flush(&mut pending, &mut out, codepage);
                                    out.push(next as char);
                                }
                            }
                        }
                        b'~' => {
                            if !group.skip {
                                flush(&mut pending, &mut out, codepage);
                                out.push('\u{00A0}');
                            }
                        }
                        b'_' => {
                            if !group.skip {
                                flush(&mut pending, &mut out, codepage);
                                out.push('-');
                            }
                        }
                        b'\n' | b'\r' => {
                            if !group.skip {
                                flush(&mut pending, &mut out, codepage);
                                out.push('\n');
                            }
                        }
                        _ => {}
                    }
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                group_start = false;
                i += 1;
                if group.skip {
                    continue;
                }
                if skip_chars > 0 {
                    skip_chars -= 1;
                    continue;
                }
                pending.push(c);
            }
        }
    }
    flush(&mut pending, &mut out, codepage);
    let text = merge_surrogates(&out);
    if text.chars().any(|c| !c.is_whitespace()) { Some(text) } else { None }
}

/// `\u` 给的是 UTF-16 码元，代理对先用私有标记存着，最后合并
fn push_utf16_unit(out: &mut String, unit: u32) {
    match char::from_u32(unit) {
        Some(c) => out.push(c),
        // 代理项不能直接放进 String，先编码成 U+F0000 区的占位字符
        None => out.push(char::from_u32(0xF0000 + unit).unwrap_or('\u{FFFD}')),
    }
}

fn merge_surrogates(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut high: Option<u32> = None;
    for c in text.chars() {
        let code = c as u32;
        if (0xF0000 + 0xD800..0xF0000 + 0xDC00).contains(&code) {
            high = Some(code - 0xF0000);
            continue;
        }
        if (0xF0000 + 0xDC00..0xF0000 + 0xE000).contains(&code) {
            if let Some(h) = high.take() {
                let combined = 0x10000 + ((h - 0xD800) << 10) + (code - 0xF0000 - 0xDC00);
                out.push(char::from_u32(combined).unwrap_or('\u{FFFD}'));
            }
            continue;
        }
        high = None;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_rtf() {
        let rtf = br"{\rtf1\ansi\ansicpg1252{\fonttbl\f0\fswiss Helvetica;}{\colortbl;\red255\green255\blue255;}\f0\fs24 Hello\par World}";
        assert_eq!(rtf_text(rtf).as_deref(), Some("Hello\nWorld"));
    }

    #[test]
    fn chinese_code_page_and_unicode() {
        // 中文写字板：\'hh 是 GBK 字节；macOS 的文本编辑：\uN
        let rtf = br"{\rtf1\ansi\ansicpg936{\fonttbl{\f0\fnil\fcharset134 SimSun;}}{\*\generator Riched20;}\f0\'b4\'f8\uc1\u26684?\'ca\'bd\'b5\'c4\uc0\u23383 \par}";
        assert_eq!(rtf_text(rtf).map(|t| t.trim().to_string()).as_deref(), Some("带格式的字"));
    }

    #[test]
    fn escapes_and_emoji() {
        let rtf = br"{\rtf1\ansi a\{b\}c\\d \u-10179?\u-8704? e}";
        assert_eq!(rtf_text(rtf).as_deref(), Some("a{b}c\\d 😀 e"));
    }

    #[test]
    fn not_rtf() {
        assert!(rtf_text(b"plain text").is_none());
        assert!(rtf_text(br"{\rtf1{\fonttbl x;}}").is_none());
    }
}
