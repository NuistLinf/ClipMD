//! 截图界面（Ctrl+Alt+A）。
//!
//! 按下快捷键时先把每块屏幕"冻住"（抓一张图），再在每块屏幕上盖一个全屏窗口画这张图：
//! - 选区：鼠标下的窗口自动选中，外面变暗；也可以拖一块。左上角实时显示分辨率
//! - 放大镜：鼠标旁边显示放大的像素、坐标和 #RRGGBB 色号，按 C 复制色号，Shift+C 复制 rgb(…)
//! - 选好后：拖动、拖边角调整；工具栏有矩形、椭圆、马赛克（7 种颜色、3 种粗细）、撤销、识别文字、钉在桌面、取消、完成
//! - Ctrl+Alt+O 是识别文字模式：框好区域马上识别，不进入标注
//!
//! 关掉截图界面后，冻住的整屏画面占的内存全部释放。

pub mod render;
mod screen;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub use render::{Mark, Tool};
pub use screen::grab_screen_rect;

use crate::app::{app, background, run_on_main};
use crate::gfx::{self, Color, Painter, Rect, Surface};
use crate::hud::{self, Toast};
use crate::icons::{self, Icon};
use crate::images::Image;
use crate::text::{Font, TextOpts};
use crate::theme;
use crate::ui::{self, Interaction};
use crate::window::{self, Handler};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Copy,
    Pin,
    Recognize,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Choosing,
    Pressing { at: (f32, f32) },
    Dragging { from: (f32, f32) },
    Editing,
    Moving { grab: (f32, f32), original: Rect },
    Resizing { handle: Handle, original: Rect },
    Drawing,
}

struct ScreenView {
    hwnd: Cell<HWND>,
    /// 屏幕像素
    rect: RECT,
    scale: f32,
    frozen: Image,
    bitmap: RefCell<Option<ID2D1Bitmap>>,
    mosaic: RefCell<Option<ID2D1Bitmap>>,
    surface: RefCell<Surface>,
    /// 这块屏幕上的窗口（DIP，从上到下），最后一个是整块屏幕
    windows: Vec<Rect>,
}

impl ScreenView {
    fn size(&self) -> Rect {
        Rect::new(0.0, 0.0, (self.rect.right - self.rect.left) as f32 / self.scale, (self.rect.bottom - self.rect.top) as f32 / self.scale)
    }

    fn bitmap(&self) -> Option<ID2D1Bitmap> {
        let mut slot = self.bitmap.borrow_mut();
        if slot.is_none() {
            *slot = gfx::bitmap_from_bgra(self.frozen.width, self.frozen.height, self.frozen.stride(), &self.frozen.pixels);
        }
        slot.clone()
    }

    fn mosaic(&self) -> Option<ID2D1Bitmap> {
        let mut slot = self.mosaic.borrow_mut();
        if slot.is_none() {
            let pix = render::pixelate(&self.frozen, (render::MOSAIC_BLOCK * self.scale).round() as u32);
            *slot = gfx::bitmap_from_bgra(pix.width, pix.height, pix.stride(), &pix.pixels);
        }
        slot.clone()
    }

    /// DIP → 冻住的图里的像素
    fn pixel_at(&self, x: f32, y: f32) -> Option<(u32, u32, [u8; 4])> {
        let (px, py) = ((x * self.scale).floor() as i64, (y * self.scale).floor() as i64);
        if px < 0 || py < 0 || px >= self.frozen.width as i64 || py >= self.frozen.height as i64 {
            return None;
        }
        let i = ((py as u32 * self.frozen.width + px as u32) * 4) as usize;
        let p = &self.frozen.pixels[i..i + 4];
        Some((px as u32, py as u32, [p[2], p[1], p[0], p[3]]))
    }
}

struct State {
    phase: Phase,
    active: usize,
    cursor: Option<(usize, f32, f32)>,
    hover_window: Option<Rect>,
    selection: Option<Rect>,
    tool: Option<Tool>,
    color: usize,
    width: usize,
    marks: Vec<Mark>,
    drawing: Option<Mark>,
    ia: Interaction,
    ocr: bool,
    hidden: bool,
}

pub struct Session {
    screens: Vec<ScreenView>,
    st: RefCell<State>,
}

thread_local! {
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

fn session() -> Option<Rc<Session>> {
    SESSION.with(|s| s.borrow().clone())
}

pub fn current() -> Option<Rc<Session>> {
    session()
}

pub fn is_active() -> bool {
    session().is_some()
}

/// 开始截图。`ocr` 为 true 时框好区域就识别文字
pub fn start(ocr: bool) {
    begin(ocr, false);
}

/// `hidden`：自检时不显示窗口，只在内存里跑
pub fn begin(ocr: bool, hidden: bool) -> Option<Rc<Session>> {
    if is_active() {
        return None;
    }
    ui::hide_tooltip();
    crate::clip_preview::close();
    let started = crate::util::now_seconds();
    let windows_px = screen::visible_windows();
    let mut screens = Vec::new();
    for monitor in screen::monitors() {
        let r = monitor.rect;
        let Some(frozen) = grab_screen_rect(r.left, r.top, r.right - r.left, r.bottom - r.top) else { continue };
        let scale = monitor.dpi as f32 / 96.0;
        let mut windows: Vec<Rect> = windows_px
            .iter()
            .filter_map(|w| {
                let (l, t, rr, b) = (w.left.max(r.left), w.top.max(r.top), w.right.min(r.right), w.bottom.min(r.bottom));
                if rr - l < 8 || b - t < 8 {
                    return None;
                }
                Some(Rect::new((l - r.left) as f32 / scale, (t - r.top) as f32 / scale, (rr - l) as f32 / scale, (b - t) as f32 / scale))
            })
            .collect();
        windows.push(Rect::new(0.0, 0.0, (r.right - r.left) as f32 / scale, (r.bottom - r.top) as f32 / scale));
        screens.push(ScreenView {
            hwnd: Cell::new(HWND::default()),
            rect: r,
            scale,
            frozen,
            bitmap: RefCell::new(None),
            mosaic: RefCell::new(None),
            surface: RefCell::new(Surface::new(HWND::default())),
            windows,
        });
    }
    if screens.is_empty() {
        hud::show(Toast::warning("截不了屏", "没有拿到屏幕画面"));
        return None;
    }
    let a = app();
    let (color, width) = {
        let s = a.settings.borrow();
        (s.capture_color.min(render::COLORS.len() - 1), s.capture_width.min(2))
    };
    let session = Rc::new(Session {
        screens,
        st: RefCell::new(State {
            phase: Phase::Choosing,
            active: 0,
            cursor: None,
            hover_window: None,
            selection: None,
            tool: None,
            color,
            width,
            marks: Vec::new(),
            drawing: None,
            ia: Interaction::default(),
            ocr,
            hidden,
        }),
    });
    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    if !hidden {
        let cursor = window::cursor_pos();
        for (index, view) in session.screens.iter().enumerate() {
            let r = view.rect;
            let overlay = Rc::new(Overlay { index });
            let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP;
            if let Ok(hwnd) = window::create("ClipMD 截图", WS_POPUP, ex, (r.left, r.top, r.right - r.left, r.bottom - r.top), None, overlay) {
                view.hwnd.set(hwnd);
                *view.surface.borrow_mut() = Surface::new(hwnd);
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
            }
            if cursor.x >= r.left && cursor.x < r.right && cursor.y >= r.top && cursor.y < r.bottom {
                let (x, y) = ((cursor.x - r.left) as f32 / view.scale, (cursor.y - r.top) as f32 / view.scale);
                session.mouse_move(index, x, y);
            }
        }
        // 鼠标所在屏幕的遮罩拿到键盘焦点
        let focus = session.st.borrow().cursor.map(|c| c.0).unwrap_or(0);
        let hwnd = session.screens[focus].hwnd.get();
        window::force_foreground(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
        for view in &session.screens {
            window::invalidate(view.hwnd.get());
        }
    }
    crate::log!("capture started ({} screens, {:.0} ms)", session.screens.len(), (crate::util::now_seconds() - started) * 1000.0);
    Some(session)
}

const TOOLBAR_H: f32 = 44.0;
const BUTTON: f32 = 34.0;
const SUBBAR_H: f32 = 38.0;

// 工具栏区域编号
const T_RECT: u32 = 1;
const T_ELLIPSE: u32 = 2;
const T_MOSAIC: u32 = 3;
const T_UNDO: u32 = 4;
const T_OCR: u32 = 5;
const T_PIN: u32 = 6;
const T_CANCEL: u32 = 7;
const T_DONE: u32 = 8;
const T_COLOR: u32 = 9;
const T_WIDTH: u32 = 10;

#[derive(Clone, Copy)]
enum Slot {
    Button(u32, Icon, &'static str),
    Gap,
}

const SLOTS: [Slot; 11] = [
    Slot::Button(T_RECT, Icon::Rectangle, "矩形（R）"),
    Slot::Button(T_ELLIPSE, Icon::Ellipse, "椭圆（O）"),
    Slot::Button(T_MOSAIC, Icon::Mosaic, "马赛克（M）"),
    Slot::Gap,
    Slot::Button(T_UNDO, Icon::Undo, "撤销（Ctrl+Z）"),
    Slot::Gap,
    Slot::Button(T_OCR, Icon::TextScan, "识别文字（T）"),
    Slot::Button(T_PIN, Icon::Pin, "钉在桌面（P）"),
    Slot::Gap,
    Slot::Button(T_CANCEL, Icon::Close, "取消（Esc）"),
    Slot::Button(T_DONE, Icon::Check, "完成，放进剪贴板（Enter）"),
];

fn toolbar_width() -> f32 {
    SLOTS.iter().map(|s| if matches!(s, Slot::Gap) { 11.0 } else { BUTTON + 2.0 }).sum::<f32>() + 10.0
}

impl Session {
    fn hwnds(&self) -> Vec<HWND> {
        self.screens.iter().map(|s| s.hwnd.get()).collect()
    }

    fn redraw(&self) {
        let hidden = self.st.borrow().hidden;
        if hidden {
            return;
        }
        for hwnd in self.hwnds() {
            window::invalidate(hwnd);
        }
    }

    fn line_width(&self, st: &State) -> f32 {
        if st.tool == Some(Tool::Mosaic) { render::BRUSH_WIDTHS[st.width] } else { render::LINE_WIDTHS[st.width] }
    }

    /// 工具栏的位置（选区下方，放不下就放上面，再不行放里面）
    fn toolbar_rect(&self, st: &State) -> Option<Rect> {
        let sel = st.selection?;
        let size = self.screens[st.active].size();
        let w = toolbar_width();
        let extra = if st.tool.is_some() { SUBBAR_H + 6.0 } else { 0.0 };
        let x = (sel.right() - w).clamp(8.0, (size.w - w - 8.0).max(8.0));
        let mut y = sel.bottom() + 10.0;
        if y + TOOLBAR_H + extra > size.h - 8.0 {
            y = sel.y - TOOLBAR_H - extra - 10.0;
            if y < 8.0 {
                y = sel.bottom() - TOOLBAR_H - extra - 10.0;
            }
        }
        Some(Rect::new(x, y, w, TOOLBAR_H))
    }

    fn handle_at(&self, st: &State, x: f32, y: f32) -> Option<Handle> {
        let sel = st.selection?;
        let near = |a: f32, b: f32| (a - b).abs() <= 7.0;
        let (l, t, r, b) = (sel.x, sel.y, sel.right(), sel.bottom());
        let inside_x = x > l + 7.0 && x < r - 7.0;
        let inside_y = y > t + 7.0 && y < b - 7.0;
        Some(match () {
            _ if near(x, l) && near(y, t) => Handle::TopLeft,
            _ if near(x, r) && near(y, t) => Handle::TopRight,
            _ if near(x, r) && near(y, b) => Handle::BottomRight,
            _ if near(x, l) && near(y, b) => Handle::BottomLeft,
            _ if near(y, t) && inside_x => Handle::Top,
            _ if near(y, b) && inside_x => Handle::Bottom,
            _ if near(x, l) && inside_y => Handle::Left,
            _ if near(x, r) && inside_y => Handle::Right,
            _ => return None,
        })
    }

    // MARK: 鼠标

    pub fn mouse_move(&self, index: usize, x: f32, y: f32) {
        let mut st = self.st.borrow_mut();
        let previous = st.cursor.map(|c| c.0);
        st.cursor = Some((index, x, y));
        match st.phase {
            Phase::Choosing => {
                st.active = index;
                st.hover_window = self.screens[index].windows.iter().find(|r| r.contains(x, y)).copied();
            }
            Phase::Pressing { at } => {
                if index == st.active && ((x - at.0).powi(2) + (y - at.1).powi(2)).sqrt() > 4.0 {
                    st.phase = Phase::Dragging { from: at };
                    st.marks.clear();
                }
            }
            _ => {}
        }
        let size = self.screens[st.active].size();
        let clamp = |v: f32, max: f32| v.clamp(0.0, max);
        match st.phase {
            Phase::Dragging { from } if index == st.active => {
                st.selection = Some(render::rect_from(from, (clamp(x, size.w), clamp(y, size.h))));
            }
            Phase::Moving { grab, original } => {
                let nx = (original.x + x - grab.0).clamp(0.0, (size.w - original.w).max(0.0));
                let ny = (original.y + y - grab.1).clamp(0.0, (size.h - original.h).max(0.0));
                st.selection = Some(Rect::new(nx, ny, original.w, original.h));
            }
            Phase::Resizing { handle, original } => {
                let (mut l, mut t, mut r, mut b) = (original.x, original.y, original.right(), original.bottom());
                let (x, y) = (clamp(x, size.w), clamp(y, size.h));
                match handle {
                    Handle::TopLeft => (l, t) = (x, y),
                    Handle::Top => t = y,
                    Handle::TopRight => (r, t) = (x, y),
                    Handle::Right => r = x,
                    Handle::BottomRight => (r, b) = (x, y),
                    Handle::Bottom => b = y,
                    Handle::BottomLeft => (l, b) = (x, y),
                    Handle::Left => l = x,
                }
                st.selection = Some(render::rect_from((l, t), (r, b)));
            }
            Phase::Drawing => {
                let sel = st.selection.unwrap_or_default();
                let p = (x.clamp(sel.x, sel.right()), y.clamp(sel.y, sel.bottom()));
                match &mut st.drawing {
                    Some(Mark::Rectangle { rect, .. }) | Some(Mark::Ellipse { rect, .. }) => {
                        // rect 的 x、y 存的是起点，w、h 存终点，画的时候再换算
                        rect.w = p.0;
                        rect.h = p.1;
                    }
                    Some(Mark::Mosaic { points, .. }) => {
                        if points.last().is_none_or(|l| (l.0 - p.0).abs() + (l.1 - p.1).abs() > 1.5) {
                            points.push(p);
                        }
                    }
                    None => {}
                }
            }
            Phase::Editing => {
                if index == st.active {
                    if st.ia.mouse_move(x, y) {
                        ui::hide_tooltip();
                    }
                }
            }
            _ => {}
        }
        drop(st);
        if let Some(prev) = previous.filter(|p| *p != index) {
            window::invalidate(self.screens[prev].hwnd.get());
        }
        window::invalidate(self.screens[index].hwnd.get());
    }

    pub fn mouse_down(&self, index: usize, x: f32, y: f32) {
        let mut st = self.st.borrow_mut();
        st.cursor = Some((index, x, y));
        match st.phase {
            Phase::Choosing => {
                st.active = index;
                st.phase = Phase::Pressing { at: (x, y) };
            }
            Phase::Editing if index == st.active => {
                if st.ia.mouse_down(x, y).is_some() {
                    // 点在工具栏上，松开时处理
                } else if let (Some(tool), Some(sel)) = (st.tool, st.selection) {
                    if sel.contains(x, y) {
                        let color = render::COLORS[st.color];
                        let width = self.line_width(&st);
                        st.drawing = Some(match tool {
                            Tool::Rectangle => Mark::Rectangle { rect: Rect::new(x, y, x, y), color, width },
                            Tool::Ellipse => Mark::Ellipse { rect: Rect::new(x, y, x, y), color, width },
                            Tool::Mosaic => Mark::Mosaic { points: vec![(x, y)], width },
                        });
                        st.phase = Phase::Drawing;
                    } else if st.marks.is_empty() {
                        st.selection = None;
                        st.tool = None;
                        st.phase = Phase::Pressing { at: (x, y) };
                    }
                } else if let Some(handle) = self.handle_at(&st, x, y) {
                    st.phase = Phase::Resizing { handle, original: st.selection.unwrap() };
                } else if st.selection.is_some_and(|s| s.contains(x, y)) {
                    st.phase = Phase::Moving { grab: (x, y), original: st.selection.unwrap() };
                } else if st.marks.is_empty() {
                    st.selection = None;
                    st.phase = Phase::Pressing { at: (x, y) };
                }
            }
            _ => {}
        }
        drop(st);
        self.redraw();
    }

    pub fn mouse_up(&self, index: usize, x: f32, y: f32) {
        let mut finish = None;
        let mut toolbar_click = None;
        {
            let mut st = self.st.borrow_mut();
            match st.phase {
                Phase::Pressing { .. } => {
                    // 单击：选中鼠标下的窗口（没有就是整块屏幕）
                    let hit = self.screens[index].windows.iter().find(|r| r.contains(x, y)).copied();
                    st.selection = hit;
                    st.phase = Phase::Editing;
                }
                Phase::Dragging { .. } => {
                    if st.selection.is_some_and(|s| s.w >= 4.0 && s.h >= 4.0) {
                        st.phase = Phase::Editing;
                    } else {
                        st.selection = None;
                        st.phase = Phase::Choosing;
                    }
                }
                Phase::Moving { .. } | Phase::Resizing { .. } => st.phase = Phase::Editing,
                Phase::Drawing => {
                    if let Some(mark) = st.drawing.take() {
                        let mark = match mark {
                            Mark::Rectangle { rect, color, width } => Mark::Rectangle { rect: render::rect_from((rect.x, rect.y), (rect.w, rect.h)), color, width },
                            Mark::Ellipse { rect, color, width } => Mark::Ellipse { rect: render::rect_from((rect.x, rect.y), (rect.w, rect.h)), color, width },
                            other => other,
                        };
                        let big_enough = match &mark {
                            Mark::Rectangle { rect, .. } | Mark::Ellipse { rect, .. } => rect.w >= 3.0 && rect.h >= 3.0,
                            Mark::Mosaic { .. } => true,
                        };
                        if big_enough {
                            st.marks.push(mark);
                        }
                    }
                    st.phase = Phase::Editing;
                }
                Phase::Editing => toolbar_click = st.ia.mouse_up(x, y),
                _ => {}
            }
            if matches!(st.phase, Phase::Editing) && st.ocr && st.selection.is_some() && toolbar_click.is_none() {
                finish = Some(Outcome::Recognize);
            }
        }
        if let Some(id) = toolbar_click {
            self.toolbar_action(id);
        }
        if let Some(outcome) = finish {
            self.finish(outcome);
            return;
        }
        self.redraw();
    }

    pub fn double_click(&self, _index: usize, x: f32, y: f32) {
        let inside = {
            let st = self.st.borrow();
            matches!(st.phase, Phase::Editing) && st.tool.is_none() && st.selection.is_some_and(|s| s.contains(x, y))
        };
        if inside {
            self.finish(Outcome::Copy);
        }
    }

    pub fn right_click(&self) {
        let reset = {
            let mut st = self.st.borrow_mut();
            if st.selection.is_some() && !st.ocr {
                st.selection = None;
                st.marks.clear();
                st.tool = None;
                st.phase = Phase::Choosing;
                true
            } else {
                false
            }
        };
        if reset {
            self.redraw();
        } else {
            self.finish(Outcome::Cancel);
        }
    }

    fn toolbar_action(&self, id: ui::Id) {
        let kind = ui::kind_of(id);
        let index = ui::index_of(id) as usize;
        match kind {
            T_RECT => self.toggle_tool(Tool::Rectangle),
            T_ELLIPSE => self.toggle_tool(Tool::Ellipse),
            T_MOSAIC => self.toggle_tool(Tool::Mosaic),
            T_UNDO => self.undo(),
            T_OCR => self.finish(Outcome::Recognize),
            T_PIN => self.finish(Outcome::Pin),
            T_CANCEL => self.finish(Outcome::Cancel),
            T_DONE => self.finish(Outcome::Copy),
            T_COLOR => {
                self.st.borrow_mut().color = index;
                app().settings.borrow_mut().capture_color = index;
                app().save_settings();
            }
            T_WIDTH => {
                self.st.borrow_mut().width = index;
                app().settings.borrow_mut().capture_width = index;
                app().save_settings();
            }
            _ => {}
        }
        self.redraw();
    }

    fn toggle_tool(&self, tool: Tool) {
        let mut st = self.st.borrow_mut();
        st.tool = if st.tool == Some(tool) { None } else { Some(tool) };
    }

    fn undo(&self) {
        self.st.borrow_mut().marks.pop();
        self.redraw();
    }

    // MARK: 键盘

    pub fn key(&self, vk: u16) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let editing = matches!(self.st.borrow().phase, Phase::Editing);
        match vk {
            0x1B => self.finish(Outcome::Cancel),
            0x0D => {
                if editing {
                    self.finish(Outcome::Copy);
                }
            }
            0x43 if ctrl => {
                if editing {
                    self.finish(Outcome::Copy);
                }
            }
            0x5A if ctrl => self.undo(),
            0x43 => self.copy_color(shift),
            0x52 if editing => self.toggle_tool(Tool::Rectangle),
            0x4F | 0x45 if editing => self.toggle_tool(Tool::Ellipse),
            0x4D if editing => self.toggle_tool(Tool::Mosaic),
            0x54 if editing => self.finish(Outcome::Recognize),
            0x50 if editing => self.finish(Outcome::Pin),
            _ => {}
        }
        self.redraw();
    }

    /// 复制鼠标下那个像素的颜色
    fn copy_color(&self, rgb: bool) {
        let Some((index, x, y)) = self.st.borrow().cursor else { return };
        let Some((_, _, c)) = self.screens[index].pixel_at(x, y) else { return };
        let text = if rgb { format!("rgb({}, {}, {})", c[0], c[1], c[2]) } else { format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]) };
        self.close();
        crate::clipboard::write_text(app().hwnd.get(), &text, None);
        hud::show(Toast::success(&format!("已复制 {text}"), "Ctrl+V 粘贴"));
    }

    // MARK: 完成

    /// 选区连同标注导出成图
    pub fn export(&self) -> Option<Image> {
        let st = self.st.borrow();
        let sel = st.selection?;
        let view = &self.screens[st.active];
        let s = view.scale;
        let px = (
            (sel.x * s).round().max(0.0) as u32,
            (sel.y * s).round().max(0.0) as u32,
            (sel.w * s).round().max(1.0) as u32,
            (sel.h * s).round().max(1.0) as u32,
        );
        render::export(&view.frozen, px, sel, &st.marks, s)
    }

    /// 选区左上角在屏幕上的像素位置
    fn selection_screen_origin(&self) -> Option<(i32, i32, f32)> {
        let st = self.st.borrow();
        let sel = st.selection?;
        let view = &self.screens[st.active];
        Some((view.rect.left + (sel.x * view.scale).round() as i32, view.rect.top + (sel.y * view.scale).round() as i32, view.scale))
    }

    pub fn finish(&self, outcome: Outcome) {
        let image = if outcome == Outcome::Cancel { None } else { self.export() };
        let origin = self.selection_screen_origin();
        self.close();
        let Some(image) = image else { return };
        match outcome {
            Outcome::Copy => {
                background("encode-capture", move || {
                    let Ok(png) = crate::images::encode_png(&image) else { return };
                    run_on_main(move || {
                        let (w, h) = (image.width, image.height);
                        app().add_captured_image(image, png);
                        hud::show(Toast::success("截图已复制", &format!("{w} × {h} · Ctrl+V 粘贴")));
                    });
                });
            }
            Outcome::Pin => {
                let (x, y, scale) = origin.unwrap_or((100, 100, 1.0));
                crate::pin::open(image, x, y, scale);
            }
            Outcome::Recognize => {
                let path = crate::util::data_dir().join("ocr-input.png");
                if crate::images::write_png(&image, &path).is_ok() {
                    app().recognize_file(path, "截图识别");
                }
            }
            Outcome::Cancel => {}
        }
    }

    /// 关掉遮罩，释放冻住的画面
    fn close(&self) {
        ui::hide_tooltip();
        for view in &self.screens {
            view.surface.borrow_mut().release();
            let hwnd = view.hwnd.get();
            if !hwnd.0.is_null() {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
        SESSION.with(|s| *s.borrow_mut() = None);
    }

    // MARK: 自检用

    pub fn test_select(&self, index: usize, rect: Rect) {
        let mut st = self.st.borrow_mut();
        st.active = index;
        st.selection = Some(rect);
        st.phase = Phase::Editing;
        drop(st);
        self.redraw();
    }

    pub fn test_mark(&self, mark: Mark) {
        self.st.borrow_mut().marks.push(mark);
        self.redraw();
    }

    pub fn test_tool(&self, tool: Option<Tool>) {
        self.st.borrow_mut().tool = tool;
        self.redraw();
    }

    pub fn screen_size(&self, index: usize) -> Rect {
        self.screens[index].size()
    }

    /// 离屏画某块屏幕的遮罩（不含系统画的东西）
    pub fn render_image(&self, index: usize) -> Option<Image> {
        let view = &self.screens[index];
        let (w, h) = (view.frozen.width, view.frozen.height);
        let pixels = gfx::render_offscreen(w, h, 96.0 * view.scale, theme::is_dark(), |p| self.draw(p, index))?;
        Some(Image { width: w, height: h, pixels })
    }

    // MARK: 绘制

    fn paint(&self, index: usize) {
        let view = &self.screens[index];
        let hwnd = view.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        let dpi = window::dpi(hwnd) as f32;
        let mut surface = std::mem::replace(&mut *view.surface.borrow_mut(), Surface::new(hwnd));
        surface.render(w as u32, h as u32, dpi, theme::is_dark(), |p| self.draw(p, index));
        *view.surface.borrow_mut() = surface;
    }

    fn draw(&self, p: &Painter, index: usize) {
        let view = &self.screens[index];
        let size = view.size();
        if let Some(bitmap) = view.bitmap() {
            p.bitmap(&bitmap, size, 1.0, false);
        }
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let on_this = st.active == index;
        let shown = if on_this {
            match st.phase {
                Phase::Choosing => st.hover_window,
                _ => st.selection,
            }
        } else {
            None
        };
        let dim = Color::BLACK.alpha(0.45);
        match shown {
            Some(sel) => {
                p.fill_rect(Rect::new(0.0, 0.0, size.w, sel.y), dim);
                p.fill_rect(Rect::new(0.0, sel.bottom(), size.w, size.h - sel.bottom()), dim);
                p.fill_rect(Rect::new(0.0, sel.y, sel.x, sel.h), dim);
                p.fill_rect(Rect::new(sel.right(), sel.y, size.w - sel.right(), sel.h), dim);
            }
            None => p.fill_rect(size, dim),
        }
        let Some(sel) = shown else {
            if on_this || st.cursor.is_some_and(|c| c.0 == index) {
                self.draw_hint(p, st, size);
                self.draw_loupe(p, st, index);
            }
            return;
        };
        // 标注（只在选区里）
        if on_this && (!st.marks.is_empty() || st.drawing.is_some()) {
            let mut marks = st.marks.clone();
            if let Some(drawing) = &st.drawing {
                marks.push(match drawing {
                    Mark::Rectangle { rect, color, width } => Mark::Rectangle { rect: render::rect_from((rect.x, rect.y), (rect.w, rect.h)), color: *color, width: *width },
                    Mark::Ellipse { rect, color, width } => Mark::Ellipse { rect: render::rect_from((rect.x, rect.y), (rect.w, rect.h)), color: *color, width: *width },
                    other => other.clone(),
                });
            }
            let mosaic = if marks.iter().any(Mark::is_mosaic) { view.mosaic() } else { None };
            p.clip(sel, || render::draw_marks(p, &marks, mosaic.as_ref(), view.scale, (0.0, 0.0)));
        }
        // 选区边框
        let editing = !matches!(st.phase, Phase::Choosing | Phase::Pressing { .. });
        p.stroke_rect(sel.inset(-1.0, -1.0), theme::BLUE, 2.0);
        if editing && matches!(st.phase, Phase::Editing | Phase::Moving { .. } | Phase::Resizing { .. } | Phase::Drawing) && !st.ocr {
            for (hx, hy) in [
                (sel.x, sel.y),
                (sel.mid_x(), sel.y),
                (sel.right(), sel.y),
                (sel.right(), sel.mid_y()),
                (sel.right(), sel.bottom()),
                (sel.mid_x(), sel.bottom()),
                (sel.x, sel.bottom()),
                (sel.x, sel.mid_y()),
            ] {
                p.fill_circle(hx, hy, 4.5, Color::WHITE);
                p.stroke_ellipse(hx, hy, 4.5, 4.5, theme::BLUE, 1.5);
            }
        }
        // 左上角的分辨率
        let (pw, ph) = ((sel.w * view.scale).round() as u32, (sel.h * view.scale).round() as u32);
        let label = format!("{pw} × {ph}");
        let font = Font::ui(12.0, 600);
        let lw = crate::text::measure(&label, font) + 16.0;
        let ly = if sel.y >= 30.0 { sel.y - 28.0 } else { sel.y + 6.0 };
        let badge = Rect::new(sel.x.min(size.w - lw - 4.0).max(4.0), ly, lw, 22.0);
        p.fill_round(badge, 6.0, Color::hex(0x111214).alpha(0.82));
        p.text(&label, badge, font, Color::WHITE, TextOpts::CENTER);

        match st.phase {
            Phase::Pressing { .. } | Phase::Dragging { .. } | Phase::Choosing => {
                self.draw_hint(p, st, size);
                self.draw_loupe(p, st, index);
            }
            _ => {
                if on_this && !st.ocr {
                    self.draw_toolbar(p, st);
                }
            }
        }
    }

    fn draw_hint(&self, p: &Painter, st: &State, size: Rect) {
        if !matches!(st.phase, Phase::Choosing) {
            return;
        }
        let text = if st.ocr { "框选要识别文字的区域 · 单击选中窗口 · Esc 取消" } else { "单击选中窗口，拖动框选区域 · C 复制色号 · Esc 取消" };
        let font = Font::ui(13.0, 500);
        let w = crate::text::measure(text, font) + 28.0;
        let r = Rect::new(size.w / 2.0 - w / 2.0, 24.0, w, 32.0);
        p.fill_round(r, 16.0, Color::hex(0x111214).alpha(0.78));
        p.text(text, r, font, Color::WHITE.alpha(0.92), TextOpts::CENTER);
    }

    /// 放大镜：17×11 个像素放大 10 倍，下面是坐标和色号
    fn draw_loupe(&self, p: &Painter, st: &State, index: usize) {
        let Some((ci, x, y)) = st.cursor else { return };
        if ci != index {
            return;
        }
        let view = &self.screens[index];
        let Some((px, py, color)) = view.pixel_at(x, y) else { return };
        let Some(bitmap) = view.bitmap() else { return };
        let (cols, rows, cell) = (17.0f32, 11.0f32, 10.0f32);
        let (lw, lh) = (cols * cell, rows * cell);
        let info_h = 64.0;
        let size = view.size();
        let mut lx = x + 22.0;
        let mut ly = y + 22.0;
        if lx + lw > size.w - 4.0 {
            lx = x - 22.0 - lw;
        }
        if ly + lh + info_h > size.h - 4.0 {
            ly = y - 22.0 - lh - info_h;
        }
        let frame = Rect::new(lx, ly, lw, lh + info_h);
        p.soft_shadow(frame, 10.0, Color::BLACK.alpha(0.5), 8.0, 3.0);
        p.fill_round(frame, 10.0, Color::hex(0x161719).alpha(0.94));
        let src = Rect::new(px as f32 - (cols / 2.0).floor(), py as f32 - (rows / 2.0).floor(), cols, rows);
        let dest = Rect::new(lx, ly, lw, lh);
        p.clip_round(Rect::new(lx, ly, lw, lh + 10.0), 10.0, || {
            p.fill_rect(dest, Color::BLACK);
            p.bitmap_part(&bitmap, dest, src, 1.0, false);
            // 中间那个像素
            let center = Rect::new(lx + (cols / 2.0).floor() * cell, ly + (rows / 2.0).floor() * cell, cell, cell);
            p.line(lx, center.mid_y(), lx + lw, center.mid_y(), theme::BLUE.alpha(0.35), cell);
            p.line(center.mid_x(), ly, center.mid_x(), ly + lh, theme::BLUE.alpha(0.35), cell);
            p.bitmap_part(&bitmap, center, Rect::new(px as f32, py as f32, 1.0, 1.0), 1.0, false);
            p.stroke_rect(center, Color::WHITE, 1.5);
        });
        let gx = view.rect.left + px as i32;
        let gy = view.rect.top + py as i32;
        let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
        let info_y = ly + lh;
        p.text(&format!("X {gx}   Y {gy}"), Rect::new(lx + 10.0, info_y + 4.0, lw - 20.0, 18.0), Font::ui(12.0, 500), Color::WHITE.alpha(0.75), TextOpts::LINE);
        let swatch = Rect::new(lx + 10.0, info_y + 25.0, 16.0, 16.0);
        p.fill_round(swatch, 4.0, Color { r: color[0] as f32 / 255.0, g: color[1] as f32 / 255.0, b: color[2] as f32 / 255.0, a: 1.0 });
        p.stroke_round(swatch, 4.0, Color::WHITE.alpha(0.5), 1.0);
        p.text(&hex, Rect::new(lx + 32.0, info_y + 23.0, lw - 40.0, 20.0), Font::mono(13.0), Color::WHITE, TextOpts::LINE);
        p.text("C 复制色号 · Shift+C 复制 RGB", Rect::new(lx + 10.0, info_y + 44.0, lw - 20.0, 16.0), Font::ui(11.0, 400), Color::WHITE.alpha(0.55), TextOpts::LINE);
    }

    fn draw_toolbar(&self, p: &Painter, st: &mut State) {
        let Some(bar) = self.toolbar_rect(st) else { return };
        st.ia.begin_frame();
        let dark = true;
        let bg = Color::hex(0x1C1D20).alpha(0.94);
        let pal = theme::palette_for(dark);
        p.soft_shadow(bar, 12.0, Color::BLACK.alpha(0.45), 8.0, 3.0);
        p.fill_round(bar, 12.0, bg);
        p.stroke_round(bar.inset(0.5, 0.5), 12.0, Color::WHITE.alpha(0.1), 1.0);
        let mut x = bar.x + 5.0;
        let has_marks = !st.marks.is_empty();
        for slot in SLOTS {
            match slot {
                Slot::Gap => {
                    p.line(x + 5.5, bar.y + 12.0, x + 5.5, bar.bottom() - 12.0, Color::WHITE.alpha(0.14), 1.0);
                    x += 11.0;
                }
                Slot::Button(kind, icon, tip) => {
                    let r = Rect::new(x, bar.y + 5.0, BUTTON, BUTTON);
                    let id = ui::id(kind, 0);
                    let selected = match kind {
                        T_RECT => st.tool == Some(Tool::Rectangle),
                        T_ELLIPSE => st.tool == Some(Tool::Ellipse),
                        T_MOSAIC => st.tool == Some(Tool::Mosaic),
                        _ => false,
                    };
                    if selected {
                        p.fill_round(r, 8.0, theme::BLUE);
                    }
                    let base = match kind {
                        T_DONE => theme::GREEN.mix(Color::WHITE, 0.15),
                        T_CANCEL => Color::hex(0xFF6B6B),
                        T_UNDO if !has_marks => Color::WHITE.alpha(0.3),
                        _ => Color::WHITE.alpha(0.85),
                    };
                    st.ia.add_tip(id, r, tip);
                    let hover = st.ia.hover(id);
                    if !selected && hover > 0.0 {
                        p.fill_round(r, 8.0, Color::WHITE.alpha(0.12 * hover));
                    }
                    icons::draw(p, icon, r, 17.0, if selected { Color::WHITE } else { base });
                    x += BUTTON + 2.0;
                }
            }
        }
        // 第二行：颜色、粗细
        if let Some(tool) = st.tool {
            let sub = Rect::new(bar.x, bar.bottom() + 6.0, if tool == Tool::Mosaic { 130.0 } else { 300.0 }, SUBBAR_H);
            let sub = Rect::new((bar.right() - sub.w).max(bar.x), sub.y, sub.w, sub.h);
            p.soft_shadow(sub, 12.0, Color::BLACK.alpha(0.4), 6.0, 2.0);
            p.fill_round(sub, 12.0, bg);
            let mut x = sub.x + 10.0;
            let widths = if tool == Tool::Mosaic { render::BRUSH_WIDTHS } else { render::LINE_WIDTHS };
            for (i, w) in widths.iter().enumerate() {
                let r = Rect::new(x, sub.y + 5.0, 28.0, 28.0);
                let id = ui::id(T_WIDTH, i as u32);
                st.ia.add(id, r);
                if st.width == i {
                    p.fill_round(r, 8.0, Color::WHITE.alpha(0.16));
                }
                let dot = if tool == Tool::Mosaic { w / 3.0 } else { w * 1.2 + 2.0 };
                p.fill_circle(r.mid_x(), r.mid_y(), dot.min(11.0) / 2.0 + 1.0, Color::WHITE.alpha(0.85));
                x += 30.0;
            }
            if tool != Tool::Mosaic {
                p.line(x + 4.0, sub.y + 10.0, x + 4.0, sub.bottom() - 10.0, Color::WHITE.alpha(0.14), 1.0);
                x += 12.0;
                for (i, c) in render::COLORS.iter().enumerate() {
                    let r = Rect::new(x, sub.y + 5.0, 26.0, 28.0);
                    let id = ui::id(T_COLOR, i as u32);
                    st.ia.add(id, r);
                    let radius = if st.color == i { 9.0 } else { 7.5 };
                    if st.color == i {
                        p.stroke_ellipse(r.mid_x(), r.mid_y(), 11.0, 11.0, Color::WHITE.alpha(0.9), 1.5);
                    }
                    p.fill_circle(r.mid_x(), r.mid_y(), radius, *c);
                    x += 26.0;
                }
            }
            let _ = pal;
        }
    }

    fn cursor_for(&self, index: usize) -> PCWSTRCursor {
        let st = self.st.borrow();
        let Some((ci, x, y)) = st.cursor else { return IDC_CROSS_ID };
        match st.phase {
            Phase::Choosing | Phase::Pressing { .. } | Phase::Dragging { .. } => IDC_CROSS_ID,
            Phase::Drawing => IDC_CROSS_ID,
            Phase::Moving { .. } => IDC_SIZEALL_ID,
            Phase::Resizing { handle, .. } => handle_cursor(handle),
            Phase::Editing => {
                if ci != index || ci != st.active {
                    return IDC_ARROW_ID;
                }
                if st.ia.hit_id(x, y).is_some() {
                    return IDC_HAND_ID;
                }
                if st.tool.is_some() && st.selection.is_some_and(|s| s.contains(x, y)) {
                    return IDC_CROSS_ID;
                }
                if let Some(handle) = self.handle_at(&st, x, y) {
                    return handle_cursor(handle);
                }
                if st.selection.is_some_and(|s| s.contains(x, y)) {
                    return IDC_SIZEALL_ID;
                }
                IDC_ARROW_ID
            }
        }
    }
}

type PCWSTRCursor = u16;
const IDC_ARROW_ID: u16 = 32512;
const IDC_CROSS_ID: u16 = 32515;
const IDC_SIZENWSE_ID: u16 = 32642;
const IDC_SIZENESW_ID: u16 = 32643;
const IDC_SIZEWE_ID: u16 = 32644;
const IDC_SIZENS_ID: u16 = 32645;
const IDC_SIZEALL_ID: u16 = 32646;
const IDC_HAND_ID: u16 = 32649;

fn handle_cursor(handle: Handle) -> u16 {
    match handle {
        Handle::TopLeft | Handle::BottomRight => IDC_SIZENWSE_ID,
        Handle::TopRight | Handle::BottomLeft => IDC_SIZENESW_ID,
        Handle::Top | Handle::Bottom => IDC_SIZENS_ID,
        Handle::Left | Handle::Right => IDC_SIZEWE_ID,
    }
}

/// 一块屏幕上的遮罩窗口
struct Overlay {
    index: usize,
}

impl Handler for Overlay {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let session = session()?;
        let (x, y) = window::mouse_dip(hwnd, lp);
        match msg {
            WM_PAINT => {
                session.paint(self.index);
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                session.mouse_move(self.index, x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(hwnd);
                }
                session.mouse_down(self.index, x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                session.mouse_up(self.index, x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONDBLCLK => {
                session.double_click(self.index, x, y);
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                session.right_click();
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                session.key(wp.0 as u16);
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                let id = session.cursor_for(self.index);
                unsafe {
                    if let Ok(cursor) = LoadCursorW(None, windows::core::PCWSTR(id as usize as *const u16)) {
                        SetCursor(Some(cursor));
                    }
                }
                Some(LRESULT(1))
            }
            WM_ACTIVATEAPP if wp.0 == 0 => {
                // 切到别的程序（Alt+Tab、Win 键）就退出截图
                let hidden = session.st.borrow().hidden;
                if !hidden {
                    run_on_main(|| {
                        if let Some(s) = SESSION.with(|s| s.borrow().clone()) {
                            s.finish(Outcome::Cancel);
                        }
                    });
                }
                None
            }
            WM_TIMER => {
                let tip = session.st.borrow_mut().ia.pending_tip();
                if let Some((text, rect)) = tip {
                    ui::show_tooltip(&text, ui::dip_rect_to_screen(hwnd, rect));
                }
                Some(LRESULT(0))
            }
            WM_CREATE => {
                unsafe {
                    SetTimer(Some(hwnd), 1, 400, None);
                }
                None
            }
            _ => None,
        }
    }
}
