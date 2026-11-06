//! ClipMD 的平台无关逻辑。
//!
//! 和 macOS 版的 `ClipMDCore` 一一对应：HTML 转 Markdown、文件命名、剪贴板记录、公式提取，
//! 测试用例也是从 Swift 版照搬过来的，保证两个平台的结果一样。
//! 只有 Windows 版才有的部分：RTF 读文字（macOS 用系统的 NSAttributedString）、LaTeX 公式排版的数据结构。

pub mod clip_history;
pub mod clipboard_markdown;
pub mod codepage;
pub mod dropped_text;
pub mod html_to_markdown;
pub mod markdown;
pub mod math;
pub mod math_extractor;
pub mod note_namer;
pub mod note_writer;
pub mod rtf;
pub mod text;
pub mod time;
