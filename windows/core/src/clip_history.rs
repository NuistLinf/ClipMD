//! 剪贴板记录的规则：去重、上限、存取。不碰剪贴板，方便测试。

use serde::{Deserialize, Serialize};

use crate::text::{is_hspace, prefix_chars};

pub const MAX_ITEMS: usize = 50;
/// 单条太大的不记（一般是整篇文档），网页格式太大的只记文字
pub const MAX_TEXT_LENGTH: usize = 100_000;
pub const MAX_HTML_LENGTH: usize = 300_000;

/// 剪贴板记录里的一条：复制过的文字或图片，还没存成文件。
/// JSON 字段名和 macOS 版一样，两边的记录文件可以通用。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipItem {
    pub id: String,
    pub text: String,
    /// 网页格式（从网页复制时有），存成 md 时用来转换
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(with = "crate::time::iso8601_serde")]
    pub date: i64,
    /// 从哪个应用复制的
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_app: Option<String>,
    #[serde(default)]
    pub character_count: usize,
    /// 列表里显示的第一行
    #[serde(default)]
    pub preview: String,
    /// 已经存成 md 的文件名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_file_name: Option<String>,
    /// 图片：存在 images 文件夹里的文件名（文字记录为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_height: Option<u32>,
    /// 图片内容的指纹，用来去重
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_hash: Option<String>,
}

impl ClipItem {
    pub fn new_text(text: String, html: Option<String>, date: i64, source_app: Option<String>) -> Self {
        let character_count = text.chars().count();
        let preview = preview_of(&text);
        ClipItem {
            id: new_id(),
            text,
            html,
            date,
            source_app,
            character_count,
            preview,
            saved_file_name: None,
            image_file: None,
            image_width: None,
            image_height: None,
            image_hash: None,
        }
    }

    pub fn new_image(image_file: String, width: u32, height: u32, hash: String, date: i64, source_app: Option<String>) -> Self {
        ClipItem {
            id: new_id(),
            text: String::new(),
            html: None,
            date,
            source_app,
            character_count: 0,
            preview: format!("图片 {width} × {height}"),
            saved_file_name: None,
            image_file: Some(image_file),
            image_width: Some(width),
            image_height: Some(height),
            image_hash: Some(hash),
        }
    }

    pub fn is_image(&self) -> bool {
        self.image_file.is_some()
    }

    /// 列表和预览里显示的标题：第一行去掉 Markdown 标题的 # 号
    pub fn title(&self) -> String {
        stripping_heading(&self.preview)
    }

    /// 去重用：文字比内容，图片比指纹
    fn content_key(&self) -> String {
        match &self.image_hash {
            Some(hash) => format!("image:{hash}"),
            None => format!("text:{}", self.text),
        }
    }
}

/// 空白、太长的返回 None
pub fn make_item(text: &str, html: Option<&str>, source_app: Option<&str>, date: i64) -> Option<ClipItem> {
    if !text.chars().any(|c| !c.is_whitespace()) || text.chars().count() > MAX_TEXT_LENGTH {
        return None;
    }
    let html = html.filter(|h| h.chars().count() <= MAX_HTML_LENGTH).map(str::to_string);
    Some(ClipItem::new_text(text.to_string(), html, date, source_app.map(str::to_string)))
}

/// 新的放最前面；同样的文字（或同一张图片）再复制一次只是挪到最前面，不重复记
pub fn adding(mut item: ClipItem, items: &[ClipItem]) -> Vec<ClipItem> {
    let key = item.content_key();
    // 之前存过的话，记得它已经存成了哪个文件
    if item.saved_file_name.is_none() {
        item.saved_file_name = items.iter().find(|i| i.content_key() == key).and_then(|i| i.saved_file_name.clone());
    }
    let mut list = Vec::with_capacity(items.len() + 1);
    list.push(item);
    list.extend(items.iter().filter(|i| i.content_key() != key).cloned());
    list.truncate(MAX_ITEMS);
    list
}

/// 还被记录用着的图片文件；其余的可以删掉
pub fn referenced_images(items: &[ClipItem]) -> std::collections::HashSet<String> {
    items.iter().filter_map(|i| i.image_file.clone()).collect()
}

pub fn stripping_heading(line: &str) -> String {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return line.to_string();
    }
    let rest = &line[hashes..];
    if !rest.starts_with(char::is_whitespace) {
        return line.to_string();
    }
    let stripped = rest.trim_matches(is_hspace);
    if stripped.is_empty() { line.to_string() } else { stripped.to_string() }
}

pub fn preview_of(text: &str) -> String {
    let head = prefix_chars(text, 400);
    let line = head
        .split(|c: char| crate::text::is_newline(c))
        .map(|l| l.trim_matches(is_hspace))
        .find(|l| !l.is_empty())
        .unwrap_or("");
    prefix_chars(line, 80).to_string()
}

pub fn encode(items: &[ClipItem]) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(items)
}

/// 文件坏了就当没有记录，不影响启动
pub fn decode(data: &[u8]) -> Vec<ClipItem> {
    serde_json::from_slice(data).unwrap_or_default()
}

/// 随机的 UUID（v4 格式）。不需要密码学强度，只要不重复
pub fn new_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let state = RandomState::new();
    let half = || {
        let mut h = state.build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.finish()
    };
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&half().to_le_bytes());
    bytes[8..].copy_from_slice(&half().to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str) -> ClipItem {
        make_item(text, None, Some("微信"), 0).unwrap()
    }

    fn texts(list: &[ClipItem]) -> Vec<&str> {
        list.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn newest_first() {
        let mut list = vec![];
        list = adding(item("一"), &list);
        list = adding(item("二"), &list);
        assert_eq!(texts(&list), ["二", "一"]);
    }

    #[test]
    fn copying_again_moves_to_top() {
        let mut list = vec![];
        for text in ["一", "二", "三", "一"] {
            list = adding(item(text), &list);
        }
        assert_eq!(texts(&list), ["一", "三", "二"]);
    }

    #[test]
    fn saved_state_survives_recopy() {
        let mut first = item("要存的");
        first.saved_file_name = Some("2026-10-24 要存的.md".into());
        let list = adding(item("要存的"), &[first]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].saved_file_name.as_deref(), Some("2026-10-24 要存的.md"));
    }

    #[test]
    fn keeps_at_most_fifty_items() {
        let mut list = vec![];
        for index in 0..60 {
            list = adding(item(&format!("第 {index} 条")), &list);
        }
        assert_eq!(list.len(), MAX_ITEMS);
        assert_eq!(list[0].text, "第 59 条");
    }

    #[test]
    fn blank_and_huge_texts_are_not_recorded() {
        assert!(make_item("  \n ", None, None, 0).is_none());
        assert!(make_item(&"字".repeat(MAX_TEXT_LENGTH + 1), None, None, 0).is_none());
    }

    #[test]
    fn huge_html_is_dropped_but_text_kept() {
        let huge = "<p>x</p>".repeat(MAX_HTML_LENGTH);
        let recorded = make_item("正文", Some(&huge), None, 0).unwrap();
        assert_eq!(recorded.text, "正文");
        assert!(recorded.html.is_none());
    }

    #[test]
    fn title_drops_markdown_heading_marks() {
        assert_eq!(item("## 训练结果总结\n正文").title(), "训练结果总结");
        assert_eq!(item("#hashtag 不是标题").title(), "#hashtag 不是标题");
        assert_eq!(item("###   ").title(), "###");
    }

    #[test]
    fn preview_is_first_non_empty_line() {
        assert_eq!(item("\n\n   第一行内容  \n第二行").preview, "第一行内容");
    }

    #[test]
    fn round_trips_through_json() {
        let list = vec![make_item("带格式的", Some("<b>带格式的</b>"), Some("微信"), 1_793_000_000).unwrap(), item("纯文字")];
        let decoded = decode(&encode(&list).unwrap());
        assert_eq!(texts(&decoded), texts(&list));
        assert_eq!(decoded[0].html.as_deref(), Some("<b>带格式的</b>"));
        assert_eq!(decoded[0].source_app.as_deref(), Some("微信"));
        assert_eq!(decoded[0].date, 1_793_000_000);
    }

    #[test]
    fn images_dedupe_by_hash_not_by_empty_text() {
        let a = ClipItem::new_image("a.png".into(), 100, 50, "aaa".into(), 0, Some("截图".into()));
        let b = ClipItem::new_image("b.png".into(), 200, 80, "bbb".into(), 0, Some("截图".into()));
        let again = ClipItem::new_image("a2.png".into(), 100, 50, "aaa".into(), 0, Some("截图".into()));
        let mut list = adding(a, &[]);
        list = adding(item("文字"), &list);
        list = adding(b, &list);
        assert_eq!(list.len(), 3);
        list = adding(again, &list);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].image_file.as_deref(), Some("a2.png"));
        assert_eq!(list[0].preview, "图片 100 × 50");
        let expected: std::collections::HashSet<String> = ["a2.png".to_string(), "b.png".to_string()].into();
        assert_eq!(referenced_images(&list), expected);
    }

    #[test]
    fn old_history_files_without_image_fields_still_load() {
        let old = format!(
            r#"[{{"id":"{}","text":"旧记录","date":"2026-10-24T08:00:00Z","characterCount":3,"preview":"旧记录"}}]"#,
            new_id()
        );
        let decoded = decode(old.as_bytes());
        assert_eq!(decoded[0].text, "旧记录");
        assert!(!decoded[0].is_image());
    }

    #[test]
    fn broken_file_means_empty_history() {
        assert!(decode("不是 JSON".as_bytes()).is_empty());
    }

    #[test]
    fn ids_are_unique_uuids() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
    }
}
