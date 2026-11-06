//! 悬浮窗：标题栏、"保存为 Markdown"大按钮、"已保存 | 剪贴板"两个列表。
//!
//! 和 macOS 版一样：不抢焦点（点它不会让正在用的程序失去焦点）、可以拖动、贴边收起、
//! 拖文字和文件进来（上半块存成 Markdown，下半块只放进剪贴板）、删除后可以撤销。
//! 窗口高度跟着列表走；切换标签时先长高、晚一点再缩回去，快速来回切换不会跳。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use clipmd_core::clip_history::ClipItem;
use clipmd_core::note_writer::SavedNote;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{self, Spring};
use crate::app::app;
use crate::clipboard::Hint;
use crate::gfx::{Color, Painter, Rect};
use crate::icons::{self, Icon};
use crate::menu::{self, Entry};
use crate::settings::Edge;
use crate::text::{Font, TextOpts};
use crate::theme::{self, Palette};
use crate::ui::{self, Canvas, Cursor, Id};
use crate::window::{self, Handler};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Saved,
    Clipboard,
}

pub const WIDTH: f32 = 300.0;
const ROW_H: f32 = 46.0;
const LIST_TOP: f32 = 166.0;
const COLLAPSED_ROWS: usize = 5;
const EXPANDED_ROWS: usize = 9;
const FOOTER_H: f32 = 30.0;
const BOTTOM: f32 = 10.0;
const EMPTY_H: f32 = 108.0;
const UNDO_H: f32 = 46.0;
/// 贴边收起时露出来的宽度
const HANDLE_W: f32 = 14.0;

const TIMER_SHRINK: usize = 1;
const TIMER_TIP: usize = 2;
const TIMER_TUCK_BACK: usize = 3;
const TIMER_CLOCK: usize = 4;
const TIMER_FEEDBACK: usize = 5;

// 可点击区域的种类
const K_SAVE: u32 = 1;
const K_SHOT: u32 = 2;
const K_FOLDER: u32 = 3;
const K_TUCK: u32 = 4;
const K_SETTINGS: u32 = 5;
const K_TAB: u32 = 6;
const K_CLEAR: u32 = 7;
const K_EXPAND: u32 = 8;
const K_NOTE: u32 = 10;
const K_NOTE_EYE: u32 = 11;
const K_NOTE_REVEAL: u32 = 12;
const K_NOTE_DELETE: u32 = 13;
const K_CLIP: u32 = 20;
const K_CLIP_EYE: u32 = 21;
const K_CLIP_SAVE: u32 = 22;
const K_CLIP_REMOVE: u32 = 23;
const K_UNDO: u32 = 30;
const K_UNDO_CLOSE: u32 = 31;
const K_EMPTY_BUTTON: u32 = 32;
const K_LIST: u32 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Save,
    Clipboard,
}

/// 拖东西进来时的样子
#[derive(Clone, Debug, PartialEq)]
pub enum DropView {
    Zones(Option<Zone>),
    Unsupported(String),
}

struct State {
    canvas: Canvas,
    tab: Tab,
    tab_spring: Spring,
    expanded: bool,
    scroll: [Spring; 2],
    hint: Hint,
    fresh: bool,
    saved_at: Option<f64>,
    saved_on_clipboard: bool,
    copied: Option<(Tab, String, f64)>,
    undo: Option<String>,
    undo_spring: Spring,
    drop: Option<DropView>,
    drop_spring: Spring,
    shake: Spring,
    appear: Spring,
    tuck_out: Spring,
    tucked: Option<Edge>,
    /// 收起前的位置（屏幕像素）
    home: Option<(i32, i32)>,
    /// 当前窗口高度（DIP）
    height: f32,
    thumbs: HashMap<String, Option<ID2D1Bitmap>>,
    indicator_until: f64,
    drag_from: Option<(Id, f32, f32)>,
    visible: bool,
}

pub struct Panel {
    hwnd: Cell<HWND>,
    st: RefCell<State>,
}

/// 点击之后要做的事。先算出来，放开状态的借用以后再做（做的事可能又要刷新悬浮窗）
enum Act {
    Save,
    Screenshot(bool),
    ScreenshotMenu(POINT),
    Folder,
    Tuck,
    Settings,
    Tab(Tab),
    Expand,
    Clear,
    CopyNote(SavedNote),
    PreviewNote(SavedNote),
    RevealNote(SavedNote),
    DeleteMenu(SavedNote, POINT),
    NoteMenu(SavedNote, POINT),
    CopyClip(String),
    PreviewClip(String),
    SaveClip(String),
    RemoveClip(String),
    ClipMenu(String, POINT),
    Undo,
    CloseUndo,
    EmptyButton,
    DragNote(SavedNote),
}

fn list_height(count: usize, expanded: bool) -> f32 {
    if count == 0 {
        return EMPTY_H;
    }
    let rows = count.min(if expanded { EXPANDED_ROWS } else { COLLAPSED_ROWS });
    rows as f32 * ROW_H + if count > COLLAPSED_ROWS { FOOTER_H } else { 4.0 }
}

impl Panel {
    pub fn create() -> Rc<Panel> {
        let panel = Rc::new(Panel {
            hwnd: Cell::new(HWND::default()),
            st: RefCell::new(State {
                canvas: Canvas::new(HWND::default()),
                tab: Tab::Saved,
                tab_spring: Spring::with(0.0, 0.34, 0.9),
                expanded: false,
                scroll: [Spring::with(0.0, 0.25, 1.0), Spring::with(0.0, 0.25, 1.0)],
                hint: Hint::Empty,
                fresh: false,
                saved_at: None,
                saved_on_clipboard: false,
                copied: None,
                undo: None,
                undo_spring: Spring::with(0.0, 0.3, 0.9),
                drop: None,
                drop_spring: Spring::with(0.0, 0.22, 1.0),
                shake: Spring::with(0.0, 0.12, 0.25),
                appear: Spring::with(0.0, 0.32, 0.86),
                tuck_out: Spring::with(1.0, 0.3, 0.92),
                tucked: None,
                home: None,
                height: 300.0,
                thumbs: HashMap::new(),
                indicator_until: 0.0,
                drag_from: None,
                visible: false,
            }),
        });
        let height = panel.target_height();
        panel.st.borrow_mut().height = height;
        let (x, y) = panel.initial_position(height);
        let scale = window::monitor_dpi_at(POINT { x, y }) as f32 / 96.0;
        let style = WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN;
        let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        let hwnd = window::create("ClipMD", style, ex, (x, y, (WIDTH * scale) as i32, (height * scale) as i32), None, panel.clone())
            .expect("创建悬浮窗失败");
        panel.hwnd.set(hwnd);
        panel.st.borrow_mut().canvas = Canvas::new(hwnd);
        panel.restyle();
        crate::dragdrop::register_drop_target(hwnd);
        // 上次是贴边收起的状态
        let tucked = app().settings.borrow().tucked;
        if let Some(edge) = tucked {
            panel.tuck_to(edge, false);
        }
        panel
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd.get()
    }

    fn initial_position(&self, height: f32) -> (i32, i32) {
        let saved = app().settings.borrow().panel_position;
        if let Some((x, y)) = saved {
            // 屏幕布局变了、原来的位置看不到了就不用
            let on_screen = unsafe { !MonitorFromPoint(POINT { x: x + 40, y: y + 20 }, MONITOR_DEFAULTTONULL).is_invalid() };
            if on_screen {
                return (x, y);
            }
        }
        let (work, _) = window::monitor_rects(POINT { x: 0, y: 0 });
        let scale = window::monitor_dpi_at(POINT { x: work.right - 10, y: work.top + 10 }) as f32 / 96.0;
        let x = work.right - ((WIDTH + 24.0) * scale) as i32;
        let y = work.top + ((work.bottom - work.top) - (height * scale) as i32) / 3;
        (x, y.max(work.top + 12))
    }

    pub fn restyle(&self) {
        window::style_window(self.hwnd(), theme::is_dark(), window::Backdrop::Acrylic, true);
        self.st.borrow_mut().thumbs.clear();
        window::invalidate(self.hwnd());
    }

    pub fn is_visible(&self) -> bool {
        window::is_visible(self.hwnd())
    }

    pub fn show(&self) {
        self.relayout(true);
        let hwnd = self.hwnd();
        {
            let mut st = self.st.borrow_mut();
            st.visible = true;
            if !window::is_visible(hwnd) {
                st.appear.snap(0.0);
            }
            st.appear.set(1.0);
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            SetTimer(Some(hwnd), TIMER_CLOCK, 30_000, None);
        }
        self.animate();
    }

    pub fn hide(&self) {
        ui::hide_tooltip();
        self.st.borrow_mut().visible = false;
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_HIDE);
            let _ = KillTimer(Some(self.hwnd()), TIMER_CLOCK);
        }
        crate::clip_preview::close();
    }

    /// 另一个 ClipMD 被打开时：把悬浮窗叫出来、轻轻晃一下
    pub fn attention(&self) {
        if self.st.borrow().tucked.is_some() {
            self.st.borrow_mut().tuck_out.set(1.0);
            self.animate();
            unsafe {
                SetTimer(Some(self.hwnd()), TIMER_TUCK_BACK, 2500, None);
            }
        }
        self.shake();
    }

    /// 数据变了：重新算高度、重画
    pub fn refresh(&self) {
        self.relayout(false);
        window::invalidate(self.hwnd());
    }

    pub fn set_hint(&self, hint: Hint, fresh: bool) {
        {
            let mut st = self.st.borrow_mut();
            st.hint = hint;
            st.fresh = fresh && hint.can_save();
        }
        window::invalidate(self.hwnd());
    }

    pub fn did_save(&self, on_clipboard: bool) {
        {
            let mut st = self.st.borrow_mut();
            st.saved_at = Some(crate::util::now_seconds());
            st.saved_on_clipboard = on_clipboard;
            st.fresh = false;
            st.tab = Tab::Saved;
            st.tab_spring.set(0.0);
            st.scroll[0].set(0.0);
            if on_clipboard {
                if let Some(first) = app().notes.borrow().notes.first() {
                    st.copied = Some((Tab::Saved, first.file_name(), crate::util::now_seconds()));
                }
            }
        }
        unsafe {
            SetTimer(Some(self.hwnd()), TIMER_FEEDBACK, 2600, None);
        }
        self.refresh();
        self.animate();
    }

    pub fn mark_copied(&self, tab: Tab, key: String) {
        self.st.borrow_mut().copied = Some((tab, key, crate::util::now_seconds()));
        unsafe {
            SetTimer(Some(self.hwnd()), TIMER_FEEDBACK, 2600, None);
        }
        window::invalidate(self.hwnd());
    }

    pub fn show_undo(&self, message: &str) {
        {
            let mut st = self.st.borrow_mut();
            st.undo = Some(message.to_string());
            st.undo_spring.set(1.0);
        }
        self.relayout(true);
        self.animate();
    }

    pub fn hide_undo(&self) {
        {
            let mut st = self.st.borrow_mut();
            if st.undo.is_none() {
                return;
            }
            st.undo = None;
            st.undo_spring.set(0.0);
        }
        self.relayout(false);
        self.animate();
    }

    /// 保存失败时大按钮左右晃一下
    pub fn shake(&self) {
        self.st.borrow_mut().shake.velocity = 420.0;
        self.animate();
    }

    pub fn select_tab(&self, tab: Tab) {
        {
            let mut st = self.st.borrow_mut();
            st.tab = tab;
            st.tab_spring.set(if tab == Tab::Saved { 0.0 } else { 1.0 });
        }
        self.relayout(false);
        self.animate();
    }

    pub fn set_drop(&self, view: Option<DropView>) {
        {
            let mut st = self.st.borrow_mut();
            if st.drop == view {
                return;
            }
            st.drop_spring.set(if view.is_some() { 1.0 } else { 0.0 });
            if view.is_some() {
                st.drop = view;
            }
            if st.drop_spring.target == 0.0 && !st.drop_spring.is_moving() {
                st.drop = None;
            }
        }
        if self.st.borrow().tucked.is_some() {
            self.st.borrow_mut().tuck_out.set(1.0);
        }
        self.animate();
    }

    /// 屏幕坐标是落在上半块还是下半块
    pub fn zone_at(&self, screen: POINT) -> Zone {
        let (_, y) = window::screen_to_dip(self.hwnd(), screen);
        let h = self.st.borrow().height;
        if y < h / 2.0 { Zone::Save } else { Zone::Clipboard }
    }

    pub fn ensure_on_screen(&self) {
        let r = window::window_rect(self.hwnd());
        let on_screen = unsafe { !MonitorFromPoint(POINT { x: r.left + 40, y: r.top + 20 }, MONITOR_DEFAULTTONULL).is_invalid() };
        if !on_screen && self.st.borrow().tucked.is_none() {
            let (x, y) = {
                app().settings.borrow_mut().panel_position = None;
                self.initial_position(self.st.borrow().height)
            };
            unsafe {
                let _ = SetWindowPos(self.hwnd(), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }

    // MARK: 高度

    fn counts(&self) -> (usize, usize) {
        let a = app();
        (a.notes.borrow().notes.len(), a.history.borrow().items.len())
    }

    fn target_height(&self) -> f32 {
        let (notes, clips) = self.counts();
        let st = self.st.borrow();
        let count = if st.tab == Tab::Saved { notes } else { clips };
        let undo = if st.undo.is_some() { UNDO_H } else { 0.0 };
        LIST_TOP + list_height(count, st.expanded) + undo + BOTTOM
    }

    /// 变高马上变，变矮等一会儿（快速切换标签不会忽高忽低）
    fn relayout(&self, immediate: bool) {
        let target = self.target_height();
        let current = self.st.borrow().height;
        let hwnd = self.hwnd();
        if hwnd.0.is_null() {
            self.st.borrow_mut().height = target;
            return;
        }
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_SHRINK);
        }
        if target > current + 0.5 || immediate {
            self.apply_height(target);
        } else if target < current - 0.5 {
            unsafe {
                SetTimer(Some(hwnd), TIMER_SHRINK, 450, None);
            }
        }
    }

    fn apply_height(&self, height: f32) {
        let hwnd = self.hwnd();
        self.st.borrow_mut().height = height;
        let scale = window::scale(hwnd);
        unsafe {
            let _ = SetWindowPos(hwnd, None, 0, 0, (WIDTH * scale).round() as i32, (height * scale).round() as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.paint();
    }

    // MARK: 贴边收起

    fn work_area(&self) -> RECT {
        let r = window::window_rect(self.hwnd());
        let (work, _) = window::monitor_rects(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 });
        work
    }

    pub fn toggle_tuck(&self) {
        let tucked = self.st.borrow().tucked;
        match tucked {
            Some(_) => self.untuck(),
            None => {
                let r = window::window_rect(self.hwnd());
                let work = self.work_area();
                // 往离得近、外面没有别的屏幕的那一边收
                let left_free = unsafe { MonitorFromPoint(POINT { x: work.left - 20, y: r.top + 20 }, MONITOR_DEFAULTTONULL).is_invalid() };
                let right_free = unsafe { MonitorFromPoint(POINT { x: work.right + 20, y: r.top + 20 }, MONITOR_DEFAULTTONULL).is_invalid() };
                let nearer_left = (r.left - work.left) < (work.right - r.right);
                let edge = match (left_free, right_free) {
                    (true, false) => Edge::Left,
                    (false, true) => Edge::Right,
                    _ => if nearer_left { Edge::Left } else { Edge::Right },
                };
                self.st.borrow_mut().home = Some((r.left, r.top));
                self.tuck_to(edge, true);
            }
        }
    }

    fn tuck_to(&self, edge: Edge, animated: bool) {
        {
            let mut st = self.st.borrow_mut();
            st.tucked = Some(edge);
            if animated {
                st.tuck_out.set(0.0);
            } else {
                st.tuck_out.snap(0.0);
            }
        }
        app().settings.borrow_mut().tucked = Some(edge);
        app().save_settings();
        if animated {
            self.animate();
        } else {
            self.apply_tuck_position();
        }
    }

    fn untuck(&self) {
        let home = self.st.borrow_mut().home.take();
        let hwnd = self.hwnd();
        {
            let mut st = self.st.borrow_mut();
            st.tucked = None;
            st.tuck_out.snap(1.0);
        }
        app().settings.borrow_mut().tucked = None;
        app().save_settings();
        let work = self.work_area();
        let r = window::window_rect(hwnd);
        let w = r.right - r.left;
        let s = window::scale(hwnd);
        let (x, y) = home.unwrap_or_else(|| {
            let margin = (16.0 * s) as i32;
            if r.left < work.left + 40 { (work.left + margin, r.top) } else { (work.right - w - margin, r.top) }
        });
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        window::invalidate(hwnd);
    }

    /// 按 tuck_out（0 收起、1 探出）放窗口
    fn apply_tuck_position(&self) {
        let (edge, out) = {
            let st = self.st.borrow();
            match st.tucked {
                Some(edge) => (edge, st.tuck_out.value),
                None => return,
            }
        };
        let hwnd = self.hwnd();
        let r = window::window_rect(hwnd);
        let w = r.right - r.left;
        let s = window::scale(hwnd);
        let work = self.work_area();
        let handle = (HANDLE_W * s) as i32;
        let margin = (10.0 * s) as i32;
        let (hidden, shown) = match edge {
            Edge::Left => (work.left - w + handle, work.left + margin),
            Edge::Right => (work.right - handle, work.right - w - margin),
        };
        let x = hidden + ((shown - hidden) as f32 * out).round() as i32;
        let y = r.top.clamp(work.top, (work.bottom - (r.bottom - r.top)).max(work.top));
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    // MARK: 动画

    fn animate(&self) {
        let hwnd = self.hwnd();
        if !hwnd.0.is_null() {
            anim::request_frame(hwnd);
        }
    }

    fn frame(&self) {
        let (moving, tuck_moving) = {
            let mut st = self.st.borrow_mut();
            let dt = st.canvas.timer.tick();
            let mut moving = false;
            moving |= st.tab_spring.step(dt);
            moving |= st.scroll[0].step(dt);
            moving |= st.scroll[1].step(dt);
            moving |= st.undo_spring.step(dt);
            moving |= st.drop_spring.step(dt);
            if st.drop_spring.target == 0.0 && !st.drop_spring.is_moving() {
                st.drop = None;
            }
            moving |= st.shake.step(dt);
            moving |= st.appear.step(dt);
            moving |= st.canvas.ia.step(dt);
            let tuck_moving = st.tuck_out.step(dt);
            moving |= tuck_moving;
            if crate::util::now_seconds() < st.indicator_until + 0.4 {
                moving = true;
            }
            (moving, tuck_moving)
        };
        if tuck_moving || self.st.borrow().tucked.is_some() {
            self.apply_tuck_position();
        }
        self.paint();
        if moving {
            self.animate();
        } else {
            self.st.borrow_mut().canvas.timer.reset();
        }
    }

    // MARK: 绘制

    fn paint(&self) {
        let hwnd = self.hwnd();
        let (w, h) = window::client_size(hwnd);
        if w <= 0 || h <= 0 {
            return;
        }
        let dpi = window::dpi(hwnd) as f32;
        let dark = theme::is_dark();
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let mut surface = std::mem::replace(&mut st.canvas.surface, crate::gfx::Surface::new(hwnd));
        surface.render(w as u32, h as u32, dpi, dark, |p| {
            let size = Rect::new(0.0, 0.0, w as f32 * 96.0 / dpi, h as f32 * 96.0 / dpi);
            draw_panel(p, st, size, window::backdrop_supported());
        });
        st.canvas.surface = surface;
    }

    /// 离屏画一张（自检、README 截图用），背景画成不透明的
    pub fn render_image(&self, dark: bool, scale: f32) -> Option<crate::images::Image> {
        let height = self.target_height();
        let (w, h) = ((WIDTH * scale) as u32, (height * scale) as u32);
        let saved = theme::is_dark();
        theme::set_appearance(if dark { theme::Appearance::Dark } else { theme::Appearance::Light });
        let pixels = {
            let mut st = self.st.borrow_mut();
            st.height = height;
            crate::gfx::render_offscreen(w, h, 96.0 * scale, dark, |p| {
                draw_panel(p, &mut st, Rect::new(0.0, 0.0, WIDTH, height), false);
            })
        };
        theme::set_appearance(if saved { theme::Appearance::Dark } else { theme::Appearance::Light });
        theme::set_appearance(app().settings.borrow().appearance);
        pixels.map(|pixels| crate::images::Image { width: w, height: h, pixels })
    }

    // MARK: 鼠标

    fn dip(&self, lp: LPARAM) -> (f32, f32) {
        window::mouse_dip(self.hwnd(), lp)
    }

    fn on_mouse_move(&self, x: f32, y: f32, button_down: bool) {
        window::track_mouse_leave(self.hwnd());
        let mut drag_note = None;
        {
            let mut st = self.st.borrow_mut();
            if st.tucked.is_some() && st.tuck_out.target < 1.0 {
                st.tuck_out.set(1.0);
                drop(st);
                self.animate();
                st = self.st.borrow_mut();
            }
            unsafe {
                let _ = KillTimer(Some(self.hwnd()), TIMER_TUCK_BACK);
            }
            if let Some((id, sx, sy)) = st.drag_from {
                if button_down && ((x - sx).powi(2) + (y - sy).powi(2)).sqrt() > 5.0 {
                    st.drag_from = None;
                    if ui::kind_of(id) == K_NOTE {
                        drag_note = app().notes.borrow().notes.get(ui::index_of(id) as usize).cloned();
                    }
                }
            }
            if st.canvas.ia.mouse_move(x, y) {
                ui::hide_tooltip();
                if st.canvas.ia.wants_tip_timer() {
                    unsafe {
                        SetTimer(Some(self.hwnd()), TIMER_TIP, 600, None);
                    }
                }
                drop(st);
                self.animate();
            }
        }
        if let Some(note) = drag_note {
            self.st.borrow_mut().canvas.ia.mouse_leave();
            unsafe {
                let _ = ReleaseCapture();
            }
            self.perform(Act::DragNote(note));
        }
    }

    fn on_mouse_leave(&self) {
        ui::hide_tooltip();
        {
            let mut st = self.st.borrow_mut();
            st.canvas.ia.mouse_leave();
        }
        let tucked = self.st.borrow().tucked.is_some();
        if tucked {
            unsafe {
                SetTimer(Some(self.hwnd()), TIMER_TUCK_BACK, 650, None);
            }
        }
        self.animate();
    }

    fn on_button_down(&self, x: f32, y: f32) {
        ui::hide_tooltip();
        let mut st = self.st.borrow_mut();
        let id = st.canvas.ia.mouse_down(x, y);
        st.drag_from = id.filter(|id| ui::kind_of(*id) == K_NOTE).map(|id| (id, x, y));
        drop(st);
        unsafe {
            SetCapture(self.hwnd());
        }
        self.animate();
    }

    fn on_button_up(&self, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let clicked = {
            let mut st = self.st.borrow_mut();
            st.drag_from = None;
            st.canvas.ia.mouse_up(x, y)
        };
        self.animate();
        if let Some(id) = clicked {
            if let Some(act) = self.action_for(id, false) {
                self.perform(act);
            }
        }
    }

    fn on_right_click(&self, x: f32, y: f32) {
        let id = self.st.borrow().canvas.ia.hit_id(x, y);
        if let Some(act) = id.and_then(|id| self.action_for(id, true)) {
            self.perform(act);
        }
    }

    fn on_wheel(&self, delta: i16) {
        let (notes, clips) = self.counts();
        let mut st = self.st.borrow_mut();
        let index = if st.tab == Tab::Saved { 0 } else { 1 };
        let count = if index == 0 { notes } else { clips };
        let visible = if st.expanded { EXPANDED_ROWS } else { COLLAPSED_ROWS };
        let max = (count.saturating_sub(visible)) as f32 * ROW_H;
        if max <= 0.0 {
            return;
        }
        let target = (st.scroll[index].target - delta as f32 / 120.0 * ROW_H * 1.5).clamp(0.0, max);
        st.scroll[index].set(target);
        st.indicator_until = crate::util::now_seconds() + 1.0;
        drop(st);
        self.animate();
    }

    fn screen_point(&self) -> POINT {
        window::cursor_pos()
    }

    fn action_for(&self, id: Id, right: bool) -> Option<Act> {
        let index = ui::index_of(id) as usize;
        let a = app();
        let note = || a.notes.borrow().notes.get(index).cloned();
        let clip = || a.history.borrow().items.get(index).map(|c| c.id.clone());
        let at = self.screen_point();
        Some(match (ui::kind_of(id), right) {
            (K_SAVE, false) => Act::Save,
            (K_SHOT, false) => Act::Screenshot(false),
            (K_SHOT, true) => Act::ScreenshotMenu(at),
            (K_FOLDER, false) => Act::Folder,
            (K_TUCK, false) => Act::Tuck,
            (K_SETTINGS, false) => Act::Settings,
            (K_TAB, false) => Act::Tab(if index == 0 { Tab::Saved } else { Tab::Clipboard }),
            (K_EXPAND, false) => Act::Expand,
            (K_CLEAR, false) => Act::Clear,
            (K_NOTE, false) => Act::CopyNote(note()?),
            (K_NOTE | K_NOTE_EYE | K_NOTE_REVEAL | K_NOTE_DELETE, true) => Act::NoteMenu(note()?, at),
            (K_NOTE_EYE, false) => Act::PreviewNote(note()?),
            (K_NOTE_REVEAL, false) => Act::RevealNote(note()?),
            (K_NOTE_DELETE, false) => Act::DeleteMenu(note()?, at),
            (K_CLIP, false) => Act::CopyClip(clip()?),
            (K_CLIP | K_CLIP_EYE | K_CLIP_SAVE | K_CLIP_REMOVE, true) => Act::ClipMenu(clip()?, at),
            (K_CLIP_EYE, false) => Act::PreviewClip(clip()?),
            (K_CLIP_SAVE, false) => Act::SaveClip(clip()?),
            (K_CLIP_REMOVE, false) => Act::RemoveClip(clip()?),
            (K_UNDO, false) => Act::Undo,
            (K_UNDO_CLOSE, false) => Act::CloseUndo,
            (K_EMPTY_BUTTON, false) => Act::EmptyButton,
            _ => return None,
        })
    }

    fn perform(&self, act: Act) {
        let a = app();
        match act {
            Act::Save => a.save_clipboard(false),
            Act::Screenshot(ocr) => crate::capture::start(ocr),
            Act::ScreenshotMenu(at) => {
                let entries = vec![menu::item(1, "截图"), menu::item(2, "截图并识别文字")];
                match menu::popup(a.hwnd.get(), &entries, at) {
                    Some(1) => crate::capture::start(false),
                    Some(2) => crate::capture::start(true),
                    _ => {}
                }
            }
            Act::Folder => a.open_folder(),
            Act::Tuck => self.toggle_tuck(),
            Act::Settings => crate::settings_window::open(),
            Act::Tab(tab) => self.select_tab(tab),
            Act::Expand => {
                {
                    let mut st = self.st.borrow_mut();
                    st.expanded = !st.expanded;
                    st.scroll[0].set(0.0);
                    st.scroll[1].set(0.0);
                }
                self.relayout(true);
                self.animate();
            }
            Act::Clear => a.clear_clips(),
            Act::CopyNote(note) => a.copy_note(&note),
            Act::PreviewNote(note) => a.preview_note(&note),
            Act::RevealNote(note) => a.reveal_note(&note),
            Act::DeleteMenu(note, at) => {
                let entries = vec![menu::item(1, "从列表移除（保留文件）"), menu::item(2, "移到回收站")];
                match menu::popup(a.hwnd.get(), &entries, at) {
                    Some(1) => a.hide_note(&note),
                    Some(2) => a.trash_note(&note),
                    _ => {}
                }
            }
            Act::NoteMenu(note, at) => {
                let preview_label = a
                    .settings
                    .borrow()
                    .preview_app
                    .as_ref()
                    .and_then(|p| p.file_stem().map(|s| format!("用 {} 打开", s.to_string_lossy())))
                    .unwrap_or_else(|| "预览和编辑".into());
                let entries = vec![
                    menu::item(1, "复制文件"),
                    menu::item(2, &preview_label),
                    menu::item(3, "在资源管理器中显示"),
                    Entry::Separator,
                    menu::item(4, "从列表移除（保留文件）"),
                    menu::item(5, "移到回收站"),
                ];
                match menu::popup(a.hwnd.get(), &entries, at) {
                    Some(1) => a.copy_note(&note),
                    Some(2) => a.preview_note(&note),
                    Some(3) => a.reveal_note(&note),
                    Some(4) => a.hide_note(&note),
                    Some(5) => a.trash_note(&note),
                    _ => {}
                }
            }
            Act::CopyClip(id) => a.copy_clip(&id),
            Act::PreviewClip(id) => crate::clip_preview::toggle(&id),
            Act::SaveClip(id) => a.save_clip(&id),
            Act::RemoveClip(id) => a.remove_clip(&id),
            Act::ClipMenu(id, at) => {
                let item = a.history.borrow().find(&id).cloned();
                let Some(item) = item else { return };
                let mut entries = vec![
                    menu::item(1, "预览"),
                    menu::item(2, if item.is_image() { "放回剪贴板（图片）" } else { "放回剪贴板" }),
                ];
                if item.is_image() {
                    entries.push(menu::item(3, "识别文字"));
                    entries.push(menu::item(4, "在资源管理器中显示"));
                } else {
                    entries.push(menu::item(5, if item.saved_file_name.is_some() { "再存一份 md" } else { "存成 md" }));
                }
                entries.push(Entry::Separator);
                entries.push(menu::item(6, "从记录里删除"));
                match menu::popup(a.hwnd.get(), &entries, at) {
                    Some(1) => crate::clip_preview::toggle(&id),
                    Some(2) => a.copy_clip(&id),
                    Some(3) => a.ocr_clip(&id),
                    Some(4) => a.reveal_clip(&id),
                    Some(5) => a.save_clip(&id),
                    Some(6) => a.remove_clip(&id),
                    _ => {}
                }
            }
            Act::Undo => a.perform_undo(),
            Act::CloseUndo => a.commit_undo(),
            Act::EmptyButton => {
                let (history, listening) = {
                    let s = a.settings.borrow();
                    (s.history, s.double_copy)
                };
                if !listening {
                    a.set_listening(true);
                } else if !history {
                    crate::settings_window::open();
                }
            }
            Act::DragNote(note) => {
                crate::dragdrop::drag_file_out(&note.path);
                a.notes.borrow_mut().refresh();
                a.refresh_panel();
            }
        }
    }

    fn save_position(&self) {
        if self.st.borrow().tucked.is_some() {
            return;
        }
        let r = window::window_rect(self.hwnd());
        app().settings.borrow_mut().panel_position = Some((r.left, r.top));
        app().save_settings();
    }
}

impl Handler for Panel {
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
                let st = self.st.borrow();
                let interactive = st.canvas.ia.hit_id(x, y).is_some_and(|id| ui::kind_of(id) != K_LIST);
                let pinned = app().settings.borrow().panel_pinned;
                if interactive || pinned || st.tucked.is_some() || st.drop.is_some() {
                    Some(LRESULT(HTCLIENT as isize))
                } else {
                    Some(LRESULT(HTCAPTION as isize))
                }
            }
            WM_NCLBUTTONDBLCLK => Some(LRESULT(0)),
            WM_NCMOUSEMOVE => {
                // 鼠标在可拖动的空白处：清掉悬停高亮
                let changed = {
                    let mut st = self.st.borrow_mut();
                    let had = st.canvas.ia.hovered.is_some();
                    st.canvas.ia.mouse_leave();
                    had
                };
                if changed {
                    ui::hide_tooltip();
                    self.animate();
                }
                None
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_SETCURSOR => {
                if crate::util::loword(lp.0 as usize) as u32 == HTCLIENT {
                    ui::apply_cursor(self.st.borrow().canvas.ia.cursor());
                    return Some(LRESULT(1));
                }
                None
            }
            WM_MOUSEMOVE => {
                let (x, y) = self.dip(lp);
                self.on_mouse_move(x, y, wp.0 & 0x0001 != 0);
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.on_mouse_leave();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = self.dip(lp);
                self.on_button_down(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = self.dip(lp);
                self.on_button_up(x, y);
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (x, y) = self.dip(lp);
                self.on_right_click(x, y);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                self.on_wheel(crate::util::wheel_delta(wp));
                Some(LRESULT(0))
            }
            WM_EXITSIZEMOVE => {
                self.save_position();
                None
            }
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                info.ptMinTrackSize = POINT { x: 1, y: 1 };
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let suggested = unsafe { &*(lp.0 as *const RECT) };
                let s = crate::util::loword(wp.0) as f32 / 96.0;
                let h = self.st.borrow().height;
                self.st.borrow_mut().thumbs.clear();
                unsafe {
                    let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, (WIDTH * s) as i32, (h * s) as i32, SWP_NOZORDER | SWP_NOACTIVATE);
                }
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
                self.frame();
                Some(LRESULT(0))
            }
            WM_TIMER => {
                match wp.0 {
                    TIMER_SHRINK => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_SHRINK);
                        }
                        let target = self.target_height();
                        self.apply_height(target);
                    }
                    TIMER_TIP => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_TIP);
                        }
                        let tip = self.st.borrow_mut().canvas.ia.pending_tip();
                        if let Some((text, rect)) = tip {
                            ui::show_tooltip(&text, ui::dip_rect_to_screen(hwnd, rect));
                        }
                    }
                    TIMER_TUCK_BACK => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_TUCK_BACK);
                        }
                        let inside = {
                            let r = window::window_rect(hwnd);
                            let c = window::cursor_pos();
                            c.x >= r.left && c.x < r.right && c.y >= r.top && c.y < r.bottom
                        };
                        if !inside && self.st.borrow().drop.is_none() {
                            self.st.borrow_mut().tuck_out.set(0.0);
                            self.animate();
                        }
                    }
                    TIMER_CLOCK => window::invalidate(hwnd),
                    TIMER_FEEDBACK => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_FEEDBACK);
                        }
                        let mut st = self.st.borrow_mut();
                        st.saved_at = None;
                        st.copied = None;
                        drop(st);
                        window::invalidate(hwnd);
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                crate::dragdrop::revoke_drop_target(hwnd);
                None
            }
            _ => None,
        }
    }
}

// MARK: 画面

fn draw_panel(p: &Painter, st: &mut State, size: Rect, backdrop: bool) {
    let pal = theme::palette();
    let a = app();
    let settings = a.settings.borrow();
    let notes = a.notes.borrow();
    let history = a.history.borrow();
    st.canvas.ia.begin_frame();

    // 背景
    if backdrop {
        p.fill_rect(size, pal.tint);
    } else if window::rounded_corners_supported() {
        p.fill_rect(size, pal.solid);
    } else {
        p.fill_round(size, 10.0, pal.solid);
        p.stroke_round(size.inset(0.5, 0.5), 10.0, pal.border, 1.0);
    }
    // 整块可以拖动，鼠标在列表空白处也算列表
    st.canvas.ia.add(ui::id(K_LIST, 0), Rect::new(0.0, LIST_TOP, size.w, size.h - LIST_TOP));

    let appear = st.appear.value.clamp(0.0, 1.0);
    let scale = 0.97 + 0.03 * appear;
    let m = windows_numerics::Matrix3x2::scale_around(scale, scale, windows_numerics::Vector2 { X: size.w / 2.0, Y: 0.0 });
    let primary = st_primary(st);
    let tab_value = st.tab_spring.value;
    let show_grabber = !settings.panel_pinned && st.tucked.is_none();
    p.with_alpha(appear.max(0.0), || {
        p.transform(m, || {
            // 顶部的小横条：提示可以拖动
            if show_grabber {
                p.fill_round(Rect::centered(size.w / 2.0, 8.0, 36.0, 4.0), 2.0, pal.tertiary.alpha(0.6));
            }
            draw_header(p, &mut st.canvas.ia, &pal, size);
            draw_primary(p, primary, &mut st.canvas.ia, &pal, &settings, size);
            draw_tabs(p, &mut st.canvas.ia, &pal, tab_value, notes.notes.len(), history.items.len(), size);
            draw_lists(p, st, &pal, &settings, &notes.notes, &history.items, size);
            draw_undo(p, st, &pal, size);
        });
    });
    draw_drop(p, st, &pal, size);
    draw_tuck_handle(p, st, &pal, size);
}

struct PrimaryState {
    shake: f32,
    hint: Hint,
    fresh: bool,
    saved: bool,
    on_clipboard: bool,
}

fn st_primary(st: &State) -> PrimaryState {
    PrimaryState { shake: st.shake.value, hint: st.hint, fresh: st.fresh, saved: st.saved_at.is_some(), on_clipboard: st.saved_on_clipboard }
}

fn draw_header(p: &Painter, ia: &mut ui::Interaction, pal: &Palette, size: Rect) {
    icons::app_glyph(p, Rect::new(16.0, 24.0, 22.0, 22.0), pal.dark);
    p.text("ClipMD", Rect::new(46.0, 20.0, 120.0, 30.0), Font::ui(15.5, 700), pal.text, TextOpts::LINE);
    let buttons = [
        (K_SHOT, Icon::Screenshot, "截图（右键可选「截图并识别文字」）"),
        (K_FOLDER, Icon::Folder, "打开保存文件夹"),
        (K_TUCK, Icon::Tuck, "贴边收起"),
        (K_SETTINGS, Icon::Settings, "设置"),
    ];
    let mut x = size.w - 12.0 - 30.0;
    for (kind, icon, tip) in buttons.iter().rev() {
        ui::icon_button(p, ia, pal, ui::id(*kind, 0), Rect::new(x, 20.0, 30.0, 30.0), *icon, tip, pal.secondary);
        x -= 32.0;
    }
}

fn draw_primary(p: &Painter, st: PrimaryState, ia: &mut ui::Interaction, pal: &Palette, settings: &crate::settings::Settings, size: Rect) {
    let rect = Rect::new(12.0 + st.shake, 60.0, size.w - 24.0, 54.0);
    let id = ui::id(K_SAVE, 0);
    ia.add_tip(id, rect, "把剪贴板里的文字存成 .md 文件，并把文件放回剪贴板（或者连点两次 Ctrl+C）");
    let hover = ia.hover(id);
    let pressed = ia.is_pressed(id);
    let (top, bottom) = if st.saved {
        (Color::hex(0x4CD07D), Color::hex(0x23A257))
    } else {
        (theme::BLUE_LIGHT, theme::BLUE)
    };
    let k = if pressed { 0.88 } else { 1.0 + 0.05 * hover };
    let adj = |c: Color| Color { r: (c.r * k).min(1.0), g: (c.g * k).min(1.0), b: (c.b * k).min(1.0), a: 1.0 };
    p.soft_shadow(rect, 15.0, (if st.saved { theme::GREEN } else { theme::BLUE }).alpha(if pal.dark { 0.25 } else { 0.3 }), 6.0, 3.0);
    p.gradient_round(rect, 15.0, adj(top), adj(bottom));
    p.stroke_round(rect.inset(0.5, 0.5), 15.0, Color::WHITE.alpha(0.18), 1.0);
    // 左边的圆形图标
    let circle = Rect::new(rect.x + 12.0, rect.mid_y() - 17.0, 34.0, 34.0);
    p.fill_circle(circle.mid_x(), circle.mid_y(), 17.0, Color::WHITE.alpha(0.22));
    icons::draw(p, if st.saved { Icon::Check } else { Icon::Download }, circle, 17.0, Color::WHITE);
    let title = if st.saved { "已保存为 Markdown" } else { "保存为 Markdown" };
    let subtitle = if st.saved {
        if st.on_clipboard { "文件已复制，Ctrl+V 发送" } else { "单击下面第一条就能复制文件" }
    } else if st.fresh {
        "刚复制了一段文字，点这里存成 md"
    } else {
        st.hint.message(settings.convert_html)
    };
    let keycap = settings.save_shortcut.label();
    let key_font = Font::ui(11.5, 600);
    let key_w = crate::text::measure(&keycap, key_font) + 16.0;
    let text_x = circle.right() + 11.0;
    let text_w = if st.saved { rect.right() - 14.0 - text_x } else { rect.right() - key_w - 12.0 - text_x - 6.0 };
    p.text(title, Rect::new(text_x, rect.y + 8.0, text_w, 22.0), Font::ui(15.0, 700), Color::WHITE, TextOpts::LINE);
    p.text(subtitle, Rect::new(text_x, rect.y + 29.0, text_w, 18.0), Font::ui(12.0, 500), Color::WHITE.alpha(0.88), TextOpts::LINE);
    if !st.saved {
        let key = Rect::new(rect.right() - key_w - 12.0, rect.mid_y() - 12.0, key_w, 24.0);
        p.fill_round(key, 12.0, Color::WHITE.alpha(0.2));
        p.text(&keycap, key, key_font, Color::WHITE, TextOpts::CENTER);
    }
}

fn tab_rects(clip_count: usize) -> (Rect, Rect, String) {
    let font = Font::ui(13.5, 600);
    let clip_label = if clip_count > 0 { format!("剪贴板 {clip_count}") } else { "剪贴板".to_string() };
    let w0 = (crate::text::measure("已保存", font) + 30.0).max(76.0);
    let w1 = (crate::text::measure(&clip_label, font) + 30.0).max(76.0);
    let track = Rect::new(12.0, 124.0, w0 + w1 + 4.0, 32.0);
    (Rect::new(track.x + 2.0, track.y + 2.0, w0, 28.0), Rect::new(track.x + 2.0 + w0, track.y + 2.0, w1, 28.0), clip_label)
}

fn draw_tabs(p: &Painter, ia: &mut ui::Interaction, pal: &Palette, t: f32, _notes: usize, clips: usize, size: Rect) {
    let (r0, r1, clip_label) = tab_rects(clips);
    let track = Rect::new(r0.x - 2.0, r0.y - 2.0, r0.w + r1.w + 4.0, 32.0);
    p.fill_round(track, 16.0, pal.fill);
    let t = t.clamp(-0.1, 1.1);
    let knob = Rect::new(r0.x + (r1.x - r0.x) * t, r0.y, r0.w + (r1.w - r0.w) * t, r0.h);
    if !pal.dark {
        p.soft_shadow(knob, 14.0, Color::BLACK.alpha(0.10), 2.5, 0.8);
    }
    p.fill_round(knob, 14.0, pal.tab_selected);
    let font = Font::ui(13.5, 600);
    for (i, (rect, label)) in [(r0, "已保存".to_string()), (r1, clip_label)].iter().enumerate() {
        let id = ui::id(K_TAB, i as u32);
        ia.add_cursor(id, *rect, Cursor::Hand);
        let selected = if i == 0 { 1.0 - t } else { t };
        let color = pal.secondary.mix(pal.text, selected.clamp(0.0, 1.0).max(ia.hover(id) * 0.6));
        p.text(label, *rect, font, color, TextOpts::CENTER);
    }
    // 剪贴板标签下的"清空"
    if clips > 0 && t > 0.01 {
        p.with_alpha(t.clamp(0.0, 1.0), || {
            let rect = Rect::new(size.w - 12.0 - 48.0, 125.0, 48.0, 30.0);
            ui::text_button(p, ia, pal, ui::id(K_CLEAR, 0), rect, "清空", Font::ui(13.0, 500), pal.secondary);
        });
    }
}

fn draw_lists(p: &Painter, st: &mut State, pal: &Palette, settings: &crate::settings::Settings, notes: &[SavedNote], clips: &[ClipItem], size: Rect) {
    let t = st.tab_spring.value.clamp(0.0, 1.0);
    let now = clipmd_core::time::now_unix();
    let offset = crate::util::utc_offset();
    let undo_h = st.undo_spring.value.clamp(0.0, 1.0) * UNDO_H;
    let list_bottom = size.h - BOTTOM - undo_h;
    let expanded = st.expanded;
    let copied = st.copied.clone();
    let indicator = ((st.indicator_until - crate::util::now_seconds()) / 0.4 + 1.0).clamp(0.0, 1.0) as f32;
    let scrolls = [st.scroll[0].value, st.scroll[1].value];
    let State { canvas, thumbs, .. } = st;
    let ia = &mut canvas.ia;

    for (tab_index, alpha, dx) in [(0usize, 1.0 - t, -18.0 * t), (1usize, t, 18.0 * (1.0 - t))] {
        if alpha <= 0.01 {
            continue;
        }
        let count = if tab_index == 0 { notes.len() } else { clips.len() };
        let visible_rows = count.min(if expanded { EXPANDED_ROWS } else { COLLAPSED_ROWS });
        let footer = count > COLLAPSED_ROWS;
        let list_rect = Rect::new(0.0, LIST_TOP, size.w, (visible_rows as f32 * ROW_H).min((list_bottom - LIST_TOP - if footer { FOOTER_H } else { 0.0 }).max(0.0)));
        let interactive = alpha > 0.5;
        let scroll = scrolls[tab_index];
        p.with_alpha(alpha, || {
            p.translate(dx, 0.0, || {
                if count == 0 {
                    draw_empty(p, ia, pal, settings, tab_index == 0, Rect::new(0.0, LIST_TOP, size.w, EMPTY_H), interactive);
                    return;
                }
                let max_scroll = (count.saturating_sub(visible_rows)) as f32 * ROW_H;
                let top_fade = if scroll > 1.0 { 14.0 } else { 0.0 };
                let bottom_fade = if scroll < max_scroll - 1.0 && count > visible_rows { 14.0 } else { 0.0 };
                p.fade_layer(list_rect, top_fade, bottom_fade, || {
                    let first = (scroll / ROW_H).floor().max(0.0) as usize;
                    let last = (((scroll + list_rect.h) / ROW_H).ceil() as usize).min(count);
                    for i in first..last {
                        let y = LIST_TOP + i as f32 * ROW_H - scroll;
                        let row = Rect::new(0.0, y, size.w, ROW_H);
                        if tab_index == 0 {
                            draw_note_row(p, ia, pal, &notes[i], i, row, now, offset, &copied, interactive && row.bottom() > list_rect.y && row.y < list_rect.bottom());
                        } else {
                            draw_clip_row(p, ia, pal, thumbs, &clips[i], i, row, now, offset, &copied, interactive && row.bottom() > list_rect.y && row.y < list_rect.bottom());
                        }
                    }
                });
                // 滚动条：只在滚动时短暂出现
                if max_scroll > 0.0 && indicator > 0.0 {
                    let track_h = list_rect.h - 8.0;
                    let thumb_h = (track_h * list_rect.h / (list_rect.h + max_scroll)).max(24.0);
                    let thumb_y = list_rect.y + 4.0 + (track_h - thumb_h) * (scroll / max_scroll).clamp(0.0, 1.0);
                    p.fill_round(Rect::new(size.w - 6.0, thumb_y, 3.0, thumb_h), 1.5, pal.text.alpha(0.22 * indicator));
                }
                if footer {
                    let rect = Rect::new(0.0, list_rect.bottom(), size.w, FOOTER_H);
                    let label = if expanded { "收起".to_string() } else { format!("展开（共 {count} 条）") };
                    if interactive {
                        let button = Rect::centered(size.w / 2.0, rect.mid_y(), crate::text::measure(&label, Font::ui(12.5, 500)) + 24.0, 24.0);
                        ui::text_button(p, ia, pal, ui::id(K_EXPAND, 0), button, &label, Font::ui(12.5, 500), pal.secondary);
                    } else {
                        p.text(&label, rect, Font::ui(12.5, 500), pal.secondary, TextOpts::CENTER);
                    }
                }
            });
        });
    }
}

fn draw_empty(p: &Painter, ia: &mut ui::Interaction, pal: &Palette, settings: &crate::settings::Settings, saved: bool, rect: Rect, interactive: bool) {
    let (icon, title, detail, button): (Icon, &str, String, Option<&str>) = if saved {
        (Icon::Document, "保存过的文件会出现在这里", format!("小技巧：连点两次 Ctrl+C 也会直接保存（{}）", settings.save_shortcut.label()), None)
    } else if !settings.double_copy {
        (Icon::Clipboard, "监听复制已关闭", "打开后才会记录复制过的内容".into(), Some("打开监听"))
    } else if !settings.history {
        (Icon::Clipboard, "剪贴板记录已关闭", "在设置里打开「记录剪贴板」".into(), Some("打开设置"))
    } else {
        (Icon::Clipboard, "复制过的文字会记在这里", "不会存成文件，需要时点 ↓ 存成 md".into(), None)
    };
    let top = rect.y + if button.is_some() { 6.0 } else { 14.0 };
    icons::draw(p, icon, Rect::new(0.0, top, rect.w, 26.0), 22.0, pal.tertiary);
    p.text(title, Rect::new(16.0, top + 30.0, rect.w - 32.0, 20.0), Font::ui(13.0, 600), pal.secondary, TextOpts::CENTER);
    p.text(&detail, Rect::new(16.0, top + 50.0, rect.w - 32.0, 18.0), Font::ui(12.0, 400), pal.tertiary, TextOpts::CENTER);
    if let Some(label) = button {
        let b = Rect::centered(rect.w / 2.0, top + 84.0, crate::text::measure(label, Font::ui(12.5, 600)) + 28.0, 26.0);
        if interactive {
            ui::pill_button(p, ia, pal, ui::id(K_EMPTY_BUTTON, 0), b, label, false);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn row_frame(p: &Painter, ia: &mut ui::Interaction, pal: &Palette, id: Id, row: Rect, interactive: bool, tip: &str) -> f32 {
    let bg = Rect::new(8.0, row.y + 2.0, row.w - 16.0, row.h - 4.0);
    if interactive {
        ia.add_tip(id, bg, tip);
    }
    let hover = ia.hover(id);
    // 鼠标在行里的按钮上时，整行也算悬停
    let row_hover = if ia.mouse.is_some_and(|(x, y)| bg.contains(x, y)) && interactive { hover.max(0.6) } else { hover };
    if row_hover > 0.0 {
        p.fill_round(bg, 10.0, if ia.is_pressed(id) { pal.pressed } else { pal.hover.alpha(row_hover) });
    }
    row_hover
}

fn row_buttons(p: &Painter, ia: &mut ui::Interaction, pal: &Palette, row: Rect, row_hover: f32, buttons: &[(u32, u32, Icon, &str, bool)], interactive: bool) -> f32 {
    let mut x = row.w - 12.0 - 30.0;
    for &(kind, index, icon, tip, highlighted) in buttons.iter().rev() {
        let rect = Rect::new(x, row.mid_y() - 15.0, 30.0, 30.0);
        if highlighted {
            p.fill_circle(rect.mid_x(), rect.mid_y(), 15.0, pal.accent.alpha(0.16));
        }
        let base = if highlighted { pal.accent } else { pal.tertiary.mix(pal.secondary, row_hover) };
        if interactive {
            ui::icon_button(p, ia, pal, ui::id(kind, index), rect, icon, tip, base);
        } else {
            icons::draw(p, icon, rect, 16.0, base);
        }
        x -= 30.0;
    }
    x + 30.0
}

#[allow(clippy::too_many_arguments)]
fn draw_note_row(p: &Painter, ia: &mut ui::Interaction, pal: &Palette, note: &SavedNote, index: usize, row: Rect, now: i64, offset: i64, copied: &Option<(Tab, String, f64)>, interactive: bool) {
    let i = index as u32;
    let hover = row_frame(p, ia, pal, ui::id(K_NOTE, i), row, interactive, "单击复制文件，或拖到聊天窗口里；拖到回收站会删除；右键有更多操作");
    let icon_box = Rect::new(18.0, row.y + 7.0, 32.0, 32.0);
    p.fill_round(icon_box, 8.0, pal.fill);
    icons::draw(p, Icon::TextLines, icon_box, 16.0, pal.secondary);
    let buttons_x = row_buttons(
        p,
        ia,
        pal,
        row,
        hover,
        &[(K_NOTE_EYE, i, Icon::Eye, "预览和编辑", false), (K_NOTE_REVEAL, i, Icon::Search, "在资源管理器中显示", false), (K_NOTE_DELETE, i, Icon::Trash, "删除", false)],
        interactive,
    );
    let text_x = icon_box.right() + 10.0;
    let text_w = buttons_x - text_x - 4.0;
    p.text(&note.title, Rect::new(text_x, row.y + 5.0, text_w, 20.0), Font::ui(14.0, 600), pal.text, TextOpts::LINE);
    let is_copied = copied.as_ref().is_some_and(|(tab, key, _)| *tab == Tab::Saved && *key == note.file_name());
    let (meta, color) = if is_copied {
        ("文件已复制，Ctrl+V 发送".to_string(), pal.accent_text)
    } else {
        (
            format!(
                "{} · {} 字",
                clipmd_core::time::relative_label(note.date, now, offset),
                clipmd_core::text::group_thousands(note.character_count)
            ),
            pal.secondary,
        )
    };
    p.text(&meta, Rect::new(text_x, row.y + 25.0, text_w, 17.0), Font::ui(12.0, 400), color, TextOpts::LINE);
}

#[allow(clippy::too_many_arguments)]
fn draw_clip_row(
    p: &Painter,
    ia: &mut ui::Interaction,
    pal: &Palette,
    thumbs: &mut HashMap<String, Option<ID2D1Bitmap>>,
    item: &ClipItem,
    index: usize,
    row: Rect,
    now: i64,
    offset: i64,
    copied: &Option<(Tab, String, f64)>,
    interactive: bool,
) {
    let i = index as u32;
    let hover = row_frame(p, ia, pal, ui::id(K_CLIP, i), row, interactive, "单击放回剪贴板；右键有更多操作");
    let icon_box = Rect::new(18.0, row.y + 7.0, 32.0, 32.0);
    let mut drew_thumb = false;
    if item.is_image() {
        let thumb = thumbs.entry(item.id.clone()).or_insert_with(|| {
            let path = crate::history::images_dir().join(item.image_file.as_deref().unwrap_or_default());
            crate::images::decode_file(&path, Some(96)).ok().and_then(|img| crate::gfx::bitmap_from_bgra(img.width, img.height, img.stride(), &img.pixels))
        });
        if let Some(bitmap) = thumb {
            let size = unsafe { bitmap.GetSize() };
            // 按比例填满方块，多出来的裁掉
            let ratio = (icon_box.w / size.width).max(icon_box.h / size.height);
            let (dw, dh) = (size.width * ratio, size.height * ratio);
            let dest = Rect::centered(icon_box.mid_x(), icon_box.mid_y(), dw, dh);
            p.clip_round(icon_box, 8.0, || p.bitmap(bitmap, dest, 1.0, true));
            p.stroke_round(icon_box.inset(0.25, 0.25), 8.0, pal.border, 0.5);
            drew_thumb = true;
        }
    }
    if !drew_thumb {
        p.fill_round(icon_box, 8.0, pal.fill);
        icons::draw(p, if item.is_image() { Icon::Photo } else { Icon::TextLines }, icon_box, 16.0, pal.secondary);
    }
    let saved = item.saved_file_name.is_some();
    let middle = if item.is_image() {
        (K_CLIP_SAVE, i, Icon::TextScan, "识别图片里的文字", false)
    } else if saved {
        (K_CLIP_SAVE, i, Icon::CheckCircle, "已存成 md（点了再存一份）", true)
    } else {
        (K_CLIP_SAVE, i, Icon::SaveDoc, "存成 md", false)
    };
    let buttons_x = row_buttons(
        p,
        ia,
        pal,
        row,
        hover,
        &[(K_CLIP_EYE, i, Icon::Eye, if item.is_image() { "预览图片" } else { "预览全文" }, false), middle, (K_CLIP_REMOVE, i, Icon::Close, "从记录里删除", false)],
        interactive,
    );
    let text_x = icon_box.right() + 10.0;
    let text_w = buttons_x - text_x - 4.0;
    let title = item.title();
    p.text(if title.is_empty() { "（空白）" } else { &title }, Rect::new(text_x, row.y + 5.0, text_w, 20.0), Font::ui(14.0, 600), pal.text, TextOpts::LINE);
    let is_copied = copied.as_ref().is_some_and(|(tab, key, _)| *tab == Tab::Clipboard && *key == item.id);
    let (meta, color) = if is_copied {
        ((if item.is_image() { "图片已放回剪贴板，Ctrl+V 粘贴" } else { "已放回剪贴板，Ctrl+V 粘贴" }).to_string(), pal.accent_text)
    } else {
        let mut parts = vec![clipmd_core::time::relative_label(item.date, now, offset)];
        if !item.is_image() {
            parts.push(format!("{} 字", clipmd_core::text::group_thousands(item.character_count)));
        }
        if let Some(source) = &item.source_app {
            parts.push(source.clone());
        }
        if saved {
            parts.push("已存".into());
        }
        (parts.join(" · "), pal.secondary)
    };
    p.text(&meta, Rect::new(text_x, row.y + 25.0, text_w, 17.0), Font::ui(12.0, 400), color, TextOpts::LINE);
}

fn draw_undo(p: &Painter, st: &mut State, pal: &Palette, size: Rect) {
    let t = st.undo_spring.value.clamp(0.0, 1.0);
    if t <= 0.01 {
        return;
    }
    let message = st.undo.clone().unwrap_or_default();
    let ia = &mut st.canvas.ia;
    let rect = Rect::new(12.0, size.h - BOTTOM - UNDO_H + 4.0 + (1.0 - t) * 12.0, size.w - 24.0, UNDO_H - 8.0);
    p.with_alpha(t, || {
        p.fill_round(rect, 12.0, pal.fill_strong);
        let undo_w = 52.0;
        let close = Rect::new(rect.right() - 32.0, rect.mid_y() - 13.0, 26.0, 26.0);
        let undo = Rect::new(close.x - undo_w - 2.0, rect.mid_y() - 13.0, undo_w, 26.0);
        p.text(&message, Rect::new(rect.x + 12.0, rect.y, undo.x - rect.x - 16.0, rect.h), Font::ui(12.5, 500), pal.text, TextOpts::LINE);
        if !message.is_empty() {
            ui::text_button(p, ia, pal, ui::id(K_UNDO, 0), undo, "撤销", Font::ui(13.0, 700), pal.accent_text);
            ui::icon_button(p, ia, pal, ui::id(K_UNDO_CLOSE, 0), close, Icon::Close, "关闭", pal.secondary);
        }
    });
}

fn draw_drop(p: &Painter, st: &State, pal: &Palette, size: Rect) {
    let t = st.drop_spring.value.clamp(0.0, 1.0);
    let Some(view) = &st.drop else { return };
    if t <= 0.01 {
        return;
    }
    p.with_alpha(t, || {
        p.fill_rect(size, if pal.dark { Color::hex(0x18191B).alpha(0.86) } else { Color::hex(0xF6F7F9).alpha(0.9) });
        match view {
            DropView::Zones(active) => {
                let half = (size.h - 24.0) / 2.0;
                let zones = [
                    (Zone::Save, Rect::new(12.0, 10.0, size.w - 24.0, half - 2.0), Icon::SaveDoc, "存成 Markdown", "存成 .md 文件，网页格式也会保留"),
                    (Zone::Clipboard, Rect::new(12.0, 14.0 + half, size.w - 24.0, half - 2.0), Icon::Clipboard, "放到剪贴板", "不存文件，Ctrl+V 直接粘贴"),
                ];
                for (zone, rect, icon, title, detail) in zones {
                    let on = *active == Some(zone);
                    p.fill_round(rect, 16.0, if on { pal.accent.alpha(0.16) } else { pal.fill });
                    p.stroke_round(rect.inset(1.0, 1.0), 16.0, if on { pal.accent } else { pal.border }, if on { 2.0 } else { 1.0 });
                    let color = if on { pal.accent_text } else { pal.secondary };
                    icons::draw(p, icon, Rect::new(rect.x, rect.mid_y() - 34.0, rect.w, 28.0), 24.0, color);
                    p.text(title, Rect::new(rect.x, rect.mid_y() - 2.0, rect.w, 22.0), Font::ui(15.0, 700), if on { pal.text } else { pal.secondary }, TextOpts::CENTER);
                    p.text(detail, Rect::new(rect.x + 10.0, rect.mid_y() + 20.0, rect.w - 20.0, 18.0), Font::ui(12.0, 400), pal.tertiary, TextOpts::CENTER);
                }
            }
            DropView::Unsupported(message) => {
                let rect = size.inset(12.0, 10.0);
                p.fill_round(rect, 16.0, pal.fill);
                p.text("这个保存不了", Rect::new(rect.x, rect.mid_y() - 24.0, rect.w, 22.0), Font::ui(15.0, 700), pal.secondary, TextOpts::CENTER);
                p.text(message, Rect::new(rect.x + 16.0, rect.mid_y() + 2.0, rect.w - 32.0, 40.0), Font::ui(12.0, 400), pal.tertiary, TextOpts::WRAP_CENTER);
            }
        }
    });
}

/// 收起时露出的那一条上画个竖着的小把手
fn draw_tuck_handle(p: &Painter, st: &State, pal: &Palette, size: Rect) {
    let Some(edge) = st.tucked else { return };
    let t = 1.0 - st.tuck_out.value.clamp(0.0, 1.0);
    if t <= 0.01 {
        return;
    }
    let x = match edge {
        Edge::Right => HANDLE_W / 2.0,
        Edge::Left => size.w - HANDLE_W / 2.0,
    };
    p.with_alpha(t, || {
        p.fill_round(Rect::centered(x, size.h / 2.0, 4.0, 44.0), 2.0, pal.secondary.alpha(0.7));
    });
}
