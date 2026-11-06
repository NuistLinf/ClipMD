//! 右键菜单：系统原生菜单（Windows 11 上自带圆角和动画），跟着 ClipMD 的外观设置切换深浅色。

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCSTR, w};

pub struct Item {
    pub id: u32,
    pub label: String,
    pub checked: bool,
    pub enabled: bool,
}

pub enum Entry {
    Item(Item),
    Separator,
    Submenu(String, Vec<Entry>),
}

pub fn item(id: u32, label: &str) -> Entry {
    Entry::Item(Item { id, label: label.to_string(), checked: false, enabled: true })
}

pub fn checked(id: u32, label: &str, on: bool) -> Entry {
    Entry::Item(Item { id, label: label.to_string(), checked: on, enabled: true })
}

pub fn disabled(id: u32, label: &str) -> Entry {
    Entry::Item(Item { id, label: label.to_string(), checked: false, enabled: false })
}

/// 让系统菜单跟着深浅色走（uxtheme 里没公开的 SetPreferredAppMode，记事本等很多程序都这么用）
pub fn apply_theme(dark: bool) {
    unsafe {
        let Ok(module) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(set_mode) = GetProcAddress(module, PCSTR(135 as *const u8)) {
            let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            set_mode(if dark { 2 } else { 3 });
        }
        if let Some(flush) = GetProcAddress(module, PCSTR(136 as *const u8)) {
            let flush: extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}

fn build(entries: &[Entry]) -> HMENU {
    unsafe {
        let menu = CreatePopupMenu().unwrap_or_default();
        for entry in entries {
            match entry {
                Entry::Item(item) => {
                    let mut flags = MF_STRING;
                    if item.checked {
                        flags |= MF_CHECKED;
                    }
                    if !item.enabled {
                        flags |= MF_GRAYED;
                    }
                    let label = crate::util::h(&item.label);
                    let _ = AppendMenuW(menu, flags, item.id as usize, &label);
                }
                Entry::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                }
                Entry::Submenu(label, children) => {
                    let sub = build(children);
                    let label = crate::util::h(label);
                    let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &label);
                }
            }
        }
        menu
    }
}

/// 在屏幕位置 `at` 弹出菜单，返回选中的编号。`owner` 要是能当前台的普通窗口（隐藏的也行）
pub fn popup(owner: HWND, entries: &[Entry], at: POINT) -> Option<u32> {
    apply_theme(crate::theme::is_dark());
    unsafe {
        let menu = build(entries);
        // 不先设成前台窗口，点菜单外面时菜单不会关
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0, at.x, at.y, owner, None);
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if chosen.0 > 0 { Some(chosen.0 as u32) } else { None }
    }
}
