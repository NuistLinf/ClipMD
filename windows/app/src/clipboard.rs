//! 系统剪贴板的读写。
//!
//! - 放文件：CF_HDROP + "Preferred DropEffect"，和在资源管理器里按 Ctrl+C 一样，粘贴到微信、邮件里就是附件
//! - 放文字：CF_UNICODETEXT，有网页格式时再加 "HTML Format"
//! - 放图片："PNG" + CF_DIBV5（画图这类老程序只认位图）
//! - 密码管理器复制的内容带 "ExcludeClipboardContentFromMonitorProcessing" 等标记，不记录

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};
use windows::core::{PCWSTR, w};

use crate::images::{self, Image};

/// 剪贴板里大概是什么。只看格式、不读内容
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Text { from_web: bool },
    Image,
    File,
    Empty,
    /// 密码类内容
    Concealed,
}

impl Hint {
    pub fn can_save(&self) -> bool {
        matches!(self, Hint::Text { .. })
    }

    pub fn message(&self, convert_html: bool) -> &'static str {
        match self {
            Hint::Text { from_web: true } => {
                if convert_html { "网页文字，会保留标题、列表等格式" } else { "剪贴板里有网页上复制的文字" }
            }
            Hint::Text { from_web: false } => "剪贴板里有文字，点一下就存",
            Hint::Image => "剪贴板里是图片，可以在「剪贴板」里识别文字",
            Hint::File => "剪贴板里是文件，先复制一段文字",
            Hint::Empty => "先复制一段文字",
            Hint::Concealed => "剪贴板里是密码类内容，不会保存",
        }
    }
}

struct Formats {
    html: u32,
    png: u32,
    exclude: u32,
    can_include: u32,
    viewer_ignore: u32,
    preferred_effect: u32,
}

fn formats() -> &'static Formats {
    static FORMATS: OnceLock<Formats> = OnceLock::new();
    FORMATS.get_or_init(|| unsafe {
        Formats {
            html: RegisterClipboardFormatW(w!("HTML Format")),
            png: RegisterClipboardFormatW(w!("PNG")),
            exclude: RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")),
            can_include: RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
            viewer_ignore: RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
            preferred_effect: RegisterClipboardFormatW(w!("Preferred DropEffect")),
        }
    })
}

fn available(format: u32) -> bool {
    unsafe { IsClipboardFormatAvailable(format).is_ok() }
}

/// 剪贴板被别的程序占着时等一下再试
struct Open;

impl Open {
    fn new(owner: Option<HWND>) -> Option<Open> {
        for attempt in 0..12 {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Some(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(if attempt < 4 { 5 } else { 20 }));
        }
        crate::log!("clipboard busy");
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub fn hint() -> Hint {
    let f = formats();
    if available(f.exclude) || available(f.viewer_ignore) || can_include_is_zero() {
        return Hint::Concealed;
    }
    if available(CF_HDROP.0 as u32) {
        return Hint::File;
    }
    if available(CF_UNICODETEXT.0 as u32) {
        return Hint::Text { from_web: available(f.html) };
    }
    if available(f.png) || available(CF_DIBV5.0 as u32) || available(CF_DIB.0 as u32) {
        return Hint::Image;
    }
    Hint::Empty
}

/// "CanIncludeInClipboardHistory" = 0 表示内容不想被记录（Windows 自己的剪贴板历史也遵守）
fn can_include_is_zero() -> bool {
    let f = formats();
    if !available(f.can_include) {
        return false;
    }
    read_bytes(f.can_include).is_some_and(|b| b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0)
}

fn read_bytes(format: u32) -> Option<Vec<u8>> {
    let _open = Open::new(None)?;
    unsafe { global_bytes(GetClipboardData(format).ok()?) }
}

unsafe fn global_bytes(handle: HANDLE) -> Option<Vec<u8>> {
    unsafe {
        let global = HGLOBAL(handle.0);
        let size = GlobalSize(global);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(data)
    }
}

pub fn read_text() -> Option<String> {
    let _open = Open::new(None)?;
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
        let global = HGLOBAL(handle.0);
        let ptr = GlobalLock(global) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let max = GlobalSize(global) / 2;
        let slice = std::slice::from_raw_parts(ptr, max);
        let text = crate::util::from_wide(slice);
        let _ = GlobalUnlock(global);
        Some(text)
    }
}

/// 网页格式：剪贴板里是带说明头的 HTML，按头里的偏移取出正文
pub fn read_html() -> Option<String> {
    let bytes = read_bytes(formats().html)?;
    parse_cf_html(&bytes)
}

pub fn parse_cf_html(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(400)]);
    let value = |key: &str| -> Option<i64> {
        let start = head.find(key)? + key.len();
        let digits: String = head[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        digits.parse().ok()
    };
    let (start, end) = match (value("StartHTML:"), value("EndHTML:")) {
        (Some(s), Some(e)) if s >= 0 && e > s => (s as usize, e as usize),
        _ => (value("StartFragment:")? as usize, value("EndFragment:")? as usize),
    };
    let end = end.min(bytes.len());
    if start >= end {
        return None;
    }
    let html = String::from_utf8_lossy(&bytes[start..end]).trim_end_matches('\0').to_string();
    if html.trim().is_empty() { None } else { Some(html) }
}

/// 剪贴板里的图片：优先 PNG（带透明度），没有再读位图
pub fn read_image() -> Option<Image> {
    let f = formats();
    if available(f.png) {
        if let Some(bytes) = read_bytes(f.png) {
            if let Ok(image) = images::decode_bytes(&bytes, None) {
                return Some(image);
            }
        }
    }
    for format in [CF_DIBV5.0 as u32, CF_DIB.0 as u32] {
        if available(format) {
            if let Some(image) = read_bytes(format).and_then(|b| images::from_dib(&b)) {
                return Some(image);
            }
        }
    }
    None
}

pub fn read_files() -> Vec<PathBuf> {
    let Some(_open) = Open::new(None) else { return Vec::new() };
    unsafe {
        let Ok(handle) = GetClipboardData(CF_HDROP.0 as u32) else { return Vec::new() };
        files_from_hdrop(HDROP(handle.0))
    }
}

pub unsafe fn files_from_hdrop(drop: HDROP) -> Vec<PathBuf> {
    unsafe {
        let count = DragQueryFileW(drop, 0xFFFF_FFFF, None);
        (0..count)
            .filter_map(|i| {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(drop, i, Some(&mut buf));
                let path = crate::util::from_wide(&buf);
                if path.is_empty() { None } else { Some(PathBuf::from(path)) }
            })
            .collect()
    }
}

/// 剪贴板最后一次是不是 ClipMD 自己写的
pub fn owned_by(hwnd: HWND) -> bool {
    unsafe { GetClipboardOwner().map(|owner| owner == hwnd).unwrap_or(false) }
}

/// 剪贴板的序号，每次内容变化加一
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

// MARK: 写

fn global_from(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
        let ptr = GlobalLock(global) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(global));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(global);
        Some(global)
    }
}

fn set(format: u32, bytes: &[u8]) -> bool {
    let Some(global) = global_from(bytes) else { return false };
    unsafe {
        if SetClipboardData(format, Some(HANDLE(global.0))).is_err() {
            let _ = GlobalFree(Some(global));
            return false;
        }
    }
    true
}

fn wide_bytes(text: &str) -> Vec<u8> {
    crate::util::wide(text).iter().flat_map(|c| c.to_le_bytes()).collect()
}

/// 不想被 Windows 剪贴板历史、云剪贴板记下的内容（颜色值这种）不用标；文件、文字照常
fn begin(owner: HWND) -> Option<Open> {
    let open = Open::new(Some(owner))?;
    unsafe {
        let _ = EmptyClipboard();
    }
    Some(open)
}

/// 和在资源管理器里对文件按 Ctrl+C 效果相同
pub fn write_files(owner: HWND, paths: &[&Path]) -> bool {
    let Some(_open) = begin(owner) else { return false };
    let header = std::mem::size_of::<DROPFILES>();
    let mut data = vec![0u8; header];
    for path in paths {
        data.extend(wide_bytes(&path.to_string_lossy()));
    }
    data.extend_from_slice(&[0, 0]);
    let drop = DROPFILES { pFiles: header as u32, pt: POINT::default(), fNC: false.into(), fWide: true.into() };
    unsafe {
        std::ptr::copy_nonoverlapping(&drop as *const DROPFILES as *const u8, data.as_mut_ptr(), header);
    }
    let ok = set(CF_HDROP.0 as u32, &data);
    // DROPEFFECT_COPY：粘贴时复制，不是移动
    set(formats().preferred_effect, &1u32.to_le_bytes());
    ok
}

pub fn write_file(owner: HWND, path: &Path) -> bool {
    write_files(owner, &[path])
}

/// 文字和网页格式一起放：粘贴到支持格式的程序里能保留标题、列表这些
pub fn write_text(owner: HWND, text: &str, html: Option<&str>) -> bool {
    let Some(_open) = begin(owner) else { return false };
    let ok = set(CF_UNICODETEXT.0 as u32, &wide_bytes(text));
    if let Some(html) = html {
        set(formats().html, &build_cf_html(html));
    }
    ok
}

pub fn write_image(owner: HWND, image: &Image, png: Option<&[u8]>) -> bool {
    let Some(_open) = begin(owner) else { return false };
    let mut ok = false;
    if let Some(png) = png {
        ok |= set(formats().png, png);
    }
    ok |= set(CF_DIBV5.0 as u32, &images::to_dibv5(image));
    ok
}

/// 把 HTML 包成剪贴板要的格式（带偏移量的说明头）
pub fn build_cf_html(html: &str) -> Vec<u8> {
    const HEADER: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let prefix = "<html><body>\r\n<!--StartFragment-->";
    let suffix = "<!--EndFragment-->\r\n</body>\r\n</html>";
    let start_html = HEADER.len();
    let start_fragment = start_html + prefix.len();
    let end_fragment = start_fragment + html.len();
    let end_html = end_fragment + suffix.len();
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    let mut out = Vec::with_capacity(end_html + 1);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(prefix.as_bytes());
    out.extend_from_slice(html.as_bytes());
    out.extend_from_slice(suffix.as_bytes());
    out.push(0);
    out
}

/// 复制剪贴板的来源程序名（"微信"、"Google Chrome"）
pub fn source_app() -> Option<String> {
    unsafe {
        let owner = GetClipboardOwner().ok().filter(|h| !h.0.is_null());
        let hwnd = owner.unwrap_or_else(|| windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow());
        process_name_of_window(hwnd)
    }
}

pub fn process_name_of_window(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::Threading::*;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(file_description(&path).unwrap_or_else(|| {
            Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(path.clone())
        }))
    }
}

/// exe 的"文件说明"（版本信息里的 FileDescription）
fn file_description(path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::*;
    unsafe {
        let wide = crate::util::h(path);
        let size = GetFileVersionInfoSizeW(&wide, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(&wide, None, size, data.as_mut_ptr() as *mut _).ok()?;
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(data.as_ptr() as *const _, w!("\\VarFileInfo\\Translation"), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let lang = *(ptr as *const u16);
        let codepage = *(ptr as *const u16).add(1);
        let key = crate::util::wide(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription"));
        if !VerQueryValueW(data.as_ptr() as *const _, PCWSTR(key.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = crate::util::from_wide(std::slice::from_raw_parts(ptr as *const u16, len as usize));
        let text = text.trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}
