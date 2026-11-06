//! 文字小工具：和 Swift 的 `CharacterSet.whitespaces` 等行为对齐。

/// 横向空白（空格、制表符、不换行空格……），不含换行。对应 Swift 的 `.whitespaces`
pub fn is_hspace(c: char) -> bool {
    c.is_whitespace() && !is_newline(c)
}

pub fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// 去掉首尾的横向空白，保留换行
pub fn trim_h(s: &str) -> &str {
    s.trim_matches(is_hspace)
}

/// 字符数（按 Unicode 标量算，和 Swift 的字符数在绝大多数文字上一样）
pub fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// 前 n 个字符
pub fn prefix_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((index, _)) => &s[..index],
        None => s,
    }
}

/// 最长的连续 `c` 有几个
pub fn longest_run(c: char, text: &str) -> usize {
    let (mut longest, mut current) = (0, 0);
    for ch in text.chars() {
        current = if ch == c { current + 1 } else { 0 };
        longest = longest.max(current);
    }
    longest
}

/// 千分位："12,345"
pub fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 简单的 64 位哈希（FNV-1a），用来给剪贴板内容做指纹，不用于安全场景
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(trim_h(" \t文字\n "), "文字\n");
        assert_eq!(prefix_chars("中文abc", 3), "中文a");
        assert_eq!(prefix_chars("短", 9), "短");
        assert_eq!(longest_run('`', "a ``` b `` c"), 3);
        assert_eq!(group_thousands(1234567), "1,234,567");
        assert_eq!(group_thousands(12), "12");
    }
}
