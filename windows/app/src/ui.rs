//! 自绘界面的小工具：可点击区域、鼠标悬停的渐变高亮、常用控件（图标按钮、分段选择、开关）、提示气泡。
//!
//! 每次绘制时登记可点击的区域，鼠标事件按上一帧登记的区域判断点到了谁。

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{FrameTimer, Spring};
use crate::gfx::{Color, Painter, Rect, Surface};
use crate::icons::{self, Icon};
use crate::text::{self, Font, TextOpts};
use crate::theme::{self, Palette};
use crate::window::{self, Handler};

pub type Id = u64;

/// 由种类和序号组成区域编号
pub const fn id(kind: u32, index: u32) -> Id {
    ((kind as u64) << 32) | index as u64
}

pub fn kind_of(id: Id) -> u32 {
    (id >> 32) as u32
}

pub fn index_of(id: Id) -> u32 {
    (id & 0xFFFF_FFFF) as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    Hand,
    IBeam,
}

#[derive(Clone, Debug)]
pub struct Region {
    pub id: Id,
    pub rect: Rect,
    pub tip: Option<String>,
    pub cursor: Cursor,
}

#[derive(Default)]
pub struct Interaction {
    pub regions: Vec<Region>,
    pub hovered: Option<Id>,
    pub pressed: Option<Id>,
    pub mouse: Option<(f32, f32)>,
    hover: HashMap<Id, Spring>,
    hover_since: f64,
    tip_shown: bool,
}

impl Interaction {
    pub fn begin_frame(&mut self) {
        self.regions.clear();
    }

    pub fn add(&mut self, id: Id, rect: Rect) {
        self.regions.push(Region { id, rect, tip: None, cursor: Cursor::Arrow });
    }

    pub fn add_tip(&mut self, id: Id, rect: Rect, tip: &str) {
        self.regions.push(Region { id, rect, tip: Some(tip.to_string()), cursor: Cursor::Arrow });
    }

    pub fn add_cursor(&mut self, id: Id, rect: Rect, cursor: Cursor) {
        self.regions.push(Region { id, rect, tip: None, cursor });
    }

    /// 最上面（最后登记）的区域
    pub fn hit(&self, x: f32, y: f32) -> Option<&Region> {
        self.regions.iter().rev().find(|r| r.rect.contains(x, y))
    }

    pub fn hit_id(&self, x: f32, y: f32) -> Option<Id> {
        self.hit(x, y).map(|r| r.id)
    }

    /// 鼠标移动，返回悬停对象是否变了
    pub fn mouse_move(&mut self, x: f32, y: f32) -> bool {
        self.mouse = Some((x, y));
        let hovered = self.hit_id(x, y);
        if hovered != self.hovered {
            self.hovered = hovered;
            self.hover_since = crate::util::now_seconds();
            self.tip_shown = false;
            if let Some(id) = hovered {
                self.hover.entry(id).or_insert_with(|| Spring::with(0.0, 0.18, 1.0)).set(1.0);
            }
            for (key, spring) in self.hover.iter_mut() {
                if Some(*key) != hovered {
                    spring.set(0.0);
                }
            }
            return true;
        }
        false
    }

    pub fn mouse_leave(&mut self) {
        self.mouse = None;
        self.hovered = None;
        self.pressed = None;
        self.tip_shown = false;
        for spring in self.hover.values_mut() {
            spring.set(0.0);
        }
    }

    pub fn mouse_down(&mut self, x: f32, y: f32) -> Option<Id> {
        self.pressed = self.hit_id(x, y);
        self.pressed
    }

    /// 松开鼠标：在按下的同一个区域里松开才算点击
    pub fn mouse_up(&mut self, x: f32, y: f32) -> Option<Id> {
        let pressed = self.pressed.take()?;
        (self.hit_id(x, y) == Some(pressed)).then_some(pressed)
    }

    /// 悬停高亮的程度 0～1（带渐变）
    pub fn hover(&self, id: Id) -> f32 {
        self.hover.get(&id).map(|s| s.value).unwrap_or(0.0)
    }

    pub fn is_pressed(&self, id: Id) -> bool {
        self.pressed == Some(id) && self.hovered == Some(id)
    }

    /// 推进悬停动画，返回是否还在动
    pub fn step(&mut self, dt: f32) -> bool {
        let mut moving = false;
        self.hover.retain(|_, s| {
            moving |= s.step(dt);
            s.value > 0.0005 || s.target > 0.0
        });
        moving
    }

    pub fn cursor(&self) -> Cursor {
        self.hovered.and_then(|h| self.regions.iter().rev().find(|r| r.id == h)).map(|r| r.cursor).unwrap_or(Cursor::Arrow)
    }

    /// 悬停够久了该显示的提示：(文字, 区域)
    pub fn pending_tip(&mut self) -> Option<(String, Rect)> {
        if self.tip_shown || self.pressed.is_some() {
            return None;
        }
        let hovered = self.hovered?;
        if crate::util::now_seconds() - self.hover_since < 0.55 {
            return None;
        }
        let region = self.regions.iter().rev().find(|r| r.id == hovered)?;
        let tip = region.tip.clone()?;
        self.tip_shown = true;
        Some((tip, region.rect))
    }

    pub fn wants_tip_timer(&self) -> bool {
        !self.tip_shown && self.hovered.is_some_and(|h| self.regions.iter().any(|r| r.id == h && r.tip.is_some()))
    }
}

pub fn apply_cursor(cursor: Cursor) {
    window::set_cursor(match cursor {
        Cursor::Arrow => IDC_ARROW,
        Cursor::Hand => IDC_HAND,
        Cursor::IBeam => IDC_IBEAM,
    });
}

// MARK: 常用控件

/// 圆形悬停底的图标按钮
pub fn icon_button(p: &Painter, ia: &mut Interaction, pal: &Palette, id: Id, rect: Rect, icon: Icon, tip: &str, base: Color) {
    ia.add_tip(id, rect, tip);
    let hover = ia.hover(id);
    let pressed = ia.is_pressed(id);
    if hover > 0.0 {
        p.fill_round(rect, rect.h / 2.0, if pressed { pal.pressed } else { pal.hover.alpha(hover * 1.4) });
    }
    let color = base.mix(pal.text, hover);
    icons::draw(p, icon, rect, (rect.h * 0.55).min(16.0), color);
}

/// 文字按钮（"清空"、"撤销"这类）
pub fn text_button(p: &Painter, ia: &mut Interaction, pal: &Palette, id: Id, rect: Rect, label: &str, font: Font, color: Color) {
    ia.add_cursor(id, rect, Cursor::Hand);
    let hover = ia.hover(id);
    if hover > 0.0 {
        p.fill_round(rect, 8.0, pal.hover.alpha(hover * 1.3));
    }
    p.text(label, rect, font, color, TextOpts::CENTER);
}

/// 胶囊形的按钮（设置里的"更改…"、预览窗口的按钮）
pub fn pill_button(p: &Painter, ia: &mut Interaction, pal: &Palette, id: Id, rect: Rect, label: &str, prominent: bool) {
    ia.add(id, rect);
    let hover = ia.hover(id);
    let pressed = ia.is_pressed(id);
    if prominent {
        let (top, bottom) = (theme::BLUE_LIGHT, theme::BLUE);
        let k = if pressed { 0.85 } else { 1.0 + hover * 0.06 };
        let adj = |c: Color| Color { r: (c.r * k).min(1.0), g: (c.g * k).min(1.0), b: (c.b * k).min(1.0), a: c.a };
        p.gradient_round(rect, rect.h / 2.0, adj(top), adj(bottom));
        p.text(label, rect, Font::ui(13.0, 600), Color::WHITE, TextOpts::CENTER);
    } else {
        p.fill_round(rect, rect.h / 2.0, if pressed { pal.pressed } else { pal.fill_strong.mix(pal.pressed, hover) });
        p.stroke_round(rect.inset(0.5, 0.5), rect.h / 2.0, pal.border, 1.0);
        p.text(label, rect, Font::ui(13.0, 500), pal.text, TextOpts::CENTER);
    }
}

/// 分段选择（"预览 | 编辑 | 并排"）。`slide` 是选中块的位置（0..n-1，可以是小数，用来做滑动动画）
pub fn segmented(p: &Painter, ia: &mut Interaction, pal: &Palette, base_id: u32, rect: Rect, labels: &[&str], slide: f32, font: Font) {
    p.fill_round(rect, rect.h / 2.0, pal.fill);
    let n = labels.len().max(1) as f32;
    let seg_w = rect.w / n;
    let knob = Rect::new(rect.x + 2.0 + slide * seg_w, rect.y + 2.0, seg_w - 4.0, rect.h - 4.0);
    if !pal.dark {
        p.soft_shadow(knob, knob.h / 2.0, Color::BLACK.alpha(0.12), 2.0, 0.5);
    }
    p.fill_round(knob, knob.h / 2.0, pal.tab_selected);
    for (i, label) in labels.iter().enumerate() {
        let r = Rect::new(rect.x + i as f32 * seg_w, rect.y, seg_w, rect.h);
        ia.add(id(base_id, i as u32), r);
        let selected = (slide - i as f32).abs() < 0.5;
        let color = if selected { pal.text } else { pal.secondary.mix(pal.text, ia.hover(id(base_id, i as u32)) * 0.5) };
        p.text(label, r, Font { weight: if selected { 600 } else { font.weight }, ..font }, color, TextOpts::CENTER);
    }
}

/// 开关。`on` 是 0～1（动画中间值）
pub fn toggle(p: &Painter, ia: &mut Interaction, pal: &Palette, id: Id, rect: Rect, on: f32) {
    ia.add(id, rect);
    let track = Rect::centered(rect.mid_x(), rect.mid_y(), 40.0, 22.0);
    let off_color = if pal.dark { Color::WHITE.alpha(0.16) } else { Color::BLACK.alpha(0.14) };
    p.fill_round(track, 11.0, off_color.mix(theme::BLUE, on));
    let knob_x = track.x + 11.0 + on * 18.0;
    p.fill_circle(knob_x, track.mid_y() + 0.5, 9.0, Color::BLACK.alpha(0.12));
    p.fill_circle(knob_x, track.mid_y(), 8.5, Color::WHITE);
}

// MARK: 提示气泡

struct TooltipState {
    text: String,
    surface: Surface,
}

pub struct Tooltip {
    hwnd: std::cell::Cell<HWND>,
    state: RefCell<TooltipState>,
}

thread_local! {
    static TOOLTIP: RefCell<Option<Rc<Tooltip>>> = const { RefCell::new(None) };
}

impl Handler for Tooltip {
    fn handle(&self, hwnd: HWND, msg: u32, _wp: WPARAM, _lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCHITTEST => Some(LRESULT(HTTRANSPARENT as isize)),
            WM_PAINT => {
                self.paint(hwnd);
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

impl Tooltip {
    fn paint(&self, hwnd: HWND) {
        let mut state = self.state.borrow_mut();
        let (w, h) = window::client_size(hwnd);
        let dpi = window::dpi(hwnd) as f32;
        let dark = theme::is_dark();
        let pal = theme::palette();
        let text = state.text.clone();
        state.surface.render(w as u32, h as u32, dpi, dark, |p| {
            let s = dpi / 96.0;
            let rect = Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s).inset(0.5, 0.5);
            p.fill_round(rect, 6.0, if dark { Color::hex(0x2B2C30) } else { Color::hex(0xFBFBFC) });
            p.stroke_round(rect, 6.0, pal.border, 1.0);
            p.text(&text, rect.inset(8.0, 0.0), Font::ui(12.0, 400), pal.text, TextOpts { ellipsis: false, ..TextOpts::WRAP_CENTER_V });
        });
    }
}

impl TextOpts {
    pub const WRAP_CENTER_V: TextOpts = TextOpts { align: text::Align::Leading, vcenter: true, wrap: true, ellipsis: false };
}

/// 在 `anchor`（屏幕像素矩形）下方显示提示
pub fn show_tooltip(text: &str, anchor: RECT) {
    hide_tooltip();
    let center = POINT { x: (anchor.left + anchor.right) / 2, y: anchor.bottom };
    let scale = window::monitor_dpi_at(center) as f32 / 96.0;
    let max_w = 260.0;
    let content_w = text::measure(text, Font::ui(12.0, 400)).min(max_w);
    let content_h = text::measure_wrapped(text, Font::ui(12.0, 400), content_w + 1.0);
    let (w, h) = (((content_w + 18.0) * scale) as i32, ((content_h + 10.0).max(24.0) * scale) as i32);
    let (work, _) = window::monitor_rects(center);
    let mut x = center.x - w / 2;
    let mut y = anchor.bottom + (6.0 * scale) as i32;
    if y + h > work.bottom {
        y = anchor.top - h - (6.0 * scale) as i32;
    }
    x = x.clamp(work.left + 4, (work.right - w - 4).max(work.left));
    y = y.max(work.top);
    let tooltip = Rc::new(Tooltip {
        hwnd: std::cell::Cell::new(HWND::default()),
        state: RefCell::new(TooltipState { text: text.to_string(), surface: Surface::new(HWND::default()) }),
    });
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP;
    if let Ok(hwnd) = window::create("", WS_POPUP, ex, (x, y, w, h), None, tooltip.clone()) {
        tooltip.hwnd.set(hwnd);
        tooltip.state.borrow_mut().surface = Surface::new(hwnd);
        window::style_window(hwnd, theme::is_dark(), window::Backdrop::None, true);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        TOOLTIP.with(|t| *t.borrow_mut() = Some(tooltip));
    }
}

pub fn hide_tooltip() {
    if let Some(tip) = TOOLTIP.with(|t| t.borrow_mut().take()) {
        unsafe {
            let _ = DestroyWindow(tip.hwnd.get());
        }
    }
}

/// 自绘窗口的公共部分：交换链、互动状态、帧计时
pub struct Canvas {
    pub surface: Surface,
    pub ia: Interaction,
    pub timer: FrameTimer,
}

impl Canvas {
    pub fn new(hwnd: HWND) -> Canvas {
        Canvas { surface: Surface::new(hwnd), ia: Interaction::default(), timer: FrameTimer::new() }
    }
}

/// 客户区的 DIP 矩形转成屏幕像素矩形（提示气泡定位用）
pub fn dip_rect_to_screen(hwnd: HWND, rect: Rect) -> RECT {
    let s = window::scale(hwnd);
    let mut tl = POINT { x: (rect.x * s) as i32, y: (rect.y * s) as i32 };
    let mut br = POINT { x: (rect.right() * s) as i32, y: (rect.bottom() * s) as i32 };
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut tl);
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut br);
    }
    RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y }
}
