//! 钉在桌面：截图浮在所有窗口上面，方便对照。拖动移动，滚轮缩放（以鼠标为中心，20%～500%），
//! 双击或 Esc 关掉，Ctrl+C 复制，右键有复制、识别文字、原始大小、关闭。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{self, FrameTimer, Spring};
use crate::app::app;
use crate::gfx::{self, Color, Rect, Surface};
use crate::images::Image;
use crate::menu;
use crate::text::{Font, TextOpts};
use crate::theme;
use crate::window::{self, Handler};

struct State {
    image: Image,
    bitmap: Option<ID2D1Bitmap>,
    surface: Surface,
    /// 缩放比例（相对截图时的大小）
    zoom: f32,
    /// 截图时屏幕的缩放（像素/DIP）
    capture_scale: f32,
    zoom_badge: Spring,
    timer: FrameTimer,
}

pub struct Pin {
    hwnd: Cell<HWND>,
    st: RefCell<State>,
}

thread_local! {
    static PINS: RefCell<Vec<Rc<Pin>>> = const { RefCell::new(Vec::new()) };
}

const TIMER_BADGE: usize = 1;

/// 在屏幕像素位置 (x, y) 钉一张图
pub fn open(image: Image, x: i32, y: i32, capture_scale: f32) {
    let pin = Rc::new(Pin {
        hwnd: Cell::new(HWND::default()),
        st: RefCell::new(State {
            image,
            bitmap: None,
            surface: Surface::new(HWND::default()),
            zoom: 1.0,
            capture_scale,
            zoom_badge: Spring::with(0.0, 0.25, 1.0),
            timer: FrameTimer::new(),
        }),
    });
    let (w, h) = {
        let st = pin.st.borrow();
        (st.image.width as i32, st.image.height as i32)
    };
    let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP;
    let Ok(hwnd) = window::create("ClipMD 钉图", WS_POPUP | WS_THICKFRAME, ex, (x, y, w, h), None, pin.clone()) else { return };
    pin.hwnd.set(hwnd);
    pin.st.borrow_mut().surface = Surface::new(hwnd);
    window::style_window(hwnd, theme::is_dark(), window::Backdrop::None, true);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    // 屏幕缩放不同（截图在 150% 的屏幕，钉到 100% 的屏幕）时按 DIP 保持同样大小
    pin.apply_zoom(1.0, None);
    PINS.with(|p| p.borrow_mut().push(pin));
}

pub fn close_all() {
    let pins: Vec<Rc<Pin>> = PINS.with(|p| p.borrow_mut().drain(..).collect());
    for pin in pins {
        unsafe {
            let _ = DestroyWindow(pin.hwnd.get());
        }
    }
}

pub fn count() -> usize {
    PINS.with(|p| p.borrow().len())
}

pub fn hwnds() -> Vec<HWND> {
    PINS.with(|p| p.borrow().iter().map(|pin| pin.hwnd.get()).collect())
}

/// 自检：放大最新的那张
pub fn zoom_latest(factor: f32) {
    if let Some(pin) = PINS.with(|p| p.borrow().last().cloned()) {
        let zoom = pin.st.borrow().zoom * factor;
        pin.apply_zoom(zoom, None);
    }
}

impl Pin {
    /// 改缩放，`anchor` 是保持不动的屏幕点（鼠标位置）
    fn apply_zoom(&self, zoom: f32, anchor: Option<POINT>) {
        let hwnd = self.hwnd.get();
        let zoom = zoom.clamp(0.2, 5.0);
        let (w0, h0, capture_scale) = {
            let st = self.st.borrow();
            (st.image.width as f32, st.image.height as f32, st.capture_scale)
        };
        let s = window::scale(hwnd);
        let (nw, nh) = ((w0 / capture_scale * s * zoom).round() as i32, (h0 / capture_scale * s * zoom).round() as i32);
        let r = window::window_rect(hwnd);
        let (ow, oh) = ((r.right - r.left).max(1) as f32, (r.bottom - r.top).max(1) as f32);
        let a = anchor.unwrap_or(POINT { x: r.left, y: r.top });
        let fx = (a.x - r.left) as f32 / ow;
        let fy = (a.y - r.top) as f32 / oh;
        let nx = a.x - (fx * nw as f32) as i32;
        let ny = a.y - (fy * nh as f32) as i32;
        self.st.borrow_mut().zoom = zoom;
        unsafe {
            let _ = SetWindowPos(hwnd, None, nx, ny, nw.max(16), nh.max(16), SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn paint(&self) {
        let hwnd = self.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        let dpi = window::dpi(hwnd) as f32;
        let mut st = self.st.borrow_mut();
        if st.bitmap.is_none() {
            st.bitmap = gfx::bitmap_from_bgra(st.image.width, st.image.height, st.image.stride(), &st.image.pixels);
        }
        let bitmap = st.bitmap.clone();
        let zoom = st.zoom;
        let badge = st.zoom_badge.value;
        let mut surface = std::mem::replace(&mut st.surface, Surface::new(hwnd));
        drop(st);
        surface.render(w as u32, h as u32, dpi, theme::is_dark(), |p| {
            let s = dpi / 96.0;
            let full = Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s);
            if let Some(bitmap) = &bitmap {
                p.bitmap(bitmap, full, 1.0, zoom < 0.999 || zoom > 1.001);
            }
            p.stroke_rect(full.inset(0.5, 0.5), theme::BLUE.alpha(0.55), 1.0);
            if badge > 0.01 {
                let label = format!("{}%", (zoom * 100.0).round());
                let r = Rect::centered(full.mid_x(), full.mid_y(), 64.0, 28.0);
                p.with_alpha(badge, || {
                    p.fill_round(r, 14.0, Color::hex(0x111214).alpha(0.8));
                    p.text(&label, r, Font::ui(13.0, 600), Color::WHITE, TextOpts::CENTER);
                });
            }
        });
        self.st.borrow_mut().surface = surface;
    }

    fn copy(&self) {
        let image = self.st.borrow().image.clone();
        let png = crate::images::encode_png(&image).ok();
        crate::clipboard::write_image(app().hwnd.get(), &image, png.as_deref());
        crate::hud::show(crate::hud::Toast::success("已复制图片", "Ctrl+V 粘贴"));
    }

    fn recognize(&self) {
        let image = self.st.borrow().image.clone();
        let path = crate::util::data_dir().join("ocr-pin.png");
        if crate::images::write_png(&image, &path).is_ok() {
            app().recognize_file(path, "钉图识别");
        }
    }

    fn close(&self) {
        let hwnd = self.hwnd.get();
        self.st.borrow_mut().surface.release();
        PINS.with(|p| p.borrow_mut().retain(|pin| pin.hwnd.get() != hwnd));
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    fn show_badge(&self) {
        let hwnd = self.hwnd.get();
        self.st.borrow_mut().zoom_badge.set(1.0);
        anim::request_frame(hwnd);
        unsafe {
            SetTimer(Some(hwnd), TIMER_BADGE, 900, None);
        }
    }
}

impl Handler for Pin {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCALCSIZE => window::borderless_nccalcsize(wp),
            // 整张图都可以拖动
            WM_NCHITTEST => Some(LRESULT(HTCAPTION as isize)),
            WM_NCLBUTTONDBLCLK => {
                self.close();
                Some(LRESULT(0))
            }
            WM_NCRBUTTONUP => {
                let at = window::cursor_pos();
                let entries = vec![
                    menu::item(1, "复制"),
                    menu::item(2, "识别文字"),
                    menu::item(3, "原始大小"),
                    menu::Entry::Separator,
                    menu::item(4, "关闭"),
                ];
                match menu::popup(hwnd, &entries, at) {
                    Some(1) => self.copy(),
                    Some(2) => self.recognize(),
                    Some(3) => {
                        self.apply_zoom(1.0, None);
                        self.show_badge();
                    }
                    Some(4) => self.close(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_SYSCOMMAND if (wp.0 & 0xFFF0) == SC_KEYMENU as usize => Some(LRESULT(0)),
            WM_MOUSEWHEEL => {
                let delta = crate::util::wheel_delta(wp) as f32 / 120.0;
                let zoom = self.st.borrow().zoom * 1.1f32.powf(delta);
                let anchor = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
                self.apply_zoom(zoom, Some(anchor));
                self.show_badge();
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                match wp.0 as u16 {
                    0x1B => self.close(),
                    0x43 if ctrl => self.copy(),
                    0x57 if ctrl => self.close(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                info.ptMinTrackSize = POINT { x: 16, y: 16 };
                Some(LRESULT(0))
            }
            WM_SIZE => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_PAINT => {
                self.paint();
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            anim::WM_FRAME => {
                let moving = {
                    let mut st = self.st.borrow_mut();
                    let dt = st.timer.tick();
                    st.zoom_badge.step(dt)
                };
                self.paint();
                if moving {
                    anim::request_frame(hwnd);
                } else {
                    self.st.borrow_mut().timer.reset();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_BADGE => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_BADGE);
                }
                self.st.borrow_mut().zoom_badge.set(0.0);
                anim::request_frame(hwnd);
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let zoom = self.st.borrow().zoom;
                self.apply_zoom(zoom, None);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
