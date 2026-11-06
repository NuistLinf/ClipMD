//! 全局快捷键：RegisterHotKey，按下时系统给消息窗口发 WM_HOTKEY。

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// 虚拟键码
    pub key: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Save = 1,
    Screenshot = 2,
    Ocr = 3,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Save, Action::Screenshot, Action::Ocr];

    pub fn title(self) -> &'static str {
        match self {
            Action::Save => "保存剪贴板",
            Action::Screenshot => "截图",
            Action::Ocr => "截图并识别文字",
        }
    }

    pub fn from_id(id: usize) -> Option<Action> {
        Action::ALL.into_iter().find(|a| *a as usize == id)
    }

    pub fn default_shortcut(self) -> Shortcut {
        match self {
            Action::Save => Shortcut::SAVE,
            Action::Screenshot => Shortcut::SCREENSHOT,
            Action::Ocr => Shortcut::OCR,
        }
    }
}

impl Shortcut {
    pub const SAVE: Shortcut = Shortcut { ctrl: true, alt: true, shift: false, win: false, key: b'S' as u16 };
    pub const SCREENSHOT: Shortcut = Shortcut { ctrl: true, alt: true, shift: false, win: false, key: b'A' as u16 };
    pub const OCR: Shortcut = Shortcut { ctrl: true, alt: true, shift: false, win: false, key: b'O' as u16 };

    /// 至少要有 Ctrl、Alt、Win 之一，免得抢走普通打字
    pub fn is_valid(&self) -> bool {
        (self.ctrl || self.alt || self.win) && self.key != 0
    }

    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.win {
            parts.push("Win".into());
        }
        parts.push(key_name(self.key));
        parts.join("+")
    }

    fn modifiers(&self) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        if self.win {
            m |= MOD_WIN;
        }
        m
    }

    /// 从按键消息里组合出快捷键（设置里录制用）
    pub fn from_key(vk: u16) -> Shortcut {
        let down = |k: VIRTUAL_KEY| unsafe { GetKeyState(k.0 as i32) } < 0;
        Shortcut { ctrl: down(VK_CONTROL), alt: down(VK_MENU), shift: down(VK_SHIFT), win: down(VK_LWIN) || down(VK_RWIN), key: vk }
    }
}

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x20 => "Space".into(),
        0x0D => "Enter".into(),
        0x09 => "Tab".into(),
        0x08 => "Backspace".into(),
        0x2E => "Delete".into(),
        0x2D => "Insert".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x2C => "PrintScreen".into(),
        0xC0 => "`".into(),
        0xBD => "-".into(),
        0xBB => "=".into(),
        0xDB => "[".into(),
        0xDD => "]".into(),
        0xDC => "\\".into(),
        0xBA => ";".into(),
        0xDE => "'".into(),
        0xBC => ",".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        _ => format!("键 {vk}"),
    }
}

/// 是不是只按了修饰键（录制时要忽略）
pub fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// 注册三个快捷键，返回注册失败的（被别的程序占用了）
pub fn register_all(hwnd: HWND, shortcuts: &[(Action, Shortcut)]) -> Vec<Action> {
    let mut failed = Vec::new();
    for &(action, shortcut) in shortcuts {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), action as i32);
            if !shortcut.is_valid() || RegisterHotKey(Some(hwnd), action as i32, shortcut.modifiers(), shortcut.key as u32).is_err() {
                failed.push(action);
            }
        }
    }
    failed
}

pub fn unregister_all(hwnd: HWND) {
    for action in Action::ALL {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), action as i32);
        }
    }
}
