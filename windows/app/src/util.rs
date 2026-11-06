//! 杂项：宽字符串、路径、时间、日志。

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{FOLDERID_Documents, FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR};

/// 以 0 结尾的 UTF-16
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub unsafe fn from_pwstr(p: PWSTR) -> String {
    if p.is_null() { String::new() } else { unsafe { p.to_string().unwrap_or_default() } }
}

pub fn h(s: &str) -> HSTRING {
    HSTRING::from(s)
}

pub fn pcwstr(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = from_pwstr(p);
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        if path.is_empty() { None } else { Some(PathBuf::from(path)) }
    }
}

/// `%LOCALAPPDATA%\ClipMD`：设置、剪贴板记录、日志
pub fn data_dir() -> PathBuf {
    // 自检时用单独的文件夹，不碰真正的记录和设置
    if let Some(dir) = std::env::var_os("CLIPMD_DATA_DIR") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }
    let base = known_folder(&FOLDERID_LocalAppData).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("ClipMD");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 「文档」文件夹，默认保存位置在 `文档\ClipMD`
pub fn documents_dir() -> PathBuf {
    known_folder(&FOLDERID_Documents).unwrap_or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Documents"))
}

/// 现在比 UTC 快多少秒（用本地时间和 UTC 时间相减，夏令时也对）
pub fn utc_offset() -> i64 {
    unsafe {
        let local = GetLocalTime();
        let utc = windows::Win32::System::SystemInformation::GetSystemTime();
        let secs = |t: windows::Win32::Foundation::SYSTEMTIME| {
            clipmd_core::time::days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32) * 86_400
                + t.wHour as i64 * 3600
                + t.wMinute as i64 * 60
        };
        let diff = secs(local) - secs(utc);
        (diff as f64 / 900.0).round() as i64 * 900
    }
}

pub fn local_date() -> clipmd_core::time::LocalDate {
    clipmd_core::time::local_time(clipmd_core::time::now_unix(), utc_offset()).date
}

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn log_line(message: &str) {
    let line = format!("{} {message}\n", clipmd_core::time::format_iso8601(clipmd_core::time::now_unix()));
    unsafe { OutputDebugStringW(&h(&line)) };
    if let Ok(mut guard) = LOG.lock() {
        if guard.is_none() {
            let path = data_dir().join("ClipMD.log");
            // 超过 1 MB 就重新开始
            if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
                let _ = std::fs::rename(&path, data_dir().join("ClipMD.old.log"));
            }
            *guard = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        }
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    }
    if std::env::var_os("CLIPMD_STDERR_LOG").is_some() {
        eprint!("{line}");
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::util::log_line(&format!($($arg)*)) };
}

pub fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

pub fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

/// 鼠标消息里的坐标（有符号，多屏时可能是负数）
pub fn point_from_lparam(lp: LPARAM) -> (i32, i32) {
    let x = (lp.0 & 0xFFFF) as i16 as i32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

pub fn wheel_delta(wp: WPARAM) -> i16 {
    ((wp.0 >> 16) & 0xFFFF) as i16
}

pub fn hwnd_is_null(hwnd: HWND) -> bool {
    hwnd.0.is_null()
}

/// 毫秒级的单调时钟，动画用
pub fn now_seconds() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// 把文件路径变成可以显示的样子："C:\Users\me\Documents\ClipMD" → "文档\ClipMD"
pub fn friendly_path(path: &std::path::Path) -> String {
    let docs = documents_dir();
    if let Ok(rest) = path.strip_prefix(&docs) {
        let rest = rest.to_string_lossy();
        return if rest.is_empty() { "文档".into() } else { format!("文档\\{rest}") };
    }
    path.to_string_lossy().into_owned()
}
