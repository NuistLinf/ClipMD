//! 拖进悬浮窗的文本文件（.md、.txt、.html、.rtf、代码文件……）读成文字。

use std::path::Path;

use crate::{codepage, rtf};

/// 太大的文件不读
const MAX_FILE_SIZE: u64 = 5_000_000;

const TEXT_EXTENSIONS: &[&str] = &[
    "md", "markdown", "mdown", "mkd", "txt", "text", "html", "htm", "rtf", "log", "csv", "tsv", "json", "jsonl", "yaml",
    "yml", "toml", "xml", "ini", "cfg", "conf", "tex", "py", "js", "mjs", "ts", "jsx", "tsx", "c", "h", "cc", "cpp",
    "hpp", "cs", "java", "kt", "swift", "go", "rs", "rb", "php", "sh", "bash", "zsh", "bat", "cmd", "ps1", "sql",
    "lua", "r", "m", "mm", "scala", "dart", "vue", "svelte", "css", "scss", "less", "gradle", "properties",
];

/// 能当文字读的文件：Markdown、纯文本（含代码文件）、网页、RTF
pub fn is_text_file(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|ext| TEXT_EXTENSIONS.contains(&ext.as_str()))
}

/// 读出来的纯文本和网页格式（网页文件只有 html，交给 `clipboard_markdown` 去转）
pub fn load(path: &Path) -> Option<(String, Option<String>)> {
    if !is_text_file(path) {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_SIZE {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => decode(&data).map(|html| (String::new(), Some(html))),
        "rtf" => rtf::rtf_text(&data).map(|text| (text, None)),
        _ => decode(&data).map(|text| (text, None)),
    }
}

/// 先按 UTF-8 读；不行再试中文 Windows 上常见的 GB18030（兼容 GBK、GB2312），最后试 UTF-16
pub fn decode(data: &[u8]) -> Option<String> {
    if let Some(rest) = data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).ok();
    }
    if let Some(rest) = data.strip_prefix(&[0xFF, 0xFE]) {
        return codepage::decode_utf16le(rest);
    }
    if let Some(rest) = data.strip_prefix(&[0xFE, 0xFF]) {
        return codepage::decode_utf16be(rest);
    }
    if let Ok(text) = std::str::from_utf8(data) {
        return Some(text.to_string());
    }
    codepage::decode(codepage::GB18030, data).or_else(|| codepage::decode_utf16le(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_markdown;
    use std::path::PathBuf;

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("ClipMDDropTests-{}", crate::clip_history::new_id()));
            std::fs::create_dir_all(&path).unwrap();
            Folder(path)
        }

        fn write(&self, data: &[u8], name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, data).unwrap();
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn utf8_text_and_markdown_files() {
        let folder = Folder::new();
        let txt = folder.write("微信里的一段话".as_bytes(), "消息.txt");
        let md = folder.write("# 标题\n\n- 一".as_bytes(), "笔记.md");
        assert_eq!(load(&txt).unwrap().0, "微信里的一段话");
        assert_eq!(load(&md).unwrap().0, "# 标题\n\n- 一");
    }

    #[test]
    fn gbk_text_file() {
        let folder = Folder::new();
        let bytes: Vec<u8> = (0..46)
            .step_by(2)
            .map(|i| u8::from_str_radix(&"d6d0cec42057696e646f777320c9cfb4e6b5c4cec4b1be"[i..i + 2], 16).unwrap())
            .collect();
        let path = folder.write(&bytes, "旧文件.txt");
        assert_eq!(load(&path).unwrap().0, "中文 Windows 上存的文本");
    }

    #[test]
    fn utf8_bom_and_utf16() {
        let folder = Folder::new();
        let bom = folder.write(&[&[0xEF, 0xBB, 0xBF][..], "记事本存的".as_bytes()].concat(), "记事本.txt");
        assert_eq!(load(&bom).unwrap().0, "记事本存的");
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("中文".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let path = folder.write(&utf16, "utf16.txt");
        assert_eq!(load(&path).unwrap().0, "中文");
    }

    #[test]
    fn rtf_file() {
        let folder = Folder::new();
        let path = folder.write(br"{\rtf1\ansi\ansicpg936\'b4\'f8\uc1\u26684?\'ca\'bd\'b5\'c4\'ce\'c4\uc0\u23383 }", "格式.rtf");
        assert_eq!(load(&path).unwrap().0.trim(), "带格式的文字");
    }

    #[test]
    fn html_file_is_passed_as_html() {
        let folder = Folder::new();
        let path = folder.write("<h1>网页</h1><p>正文</p>".as_bytes(), "页面.html");
        let (text, html) = load(&path).unwrap();
        assert!(text.is_empty());
        assert_eq!(clipboard_markdown::best(&text, html.as_deref()).0, "# 网页\n\n正文");
    }

    #[test]
    fn non_text_files_are_rejected() {
        assert!(!is_text_file(Path::new("/tmp/图片.png")));
        assert!(!is_text_file(Path::new("/tmp/压缩包.zip")));
        assert!(is_text_file(Path::new("/tmp/脚本.py")));
        assert!(load(Path::new("/tmp/图片.png")).is_none());
    }
}
