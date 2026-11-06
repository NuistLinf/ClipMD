//! 托盘图标：单色的 ClipMD 标志，任务栏深色时是白色、浅色时是黑色（和 macOS 菜单栏的模板图一样跟着变）。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON, SM_CXSMICON};

use crate::gfx::{Color, Rect};
use crate::icons;

pub struct Tray {
    hwnd: HWND,
    message: u32,
    icon: Option<HICON>,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND, message: u32) -> Option<Tray> {
        let mut tray = Tray { hwnd, message, icon: None, added: false };
        tray.readd();
        Some(tray)
    }

    fn make_icon() -> Option<HICON> {
        let dpi = crate::window::monitor_dpi_at(windows::Win32::Foundation::POINT { x: 0, y: 0 });
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.max(16) as u32;
        let color = if crate::theme::taskbar_dark() { Color::WHITE } else { Color::hex(0x1A1A1A) };
        icons::make_icon(size, |p| {
            let r = Rect::new(0.0, 0.0, size as f32, size as f32);
            icons::tray_glyph(p, r, color, size);
        })
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: self.message,
            hIcon: self.icon.unwrap_or_default(),
            ..Default::default()
        };
        let tip: Vec<u16> = "ClipMD：复制后按 Ctrl+Alt+S 存成 Markdown".encode_utf16().collect();
        data.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        data
    }

    /// 加上图标（资源管理器重启后也调用这个）
    pub fn readd(&mut self) {
        if let Some(old) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
        self.icon = Self::make_icon();
        let data = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            self.added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    /// 任务栏深浅变了，换图标颜色
    pub fn update_icon(&mut self) {
        let old = self.icon.take();
        self.icon = Self::make_icon();
        let data = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            if let Some(old) = old {
                let _ = DestroyIcon(old);
            }
        }
    }

    pub fn remove(&mut self) {
        let data = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            if let Some(icon) = self.icon.take() {
                let _ = DestroyIcon(icon);
            }
        }
        self.added = false;
    }
}
