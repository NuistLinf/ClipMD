//! 抓屏和窗口列表。
//!
//! 抓屏用 GDI 的 BitBlt（DWM 合成后的画面，和看到的一样）；窗口用 EnumWindows 按从上到下的顺序列出，
//! 位置用 DWM 的"扩展边框"（不含 Windows 10/11 窗口四周看不见的拖拽边）。

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

use crate::images::Image;

/// 抓屏幕上一块区域（屏幕像素坐标）
pub fn grab_screen_rect(x: i32, y: i32, w: i32, h: i32) -> Option<Image> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        let mut result = None;
        if let Some(bitmap) = bitmap {
            let old = SelectObject(mem, bitmap.into());
            if BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok() {
                let len = (w * h * 4) as usize;
                let mut pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
                for px in pixels.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                result = Some(Image { width: w as u32, height: h as u32, pixels });
            }
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        result
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    /// 屏幕像素
    pub rect: RECT,
    pub dpi: u32,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _dc: HDC, rect: *mut RECT, data: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(data.0 as *mut Vec<Monitor>);
            let (mut x, mut y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
            list.push(Monitor { rect: *rect, dpi: x });
        }
        true.into()
    }
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

/// 屏幕上能看到的窗口（从上到下），屏幕像素坐标
pub fn visible_windows() -> Vec<RECT> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(data.0 as *mut Vec<RECT>);
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return true.into();
            }
            // 虚拟桌面上别的桌面的窗口、UWP 挂起的窗口是"隐身"的
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
            if cloaked != 0 {
                return true.into();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == std::process::id() {
                // ClipMD 自己的截图遮罩不算；悬浮窗算（可以截它）
                let mut title = [0u16; 64];
                let len = GetWindowTextW(hwnd, &mut title);
                let title = String::from_utf16_lossy(&title[..len.max(0) as usize]);
                if title != "ClipMD" {
                    return true.into();
                }
            }
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TRANSPARENT.0 != 0 && ex & WS_EX_LAYERED.0 != 0 {
                return true.into();
            }
            let mut rect = RECT::default();
            if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_err() {
                let _ = GetWindowRect(hwnd, &mut rect);
            }
            if rect.right - rect.left >= 8 && rect.bottom - rect.top >= 8 {
                list.push(rect);
            }
        }
        true.into()
    }
    let mut list: Vec<RECT> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}
