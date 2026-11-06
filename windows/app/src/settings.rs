//! 偏好设置：存在 `%LOCALAPPDATA%\ClipMD\settings.json`。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hotkeys::Shortcut;
use crate::theme::Appearance;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// 保存位置；None 是默认的 `文档\ClipMD`
    pub save_folder: Option<PathBuf>,
    /// 从网页复制时，把网页格式转成 Markdown
    pub convert_html: bool,
    /// 监听复制：连点两次保存、记录剪贴板都靠它
    pub double_copy: bool,
    /// 记录剪贴板
    pub history: bool,
    pub appearance: Appearance,
    /// 点眼睛按钮时用哪个程序打开；None 是内置预览
    pub preview_app: Option<PathBuf>,
    pub save_shortcut: Shortcut,
    pub screenshot_shortcut: Shortcut,
    pub ocr_shortcut: Shortcut,
    /// 悬浮窗位置（屏幕像素，左上角）
    pub panel_position: Option<(i32, i32)>,
    /// 固定位置：不能拖动
    pub panel_pinned: bool,
    /// 贴边收起在哪一边
    pub tucked: Option<Edge>,
    /// 悬浮窗关着（只留托盘图标）
    pub panel_hidden: bool,
    /// 从列表里移除、但文件还在的文件名
    pub hidden_notes: Vec<String>,
    /// 截图标注的颜色、粗细
    pub capture_color: usize,
    pub capture_width: usize,
    /// 开机自动启动
    pub launch_at_login: bool,
    /// 第一次启动的欢迎提示看过了
    pub welcomed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            save_folder: None,
            convert_html: true,
            double_copy: true,
            history: true,
            appearance: Appearance::System,
            preview_app: None,
            save_shortcut: Shortcut::SAVE,
            screenshot_shortcut: Shortcut::SCREENSHOT,
            ocr_shortcut: Shortcut::OCR,
            panel_position: None,
            panel_pinned: false,
            tucked: None,
            panel_hidden: false,
            hidden_notes: Vec::new(),
            capture_color: 0,
            capture_width: 1,
            launch_at_login: false,
            welcomed: false,
        }
    }
}

fn path() -> PathBuf {
    crate::util::data_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read(path()).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(data) = serde_json::to_vec_pretty(self) {
            let target = path();
            let temp = target.with_extension("json.tmp");
            if std::fs::write(&temp, data).is_ok() {
                let _ = std::fs::rename(&temp, &target);
            }
        }
    }

    pub fn folder(&self) -> PathBuf {
        self.save_folder.clone().unwrap_or_else(|| crate::util::documents_dir().join("ClipMD"))
    }
}

/// 开机启动：写在 HKCU\...\Run 里
pub fn set_launch_at_login(enabled: bool) {
    use windows::Win32::System::Registry::*;
    use windows::core::w;
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), Some(0), KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        if enabled {
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let value = crate::util::wide(&format!("\"{exe}\" --background"));
            let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
            let _ = RegSetValueExW(key, w!("ClipMD"), None, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, w!("ClipMD"));
        }
        let _ = RegCloseKey(key);
    }
}
