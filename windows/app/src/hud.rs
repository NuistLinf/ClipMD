//! 屏幕下方的小提示（"已保存为 Markdown"、"正在识别文字…"），淡入、停留一会儿、淡出。
//! 窗口整个是透明的，卡片和阴影都自己画，所以淡入淡出时阴影也跟着一起变。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{self, FrameTimer, Spring};
use crate::gfx::{Color, Rect, Surface};
use crate::icons::{self, Icon};
use crate::text::{self, Font, TextOpts};
use crate::theme;
use crate::window::{self, Handler};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Success,
    Warning,
    Working,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: Kind,
    pub title: String,
    pub detail: String,
}

impl Toast {
    pub fn success(title: &str, detail: &str) -> Toast {
        Toast { kind: Kind::Success, title: title.into(), detail: detail.into() }
    }

    pub fn warning(title: &str, detail: &str) -> Toast {
        Toast { kind: Kind::Warning, title: title.into(), detail: detail.into() }
    }

    pub fn working(title: &str, detail: &str) -> Toast {
        Toast { kind: Kind::Working, title: title.into(), detail: detail.into() }
    }
}

const MARGIN: f32 = 18.0;
const CARD_H: f32 = 58.0;
const TIMER_HIDE: usize = 1;
const TIMER_DELAYED: usize = 2;

struct State {
    toast: Toast,
    surface: Surface,
    appear: Spring,
    timer: FrameTimer,
    spin: f32,
    closing: bool,
}

pub struct Hud {
    hwnd: Cell<HWND>,
    state: RefCell<State>,
}

thread_local! {
    static HUD: RefCell<Option<Rc<Hud>>> = const { RefCell::new(None) };
    static DELAYED: RefCell<Option<Toast>> = const { RefCell::new(None) };
    static GENERATION: Cell<u32> = const { Cell::new(0) };
}

/// 显示提示（替换正在显示的）
pub fn show(toast: Toast) {
    GENERATION.with(|g| g.set(g.get() + 1));
    DELAYED.with(|d| *d.borrow_mut() = None);
    crate::log!("hud: {} · {}", toast.title, toast.detail);
    let existing = HUD.with(|h| h.borrow().clone());
    if let Some(hud) = existing {
        hud.replace(toast);
        return;
    }
    if let Some(hud) = Hud::create(toast) {
        HUD.with(|h| *h.borrow_mut() = Some(hud));
    }
}

/// 过一会儿再显示：如果这期间已经有别的提示（比如识别很快就完成了），就不显示
pub fn show_after(delay: f64, toast: Toast) {
    let generation = GENERATION.with(|g| g.get());
    DELAYED.with(|d| *d.borrow_mut() = Some(toast));
    let target = crate::app::app().hwnd.get().0 as isize;
    let _ = target;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs_f64(delay));
        crate::app::run_on_main(move || {
            if GENERATION.with(|g| g.get()) == generation {
                if let Some(toast) = DELAYED.with(|d| d.borrow_mut().take()) {
                    show(toast);
                }
            }
        });
    });
}

fn card_width(toast: &Toast) -> f32 {
    let title = text::measure(&toast.title, Font::ui(14.0, 600));
    let detail = text::measure(&toast.detail, Font::ui(12.0, 400));
    (title.max(detail) + 74.0).clamp(240.0, 460.0)
}

impl Hud {
    fn create(toast: Toast) -> Option<Rc<Hud>> {
        let hud = Rc::new(Hud {
            hwnd: Cell::new(HWND::default()),
            state: RefCell::new(State {
                toast,
                surface: Surface::new(HWND::default()),
                appear: Spring::with(0.0, 0.3, 0.9),
                timer: FrameTimer::new(),
                spin: 0.0,
                closing: false,
            }),
        });
        let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP;
        let hwnd = window::create("ClipMD 提示", WS_POPUP, ex, (0, 0, 10, 10), None, hud.clone()).ok()?;
        hud.hwnd.set(hwnd);
        hud.state.borrow_mut().surface = Surface::new(hwnd);
        hud.place();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        hud.state.borrow_mut().appear.set(1.0);
        hud.schedule_hide();
        anim::request_frame(hwnd);
        Some(hud)
    }

    /// 放在鼠标所在屏幕的下方中间
    fn place(&self) {
        let hwnd = self.hwnd.get();
        let cursor = window::cursor_pos();
        let (work, _) = window::monitor_rects(cursor);
        let scale = window::monitor_dpi_at(cursor) as f32 / 96.0;
        let width = card_width(&self.state.borrow().toast) + MARGIN * 2.0;
        let height = CARD_H + MARGIN * 2.0;
        let (w, h) = ((width * scale) as i32, (height * scale) as i32);
        let x = (work.left + work.right) / 2 - w / 2;
        let y = work.bottom - h - (60.0 * scale) as i32;
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        }
    }

    fn replace(&self, toast: Toast) {
        {
            let mut state = self.state.borrow_mut();
            state.toast = toast;
            state.closing = false;
            state.appear.set(1.0);
        }
        self.place();
        self.schedule_hide();
        window::invalidate(self.hwnd.get());
        anim::request_frame(self.hwnd.get());
    }

    fn schedule_hide(&self) {
        let kind = self.state.borrow().toast.kind;
        let ms = match kind {
            Kind::Working => 60_000,
            Kind::Warning => 3200,
            Kind::Success => 2200,
        };
        unsafe {
            SetTimer(Some(self.hwnd.get()), TIMER_HIDE, ms, None);
        }
    }

    fn start_closing(&self) {
        self.state.borrow_mut().closing = true;
        self.state.borrow_mut().appear.set(0.0);
        anim::request_frame(self.hwnd.get());
    }

    fn paint(&self) {
        let hwnd = self.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        let dpi = window::dpi(hwnd) as f32;
        let dark = theme::is_dark();
        let pal = theme::palette();
        let mut state = self.state.borrow_mut();
        let toast = state.toast.clone();
        let appear = state.appear.value.clamp(0.0, 1.0);
        let spin = state.spin;
        state.surface.render(w as u32, h as u32, dpi, dark, |p| {
            let s = dpi / 96.0;
            let full = Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s);
            let card = full.inset(MARGIN, MARGIN).offset(0.0, (1.0 - appear) * 10.0);
            p.with_alpha(appear, || {
                p.soft_shadow(card, 14.0, Color::BLACK.alpha(if dark { 0.5 } else { 0.22 }), 12.0, 4.0);
                p.fill_round(card, 14.0, if dark { Color::hex(0x2A2B2F) } else { Color::hex(0xFDFDFE) });
                p.stroke_round(card.inset(0.5, 0.5), 14.0, pal.border, 1.0);
                let icon_rect = Rect::new(card.x + 14.0, card.mid_y() - 15.0, 30.0, 30.0);
                match toast.kind {
                    Kind::Success => {
                        p.fill_circle(icon_rect.mid_x(), icon_rect.mid_y(), 15.0, theme::GREEN.alpha(0.16));
                        icons::draw(p, Icon::Check, icon_rect, 16.0, theme::GREEN);
                    }
                    Kind::Warning => {
                        p.fill_circle(icon_rect.mid_x(), icon_rect.mid_y(), 15.0, Color::hex(0xF5A524).alpha(0.18));
                        p.text("!", icon_rect, Font::ui(17.0, 700), Color::hex(0xE08A00), TextOpts::CENTER);
                    }
                    Kind::Working => {
                        // 转圈：一段圆弧
                        let (cx, cy, r) = (icon_rect.mid_x(), icon_rect.mid_y(), 9.0);
                        p.stroke_ellipse(cx, cy, r, r, pal.fill_strong, 2.5);
                        let pts: Vec<(f32, f32)> = (0..=16)
                            .map(|i| {
                                let a = spin + i as f32 / 16.0 * 1.6;
                                (cx + r * a.cos(), cy + r * a.sin())
                            })
                            .collect();
                        p.polyline(&pts, theme::BLUE, 2.5, false);
                    }
                }
                let text_x = card.x + 54.0;
                let text_w = card.right() - text_x - 14.0;
                p.text(&toast.title, Rect::new(text_x, card.y + 10.0, text_w, 20.0), Font::ui(14.0, 600), pal.text, TextOpts::LINE);
                p.text(&toast.detail, Rect::new(text_x, card.y + 30.0, text_w, 18.0), Font::ui(12.0, 400), pal.secondary, TextOpts::LINE);
            });
        });
    }

    fn frame(&self) {
        let (moving, closed) = {
            let mut state = self.state.borrow_mut();
            let dt = state.timer.tick();
            let mut moving = state.appear.step(dt);
            if state.toast.kind == Kind::Working && !state.closing {
                state.spin += dt * 7.0;
                moving = true;
            }
            (moving, state.closing && !state.appear.is_moving() && state.appear.value <= 0.01)
        };
        if closed {
            unsafe {
                let _ = DestroyWindow(self.hwnd.get());
            }
            HUD.with(|h| *h.borrow_mut() = None);
            return;
        }
        self.paint();
        if moving {
            anim::request_frame(self.hwnd.get());
        } else {
            self.state.borrow_mut().timer.reset();
        }
    }
}

impl Handler for Hud {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, _lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => {
                self.paint();
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            anim::WM_FRAME => {
                self.frame();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_HIDE => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_HIDE);
                }
                self.start_closing();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_DELAYED => Some(LRESULT(0)),
            WM_NCHITTEST => Some(LRESULT(HTTRANSPARENT as isize)),
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            _ => None,
        }
    }
}
