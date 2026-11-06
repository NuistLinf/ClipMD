//! 窗口的基础：注册窗口类、把消息转给 Rust 对象、DPI、DWM 的圆角和亚克力背景。

#![allow(dead_code)]

use std::rc::Rc;
use std::sync::OnceLock;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, Result, w};

use crate::gfx::Rect;

/// 收到消息的对象。返回 None 表示交给系统默认处理
pub trait Handler {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT>;
}

pub const CLASS: PCWSTR = w!("ClipMD.Window");

pub fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).unwrap_or_default() }
}

fn register() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| unsafe {
        let icon = LoadIconW(Some(instance()), PCWSTR(1 as *const u16)).unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
    });
}

/// 建一个窗口，消息交给 `handler`。窗口销毁时释放 handler
pub fn create(
    title: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    rect: (i32, i32, i32, i32),
    owner: Option<HWND>,
    handler: Rc<dyn Handler>,
) -> Result<HWND> {
    register();
    let boxed: *mut Rc<dyn Handler> = Box::into_raw(Box::new(handler));
    let title = crate::util::h(title);
    unsafe {
        let hwnd = CreateWindowExW(
            ex_style,
            CLASS,
            &title,
            style,
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            owner,
            None,
            Some(instance()),
            Some(boxed as *const _),
        );
        if hwnd.is_err() {
            drop(Box::from_raw(boxed));
        }
        hwnd
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = lp.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Rc<dyn Handler>;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        // 先拿一份引用，处理消息期间对象不会被释放
        let handler = (*ptr).clone();
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
        }
        // 某条消息处理出错（panic）时记进日志、交给系统默认处理，程序继续运行
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler.handle(hwnd, msg, wp, lp)));
        match result {
            Ok(Some(result)) => result,
            Ok(None) => DefWindowProcW(hwnd, msg, wp, lp),
            Err(_) => {
                crate::log!("handler panicked on message 0x{msg:04X}");
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

// MARK: DPI

pub fn dpi(hwnd: HWND) -> u32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 { 96 } else { d }
}

pub fn scale(hwnd: HWND) -> f32 {
    dpi(hwnd) as f32 / 96.0
}

pub fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

pub fn window_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    rect
}

/// 鼠标位置（客户区 DIP）
pub fn mouse_dip(hwnd: HWND, lp: LPARAM) -> (f32, f32) {
    let (x, y) = crate::util::point_from_lparam(lp);
    let s = scale(hwnd);
    (x as f32 / s, y as f32 / s)
}

pub fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

/// 屏幕坐标 → 客户区 DIP
pub fn screen_to_dip(hwnd: HWND, p: POINT) -> (f32, f32) {
    let mut p = p;
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    let s = scale(hwnd);
    (p.x as f32 / s, p.y as f32 / s)
}

/// 某个点所在屏幕的工作区（去掉任务栏）和整个屏幕
pub fn monitor_rects(p: POINT) -> (RECT, RECT) {
    unsafe {
        let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        (info.rcWork, info.rcMonitor)
    }
}

pub fn monitor_dpi_at(p: POINT) -> u32 {
    unsafe {
        let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (96u32, 96u32);
        let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(monitor, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut x, &mut y);
        x
    }
}

pub fn track_mouse_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut tme);
    }
}

pub fn set_cursor(id: PCWSTR) {
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, id) {
            SetCursor(Some(cursor));
        }
    }
}

pub fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// WM_PAINT 里调用：告诉系统这次已经画过了（我们用交换链画，不用 GDI）
pub fn validate(hwnd: HWND) {
    unsafe {
        let _ = ValidateRect(Some(hwnd), None);
    }
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

// MARK: 系统版本、DWM 外观

/// Windows 的内部版本号（Windows 11 22H2 是 22621）
pub fn os_build() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| {
        #[repr(C)]
        struct OsVersionInfo {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            csd: [u16; 128],
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
        }
        let mut info = OsVersionInfo { size: std::mem::size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
        unsafe { RtlGetVersion(&mut info) };
        info.build
    })
}

/// 系统画的亚克力/云母背景要 Windows 11 22H2 以上
pub fn backdrop_supported() -> bool {
    std::env::var_os("CLIPMD_NO_BACKDROP").is_none() && os_build() >= 22621
}

/// Windows 11 才有系统画的圆角
pub fn rounded_corners_supported() -> bool {
    os_build() >= 22000
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// 没有材质，自己画不透明的底
    None,
    /// 亚克力：浮动面板、菜单这类临时窗口
    Acrylic,
    /// 云母：普通窗口（设置、预览）
    Mica,
}

/// 设置标题栏深浅、圆角和背景材质
pub fn style_window(hwnd: HWND, dark: bool, backdrop: Backdrop, corners: bool) {
    unsafe {
        let value: BOOL = dark.into();
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const _ as *const _, 4);
        if corners {
            let pref = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, 4);
        }
        if backdrop_supported() && backdrop != Backdrop::None {
            let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let kind = match backdrop {
                Backdrop::Acrylic => DWMSBT_TRANSIENTWINDOW,
                _ => DWMSBT_MAINWINDOW,
            };
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as *const _, 4);
        } else {
            // 只要窗口阴影：伸进客户区 1 像素就够
            let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        }
    }
}

/// 无边框窗口：WM_NCCALCSIZE 里把整个窗口都当客户区
pub fn borderless_nccalcsize(wp: WPARAM) -> Option<LRESULT> {
    if wp.0 != 0 { Some(LRESULT(0)) } else { None }
}

/// 把 DIP 矩形换成屏幕像素矩形
pub fn dip_rect_to_px(rect: Rect, scale: f32) -> (i32, i32, i32, i32) {
    (
        (rect.x * scale).round() as i32,
        (rect.y * scale).round() as i32,
        (rect.w * scale).round() as i32,
        (rect.h * scale).round() as i32,
    )
}

/// 把窗口放到最前面并激活。Windows 不让后台程序随便抢焦点，先模拟一下 Alt 键再设
pub fn force_foreground(hwnd: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    unsafe {
        if GetForegroundWindow() == hwnd {
            return;
        }
        let inputs = [
            INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MENU, ..Default::default() } } },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MENU, dwFlags: KEYEVENTF_KEYUP, ..Default::default() } },
            },
        ];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(hwnd);
    }
}
