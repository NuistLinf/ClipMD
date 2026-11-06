//! 剪贴板记录的预览：在悬浮窗旁边弹出。文字可以切换「排版」（按 Markdown 显示）和「原文」，图片看大图。
//! 底部有"放回剪贴板""存成 md / 识别文字""关闭"。点外面、按 Esc 就关。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use clipmd_core::clip_history::ClipItem;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{self, FrameTimer, Spring};
use crate::app::app;
use crate::gfx::{self, Rect, Surface};
use crate::markdown_view::DocView;
use crate::text::{self, Font, TextOpts};
use crate::theme;
use crate::ui::{self, Interaction};
use crate::window::{self, Handler};

const WIDTH: f32 = 440.0;
const HEADER: f32 = 50.0;
const FOOTER: f32 = 60.0;

const K_MODE: u32 = 1;
const K_COPY: u32 = 2;
const K_SAVE: u32 = 3;
const K_CLOSE: u32 = 4;

struct State {
    item: ClipItem,
    formatted: bool,
    mode_spring: Spring,
    doc: Option<DocView>,
    bitmap: Option<ID2D1Bitmap>,
    image_size: (u32, u32),
    surface: Surface,
    ia: Interaction,
    scroll: Spring,
    timer: FrameTimer,
    appear: Spring,
    content_h: f32,
}

pub struct ClipPreview {
    hwnd: Cell<HWND>,
    st: RefCell<State>,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<ClipPreview>>> = const { RefCell::new(None) };
}

pub fn toggle(id: &str) {
    let open_id = CURRENT.with(|c| c.borrow().as_ref().map(|p| p.st.borrow().item.id.clone()));
    close();
    if open_id.as_deref() == Some(id) {
        return;
    }
    let Some(item) = app().history.borrow().find(id).cloned() else { return };
    show(item);
}

pub fn close() {
    if let Some(p) = CURRENT.with(|c| c.borrow_mut().take()) {
        p.st.borrow_mut().surface.release();
        unsafe {
            let _ = DestroyWindow(p.hwnd.get());
        }
    }
}

pub fn restyle() {
    if let Some(p) = CURRENT.with(|c| c.borrow().clone()) {
        window::style_window(p.hwnd.get(), theme::is_dark(), window::Backdrop::Acrylic, true);
        window::invalidate(p.hwnd.get());
    }
}

pub fn hwnd() -> Option<HWND> {
    CURRENT.with(|c| c.borrow().as_ref().map(|p| p.hwnd.get()))
}

/// 自检：切换排版和原文
pub fn set_formatted(formatted: bool) {
    if let Some(p) = CURRENT.with(|c| c.borrow().clone()) {
        p.set_mode(formatted);
    }
}

fn show(item: ClipItem) {
    let mut doc = None;
    let mut bitmap = None;
    let mut image_size = (0, 0);
    let mut content_h;
    let inner_w = WIDTH - 40.0;
    if item.is_image() {
        let path = crate::history::images_dir().join(item.image_file.as_deref().unwrap_or_default());
        if let Ok(image) = crate::images::decode_file(&path, Some(1400)) {
            bitmap = gfx::bitmap_from_bgra(image.width, image.height, image.stride(), &image.pixels);
        }
        image_size = (item.image_width.unwrap_or(1), item.image_height.unwrap_or(1));
        let ratio = image_size.1 as f32 / image_size.0.max(1) as f32;
        content_h = (inner_w * ratio).min(420.0) + 24.0;
    } else {
        let mut d = DocView::new(&item.text, 14.0);
        d.layout(inner_w);
        content_h = d.height + 32.0;
        doc = Some(d);
    }
    content_h = content_h.clamp(120.0, 460.0);
    let height = HEADER + content_h + FOOTER;
    let preview = Rc::new(ClipPreview {
        hwnd: Cell::new(HWND::default()),
        st: RefCell::new(State {
            item,
            formatted: true,
            mode_spring: Spring::with(0.0, 0.3, 0.9),
            doc,
            bitmap,
            image_size,
            surface: Surface::new(HWND::default()),
            ia: Interaction::default(),
            scroll: Spring::with(0.0, 0.22, 1.0),
            timer: FrameTimer::new(),
            appear: Spring::with(0.0, 0.28, 0.86),
            content_h: 0.0,
        }),
    });
    // 放在悬浮窗旁边：左边放得下就放左边
    let panel_rect = app().panel().map(|p| window::window_rect(p.hwnd())).unwrap_or_default();
    let (work, _) = window::monitor_rects(POINT { x: panel_rect.left + 10, y: panel_rect.top + 10 });
    let scale = window::monitor_dpi_at(POINT { x: panel_rect.left + 10, y: panel_rect.top + 10 }) as f32 / 96.0;
    let (w, h) = ((WIDTH * scale) as i32, (height * scale) as i32);
    let gap = (10.0 * scale) as i32;
    let x = if panel_rect.left - gap - w >= work.left { panel_rect.left - gap - w } else { (panel_rect.right + gap).min(work.right - w) };
    let y = panel_rect.top.clamp(work.top, (work.bottom - h).max(work.top));
    let style = WS_POPUP | WS_THICKFRAME;
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP;
    let Ok(hwnd) = window::create("ClipMD 预览", style, ex, (x, y, w, h), None, preview.clone()) else { return };
    preview.hwnd.set(hwnd);
    preview.st.borrow_mut().surface = Surface::new(hwnd);
    window::style_window(hwnd, theme::is_dark(), window::Backdrop::Acrylic, true);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    window::force_foreground(hwnd);
    preview.st.borrow_mut().appear.set(1.0);
    anim::request_frame(hwnd);
    CURRENT.with(|c| *c.borrow_mut() = Some(preview));
}

impl ClipPreview {
    fn size(&self) -> Rect {
        let hwnd = self.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        let s = window::scale(hwnd);
        Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s)
    }

    fn set_mode(&self, formatted: bool) {
        let mut st = self.st.borrow_mut();
        st.formatted = formatted;
        st.mode_spring.set(if formatted { 0.0 } else { 1.0 });
        st.scroll.snap(0.0);
        drop(st);
        anim::request_frame(self.hwnd.get());
    }

    fn paint(&self) {
        let hwnd = self.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        if w <= 0 || h <= 0 {
            return;
        }
        let dpi = window::dpi(hwnd) as f32;
        let dark = theme::is_dark();
        let pal = theme::palette();
        let size = self.size();
        let backdrop = window::backdrop_supported();
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let mut surface = std::mem::replace(&mut st.surface, Surface::new(hwnd));
        surface.render(w as u32, h as u32, dpi, dark, |p| {
            p.fill_rect(size, if backdrop { pal.tint } else { pal.solid });
            let appear = st.appear.value.clamp(0.0, 1.0);
            p.with_alpha(appear, || {
                st.ia.begin_frame();
                let title = st.item.title();
                let is_image = st.item.is_image();
                let title_w = if is_image { size.w - 40.0 } else { size.w - 40.0 - 150.0 };
                p.text(if title.is_empty() { "（空白）" } else { &title }, Rect::new(20.0, 0.0, title_w, HEADER), Font::ui(14.0, 600), pal.text, TextOpts::LINE);
                if !is_image {
                    ui::segmented(p, &mut st.ia, &pal, K_MODE, Rect::new(size.w - 20.0 - 140.0, 11.0, 140.0, 28.0), &["排版", "原文"], st.mode_spring.value, Font::ui(12.5, 500));
                }
                p.fill_rect(Rect::new(0.0, HEADER - 1.0, size.w, 1.0), pal.separator);
                // 内容
                let body = Rect::new(0.0, HEADER, size.w, size.h - HEADER - FOOTER);
                let scroll = st.scroll.value;
                p.clip(body, || {
                    if let Some(bitmap) = &st.bitmap {
                        let (iw, ih) = (st.image_size.0 as f32, st.image_size.1 as f32);
                        let fit = ((body.w - 40.0) / iw).min((body.h - 24.0) / ih).min(1.0);
                        let (dw, dh) = (iw * fit, ih * fit);
                        let dest = Rect::centered(body.mid_x(), body.mid_y(), dw, dh);
                        p.fill_rect(dest.inset(-1.0, -1.0), pal.border);
                        p.bitmap(bitmap, dest, 1.0, true);
                        st.content_h = body.h;
                    } else if st.formatted {
                        if let Some(doc) = &mut st.doc {
                            doc.layout(body.w - 40.0);
                            st.content_h = doc.height + 32.0;
                            doc.draw(p, &pal, (20.0, body.y + 16.0 - scroll), body);
                        }
                    } else {
                        let font = Font::ui(13.5, 400);
                        if let Some(l) = text::layout(&st.item.text, font, body.w - 40.0, 1_000_000.0, TextOpts::WRAP) {
                            st.content_h = l.height + 32.0;
                            p.draw_layout(&l.layout, 20.0, body.y + 16.0 - scroll, pal.text);
                        }
                    }
                });
                // 底部
                p.fill_rect(Rect::new(0.0, size.h - FOOTER, size.w, 1.0), pal.separator);
                let meta = if is_image {
                    format!("{} × {}", st.image_size.0, st.image_size.1)
                } else {
                    format!("{} 字", clipmd_core::text::group_thousands(st.item.character_count))
                };
                p.text(&meta, Rect::new(20.0, size.h - FOOTER, 120.0, FOOTER), Font::ui(12.0, 400), pal.secondary, TextOpts::LINE);
                let by = size.h - FOOTER + 14.0;
                let close = Rect::new(size.w - 20.0 - 64.0, by, 64.0, 32.0);
                let save = Rect::new(close.x - 8.0 - 96.0, by, 96.0, 32.0);
                let copy = Rect::new(save.x - 8.0 - 112.0, by, 112.0, 32.0);
                ui::pill_button(p, &mut st.ia, &pal, ui::id(K_CLOSE, 0), close, "关闭", false);
                ui::pill_button(p, &mut st.ia, &pal, ui::id(K_SAVE, 0), save, if is_image { "识别文字" } else { "存成 md" }, false);
                ui::pill_button(p, &mut st.ia, &pal, ui::id(K_COPY, 0), copy, "放回剪贴板", true);
            });
        });
        st.surface = surface;
    }

    fn frame(&self) {
        let moving = {
            let mut st = self.st.borrow_mut();
            let dt = st.timer.tick();
            st.appear.step(dt) | st.mode_spring.step(dt) | st.scroll.step(dt) | st.ia.step(dt)
        };
        self.paint();
        if moving {
            anim::request_frame(self.hwnd.get());
        } else {
            self.st.borrow_mut().timer.reset();
        }
    }

    fn click(&self, id: ui::Id) {
        let item_id = self.st.borrow().item.id.clone();
        match ui::kind_of(id) {
            K_MODE => self.set_mode(ui::index_of(id) == 0),
            K_COPY => {
                close();
                app().copy_clip(&item_id);
            }
            K_SAVE => {
                close();
                app().save_clip(&item_id);
            }
            K_CLOSE => close(),
            _ => {}
        }
    }

    fn copy_text(&self) {
        let st = self.st.borrow();
        if !st.item.is_image() {
            crate::clipboard::write_text(app().hwnd.get(), &st.item.text, st.item.html.as_deref());
            crate::hud::show(crate::hud::Toast::success("已复制全文", "Ctrl+V 粘贴"));
        }
    }
}

impl Handler for ClipPreview {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCALCSIZE => window::borderless_nccalcsize(wp),
            WM_NCHITTEST => {
                let mut p = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut p);
                }
                let s = window::scale(hwnd);
                let (x, y) = (p.x as f32 / s, p.y as f32 / s);
                let interactive = self.st.borrow().ia.hit_id(x, y).is_some();
                // 标题栏空白处可以拖
                if y < HEADER && !interactive { Some(LRESULT(HTCAPTION as isize)) } else { Some(LRESULT(HTCLIENT as isize)) }
            }
            WM_PAINT => {
                self.paint();
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            WM_SIZE => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                if crate::util::loword(wp.0) == 0 {
                    // 点了别的地方：关掉（推迟一下，别在激活消息里销毁窗口）
                    crate::app::run_on_main(|| {
                        let still_inactive = CURRENT.with(|c| c.borrow().as_ref().map(|p| unsafe { GetForegroundWindow() } != p.hwnd.get())).unwrap_or(false);
                        if still_inactive {
                            close();
                        }
                    });
                }
                None
            }
            WM_MOUSEMOVE => {
                window::track_mouse_leave(hwnd);
                let (x, y) = window::mouse_dip(hwnd, lp);
                if self.st.borrow_mut().ia.mouse_move(x, y) {
                    anim::request_frame(hwnd);
                }
                Some(LRESULT(0))
            }
            windows::Win32::UI::Controls::WM_MOUSELEAVE => {
                self.st.borrow_mut().ia.mouse_leave();
                anim::request_frame(hwnd);
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = window::mouse_dip(hwnd, lp);
                self.st.borrow_mut().ia.mouse_down(x, y);
                unsafe {
                    SetCapture(hwnd);
                }
                anim::request_frame(hwnd);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let (x, y) = window::mouse_dip(hwnd, lp);
                let clicked = self.st.borrow_mut().ia.mouse_up(x, y);
                if let Some(id) = clicked {
                    self.click(id);
                } else {
                    anim::request_frame(hwnd);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = crate::util::wheel_delta(wp) as f32;
                let body_h = self.size().h - HEADER - FOOTER;
                let mut st = self.st.borrow_mut();
                let max = (st.content_h - body_h).max(0.0);
                let target = (st.scroll.target - delta / 120.0 * 60.0).clamp(0.0, max);
                st.scroll.set(target);
                drop(st);
                anim::request_frame(hwnd);
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                match wp.0 as u16 {
                    0x1B => close(),
                    0x57 if ctrl => close(),
                    0x43 | 0x41 if ctrl => self.copy_text(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            anim::WM_FRAME => {
                self.frame();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

