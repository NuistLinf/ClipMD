//! 监听复制：检测"连点两次复制"，记录剪贴板，提示悬浮窗"刚复制了一段文字"。
//!
//! Windows 有剪贴板变化通知（AddClipboardFormatListener），不用像 macOS 版那样每 0.1 秒轮询，闲着时完全不占 CPU。
//!
//! 判断规则和 macOS 版一样：同一段文字 3 秒内被复制两次才算连点。另外有的程序一次复制会分两次写剪贴板，
//! 相隔不到 0.12 秒的两次变化当成一次。ClipMD 自己写的剪贴板不算。

use clipmd_core::text::fnv1a;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::app::{App, WM_CLIP_RETRY, background, run_on_main};
use crate::clipboard::{self, Hint};
use crate::log;

const SAME_TEXT_WINDOW: f64 = 3.0;
const MERGE_WINDOW: f64 = 0.12;
const MAX_EMPTY_RETRIES: u32 = 5;

#[derive(Default)]
pub struct Watcher {
    last: Option<(f64, u64)>,
    last_sequence: u32,
}

impl Watcher {
    pub fn reset(&mut self) {
        self.last = None;
    }

    pub fn on_clipboard_update(app: &App, retry: u32) {
        let hwnd = app.hwnd.get();
        let sequence = clipboard::sequence();
        if clipboard::owned_by(hwnd) {
            // 自己写的：不算复制，也不记录
            let mut w = app.watcher.borrow_mut();
            w.last = None;
            w.last_sequence = sequence;
            drop(w);
            app.refresh_hint();
            return;
        }
        let hint = clipboard::hint();
        if hint == Hint::Empty && retry < MAX_EMPTY_RETRIES {
            // 程序"先清空、再写入"，正好在中间收到通知；过一会再看
            let target = hwnd.0 as isize;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(40));
                unsafe {
                    let _ = PostMessageW(Some(windows::Win32::Foundation::HWND(target as *mut _)), WM_CLIP_RETRY, WPARAM(retry as usize + 1), LPARAM(0));
                }
            });
            return;
        }
        let now = crate::util::now_seconds();
        let settings = app.settings.borrow().clone();
        match hint {
            Hint::Text { .. } => {
                let Some(text) = clipboard::read_text() else { return };
                if !text.chars().any(|c| !c.is_whitespace()) {
                    return;
                }
                let hash = fnv1a(text.as_bytes());
                let previous = app.watcher.borrow().last;
                let gap = previous.map(|(t, _)| now - t).unwrap_or(f64::MAX);
                let same = previous.is_some_and(|(_, h)| h == hash);
                if same && gap < MERGE_WINDOW {
                    // 同一次复制的第二次写入
                    return;
                }
                let is_double = same && gap <= SAME_TEXT_WINDOW;
                log!("clipboard text {} chars, gap={:.0}ms double={is_double}", text.chars().count(), gap.min(99_999.0) * 1000.0);
                {
                    let mut w = app.watcher.borrow_mut();
                    w.last = if is_double { None } else { Some((now, hash)) };
                    w.last_sequence = sequence;
                }
                if settings.history {
                    let html = clipboard::read_html();
                    let source = clipboard::source_app();
                    app.history.borrow_mut().add_text(&text, html.as_deref(), source.as_deref());
                }
                if is_double {
                    app.save_clipboard(true);
                } else if let Some(panel) = app.panel() {
                    panel.set_hint(hint, true);
                }
                app.refresh_panel();
            }
            Hint::Image => {
                app.watcher.borrow_mut().last = None;
                if let Some(panel) = app.panel() {
                    panel.set_hint(hint, false);
                }
                if !settings.history {
                    return;
                }
                let Some(image) = clipboard::read_image() else { return };
                let source = clipboard::source_app();
                // 编码 PNG 放到后台，大截图也不卡界面
                background("encode-clip", move || {
                    let Ok(png) = crate::images::encode_png(&image) else { return };
                    run_on_main(move || {
                        let app = crate::app::app();
                        app.history.borrow_mut().add_image(&image, &png, source.as_deref());
                        app.refresh_panel();
                    });
                });
            }
            _ => {
                app.watcher.borrow_mut().last = None;
                if let Some(panel) = app.panel() {
                    panel.set_hint(hint, false);
                }
            }
        }
    }
}
