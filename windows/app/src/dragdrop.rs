//! 拖放：把文字、文本文件拖进悬浮窗（上半块存成 Markdown，下半块只放进剪贴板）；
//! 把已保存的 md 文件拖出去（拖到微信就是发送文件，拖到回收站就是删除）。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use clipmd_core::dropped_text;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::*;
use windows::core::{Ref, Result, implement, w};

use crate::app::app;
use crate::hud::{self, Toast};
use crate::panel::{DropView, Zone};

/// 拖进来的东西
#[derive(Clone, Debug)]
enum Payload {
    Text { text: String, html: Option<String> },
    Files(Vec<PathBuf>),
    Unsupported(String),
}

#[implement(IDropTarget)]
struct DropTarget {
    payload: RefCell<Option<Payload>>,
}

fn format(cf: u32) -> FORMATETC {
    FORMATETC { cfFormat: cf as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

fn read_global(data: &IDataObject, cf: u32) -> Option<Vec<u8>> {
    unsafe {
        let fmt = format(cf);
        if data.QueryGetData(&fmt).is_err() {
            return None;
        }
        let mut medium = data.GetData(&fmt).ok()?;
        let global = medium.u.hGlobal;
        let size = GlobalSize(global);
        let ptr = GlobalLock(global) as *const u8;
        let bytes = if ptr.is_null() { None } else { Some(std::slice::from_raw_parts(ptr, size).to_vec()) };
        let _ = GlobalUnlock(global);
        ReleaseStgMedium(&mut medium);
        bytes
    }
}

fn read_payload(data: &IDataObject) -> Payload {
    let html_format = unsafe { windows::Win32::System::DataExchange::RegisterClipboardFormatW(w!("HTML Format")) };
    if let Some(bytes) = read_global(data, CF_HDROP.0 as u32) {
        let files = parse_dropfiles(&bytes);
        if !files.is_empty() && files.iter().all(|f| dropped_text::is_text_file(f)) {
            return Payload::Files(files);
        }
        return Payload::Unsupported("只能存文字和 .md、.txt、.html、.rtf、代码这类文本文件".into());
    }
    if let Some(bytes) = read_global(data, CF_UNICODETEXT.0 as u32) {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let text = crate::util::from_wide(&units);
        let html = read_global(data, html_format).and_then(|b| crate::clipboard::parse_cf_html(&b));
        if text.chars().any(|c| !c.is_whitespace()) || html.is_some() {
            return Payload::Text { text, html };
        }
    }
    Payload::Unsupported("先选中一段文字再拖进来".into())
}

/// DROPFILES 结构后面跟着以 0 分隔的路径
fn parse_dropfiles(bytes: &[u8]) -> Vec<PathBuf> {
    if bytes.len() < 20 {
        return Vec::new();
    }
    let offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let wide = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) != 0;
    if offset >= bytes.len() || !wide {
        return Vec::new();
    }
    let units: Vec<u16> = bytes[offset..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    units.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| PathBuf::from(String::from_utf16_lossy(s))).collect()
}

fn view_for(payload: &Payload, zone: Zone) -> DropView {
    match payload {
        Payload::Unsupported(message) => DropView::Unsupported(message.clone()),
        _ => DropView::Zones(Some(zone)),
    }
}

impl DropTarget_Impl {
    fn update(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        let Some(panel) = app().panel() else { return };
        let zone = panel.zone_at(POINT { x: pt.x, y: pt.y });
        let payload = self.payload.borrow().clone();
        let (view, allowed) = match &payload {
            Some(p) => (view_for(p, zone), !matches!(p, Payload::Unsupported(_))),
            None => (DropView::Unsupported("拖不进来".into()), false),
        };
        panel.set_drop(Some(view));
        unsafe {
            if !effect.is_null() {
                *effect = if allowed { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
            }
        }
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let payload = data.ok().map(read_payload).unwrap_or(Payload::Unsupported("拖不进来".into()));
        *self.payload.borrow_mut() = Some(payload);
        self.update(pt, effect);
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.update(pt, effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        *self.payload.borrow_mut() = None;
        if let Some(panel) = app().panel() {
            panel.set_drop(None);
        }
        Ok(())
    }

    fn Drop(&self, _data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let payload = self.payload.borrow_mut().take();
        let panel = app().panel();
        let zone = panel.as_ref().map(|p| p.zone_at(POINT { x: pt.x, y: pt.y })).unwrap_or(Zone::Save);
        if let Some(panel) = &panel {
            panel.set_drop(None);
        }
        unsafe {
            if !effect.is_null() {
                *effect = if matches!(payload, Some(Payload::Unsupported(_)) | None) { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
            }
        }
        if let Some(payload) = payload {
            // 拖放的模态循环里不要弹东西，回到消息循环再处理
            let send = SendPayload(payload);
            crate::app::run_on_main(move || handle_drop(send.0, zone));
        }
        Ok(())
    }
}

struct SendPayload(Payload);
// Payload 里只有字符串和路径
unsafe impl Send for SendPayload {}

fn handle_drop(payload: Payload, zone: Zone) {
    let a = app();
    match payload {
        Payload::Text { text, html } => match zone {
            Zone::Save => {
                a.save_text(&text, html.as_deref(), false);
            }
            Zone::Clipboard => {
                crate::clipboard::write_text(a.hwnd.get(), &text, html.as_deref());
                if a.settings.borrow().history {
                    a.history.borrow_mut().add_text(&text, html.as_deref(), Some("拖进来的"));
                }
                a.refresh_panel();
                hud::show(Toast::success("已放到剪贴板", "Ctrl+V 直接粘贴"));
            }
        },
        Payload::Files(files) => {
            let mut saved = 0;
            let mut combined = String::new();
            for file in &files {
                let Some((text, html)) = dropped_text::load(file) else { continue };
                match zone {
                    Zone::Save => {
                        if a.save_text(&text, html.as_deref(), false).is_some() {
                            saved += 1;
                        }
                    }
                    Zone::Clipboard => {
                        let (markdown, _) = clipmd_core::clipboard_markdown::best(&text, html.as_deref());
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(&markdown);
                    }
                }
            }
            match zone {
                Zone::Save if saved > 1 => hud::show(Toast::success(&format!("已保存 {saved} 个文件"), "在「已保存」里")),
                Zone::Save if saved == 0 => hud::show(Toast::warning("读不出文字", "文件可能是空的，或者编码不认识")),
                Zone::Clipboard if !combined.is_empty() => {
                    crate::clipboard::write_text(a.hwnd.get(), &combined, None);
                    hud::show(Toast::success("已放到剪贴板", "Ctrl+V 直接粘贴"));
                }
                _ => {}
            }
        }
        Payload::Unsupported(_) => {}
    }
}

pub fn register_drop_target(hwnd: HWND) {
    let target: IDropTarget = DropTarget { payload: RefCell::new(None) }.into();
    unsafe {
        if let Err(e) = RegisterDragDrop(hwnd, &target) {
            crate::log!("RegisterDragDrop failed: {e:?}");
        }
    }
}

pub fn revoke_drop_target(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

/// 把文件拖出去（会阻塞到松开鼠标）。用系统给文件建的数据对象，资源管理器、微信、回收站都认
pub fn drag_file_out(path: &Path) {
    unsafe {
        let wide = crate::util::h(&path.to_string_lossy());
        let item: Result<IShellItem> = SHCreateItemFromParsingName(&wide, None);
        let Ok(item) = item else { return };
        let data: Result<IDataObject> = item.BindToHandler(None, &BHID_DataObject);
        let Ok(data) = data else { return };
        // 默认是复制：拖进同一个磁盘的文件夹也不会把原文件挪走
        let preferred = windows::Win32::System::DataExchange::RegisterClipboardFormatW(w!("Preferred DropEffect"));
        if let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, 4) {
            let ptr = GlobalLock(global) as *mut u32;
            if !ptr.is_null() {
                *ptr = DROPEFFECT_COPY.0;
                let _ = GlobalUnlock(global);
                let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: global }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
                let _ = data.SetData(&format(preferred), &medium, true);
            }
        }
        let result = SHDoDragDrop(None, &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK);
        crate::log!("drag out {}: {:?}", path.display(), result.map(|e| e.0));
    }
}
