//! 预览和编辑窗口：预览 / 编辑 / 并排三种模式，停止输入后自动保存，原生渲染 LaTeX 公式。
//! 每个文件一个窗口，再点同一个文件就把已经开着的窗口叫到前面。

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::{self, FrameTimer, Spring};
use crate::editor::Editor;
use crate::gfx::{Color, Rect, Surface};
use crate::icons::Icon;
use crate::markdown_view::DocView;
use crate::text::{Font, TextOpts};
use crate::theme;
use crate::ui::{self, Interaction};
use crate::window::{self, Handler};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Preview,
    Edit,
    Split,
}

const TOOLBAR_H: f32 = 52.0;
const EDITOR_ID: usize = 100;
const TIMER_SAVE: usize = 1;
const TIMER_STATUS: usize = 2;
const TIMER_TIP: usize = 3;

const K_MODE: u32 = 1;
const K_COPY: u32 = 2;
const K_REVEAL: u32 = 3;
const K_OPEN: u32 = 4;

struct State {
    path: PathBuf,
    mode: Mode,
    mode_spring: Spring,
    doc: DocView,
    editor: Option<Editor>,
    surface: Surface,
    ia: Interaction,
    scroll: Spring,
    timer: FrameTimer,
    status: Option<String>,
    dirty: bool,
    indicator_until: f64,
}

pub struct Preview {
    hwnd: Cell<HWND>,
    st: RefCell<State>,
}

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Preview>>> = const { RefCell::new(Vec::new()) };
}

pub fn open(path: &Path) {
    if let Some(existing) = WINDOWS.with(|w| w.borrow().iter().find(|p| p.st.borrow().path == path).cloned()) {
        let hwnd = existing.hwnd.get();
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
        window::force_foreground(hwnd);
        return;
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let preview = Rc::new(Preview {
        hwnd: Cell::new(HWND::default()),
        st: RefCell::new(State {
            path: path.to_path_buf(),
            mode: Mode::Preview,
            mode_spring: Spring::with(0.0, 0.3, 0.9),
            doc: DocView::new(&text, 15.0),
            editor: None,
            surface: Surface::new_direct(HWND::default()),
            ia: Interaction::default(),
            scroll: Spring::with(0.0, 0.22, 1.0),
            timer: FrameTimer::new(),
            status: None,
            dirty: false,
            indicator_until: 0.0,
        }),
    });
    let title = path.file_stem().map(|s| clipmd_core::note_writer::display_title(&s.to_string_lossy())).unwrap_or_default();
    let cursor = window::cursor_pos();
    let (work, _) = window::monitor_rects(cursor);
    let scale = window::monitor_dpi_at(cursor) as f32 / 96.0;
    let (w, h) = ((780.0 * scale) as i32, ((860.0 * scale) as i32).min(work.bottom - work.top - 40));
    let offset = WINDOWS.with(|w| w.borrow().len()) as i32 * (24.0 * scale) as i32;
    let x = work.left + (work.right - work.left - w) / 2 + offset;
    let y = work.top + (work.bottom - work.top - h) / 2 + offset;
    let Ok(hwnd) = window::create(&format!("{title} — ClipMD"), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, WINDOW_EX_STYLE(0), (x, y, w, h), None, preview.clone()) else { return };
    preview.hwnd.set(hwnd);
    {
        let mut st = preview.st.borrow_mut();
        st.surface = Surface::new_direct(hwnd);
        if let Some(editor) = Editor::new(hwnd, EDITOR_ID) {
            editor.set_text(&text);
            st.editor = Some(editor);
        }
    }
    window::style_window(hwnd, theme::is_dark(), window::Backdrop::None, false);
    preview.layout_children();
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    window::force_foreground(hwnd);
    WINDOWS.with(|w| w.borrow_mut().push(preview));
}

/// 文件要被删除了：关掉它的预览
pub fn close_for(path: &Path) {
    let found = WINDOWS.with(|w| w.borrow().iter().find(|p| p.st.borrow().path == path).cloned());
    if let Some(p) = found {
        unsafe {
            let _ = DestroyWindow(p.hwnd.get());
        }
    }
}

pub fn restyle_all() {
    let all: Vec<Rc<Preview>> = WINDOWS.with(|w| w.borrow().clone());
    for p in all {
        let hwnd = p.hwnd.get();
        window::style_window(hwnd, theme::is_dark(), window::Backdrop::None, false);
        if let Some(editor) = &p.st.borrow().editor {
            editor.apply_theme(theme::is_dark());
        }
        window::invalidate(hwnd);
    }
}

pub fn count() -> usize {
    WINDOWS.with(|w| w.borrow().len())
}

/// 自检：最新打开的预览窗口
pub fn latest() -> Option<(HWND, Rc<dyn Fn(Mode)>)> {
    let p = WINDOWS.with(|w| w.borrow().last().cloned())?;
    let hwnd = p.hwnd.get();
    let setter: Rc<dyn Fn(Mode)> = Rc::new(move |mode| p.set_mode(mode));
    Some((hwnd, setter))
}

pub fn close_all() {
    let all: Vec<Rc<Preview>> = WINDOWS.with(|w| w.borrow().clone());
    for p in all {
        unsafe {
            let _ = DestroyWindow(p.hwnd.get());
        }
    }
}

impl Preview {
    fn size_dip(&self) -> Rect {
        let hwnd = self.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        let s = window::scale(hwnd);
        Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s)
    }

    /// 预览区（并排时是右半边）
    fn preview_rect(&self, mode: Mode) -> Rect {
        let size = self.size_dip();
        let body = Rect::new(0.0, TOOLBAR_H, size.w, (size.h - TOOLBAR_H).max(0.0));
        match mode {
            Mode::Split => Rect::new(body.w / 2.0, body.y, body.w / 2.0, body.h),
            _ => body,
        }
    }

    fn layout_children(&self) {
        let hwnd = self.hwnd.get();
        let s = window::scale(hwnd);
        let size = self.size_dip();
        let st = self.st.borrow();
        if let Some(editor) = &st.editor {
            let body = Rect::new(0.0, TOOLBAR_H, size.w, (size.h - TOOLBAR_H).max(0.0));
            match st.mode {
                Mode::Preview => editor.show(false),
                Mode::Edit => {
                    editor.place(body, s);
                    editor.show(true);
                }
                Mode::Split => {
                    editor.place(Rect::new(0.0, body.y, (body.w / 2.0).floor(), body.h), s);
                    editor.show(true);
                }
            }
        }
    }

    fn set_mode(&self, mode: Mode) {
        {
            let mut st = self.st.borrow_mut();
            if st.mode == mode {
                return;
            }
            // 从编辑切回预览：用编辑框里最新的内容
            if st.dirty || mode != Mode::Edit {
                if let Some(text) = st.editor.as_ref().map(|e| e.text()) {
                    st.doc.set_source(&text);
                }
            }
            st.mode = mode;
            st.mode_spring.set(match mode {
                Mode::Preview => 0.0,
                Mode::Edit => 1.0,
                Mode::Split => 2.0,
            });
        }
        self.layout_children();
        if mode != Mode::Preview {
            if let Some(editor) = &self.st.borrow().editor {
                editor.focus();
            }
        }
        window::invalidate(self.hwnd.get());
        anim::request_frame(self.hwnd.get());
    }

    fn save_now(&self) {
        let (path, text) = {
            let st = self.st.borrow();
            if !st.dirty {
                return;
            }
            (st.path.clone(), st.editor.as_ref().map(|e| e.text()).unwrap_or_default())
        };
        match clipmd_core::note_writer::overwrite(&text, &path) {
            Ok(()) => {
                let mut st = self.st.borrow_mut();
                st.dirty = false;
                st.status = Some("已自动保存".into());
                st.doc.set_source(&text);
            }
            Err(e) => self.st.borrow_mut().status = Some(format!("保存失败：{e}")),
        }
        unsafe {
            SetTimer(Some(self.hwnd.get()), TIMER_STATUS, 2000, None);
        }
        crate::app::app().notes.borrow_mut().refresh();
        crate::app::app().refresh_panel();
        window::invalidate(self.hwnd.get());
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
        let size = self.size_dip();
        let mode_now = self.st.borrow().mode;
        let preview_rect = self.preview_rect(mode_now);
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let mut surface = std::mem::replace(&mut st.surface, Surface::new_direct(hwnd));
        let content_w = (preview_rect.w - 64.0).clamp(200.0, 760.0);
        st.doc.layout(content_w);
        surface.render(w as u32, h as u32, dpi, dark, |p| {
            let bg = if dark { Color::hex(0x1F2023) } else { Color::hex(0xFFFFFF) };
            let bar_bg = if dark { Color::hex(0x26272B) } else { Color::hex(0xF6F7F9) };
            p.fill_rect(size, bg);
            // 预览内容
            if st.mode != Mode::Edit {
                let max_scroll = (st.doc.height + 48.0 - preview_rect.h).max(0.0);
                let scroll = st.scroll.value.clamp(0.0, max_scroll);
                let x = preview_rect.x + ((preview_rect.w - content_w) / 2.0).max(24.0);
                p.clip(preview_rect, || {
                    st.doc.draw(p, &pal, (x, preview_rect.y + 24.0 - scroll), preview_rect);
                });
                let indicator = ((st.indicator_until - crate::util::now_seconds()) / 0.4 + 1.0).clamp(0.0, 1.0) as f32;
                if max_scroll > 0.0 && indicator > 0.0 {
                    let track = preview_rect.h - 12.0;
                    let thumb = (track * preview_rect.h / (preview_rect.h + max_scroll)).max(30.0);
                    let ty = preview_rect.y + 6.0 + (track - thumb) * (scroll / max_scroll);
                    p.fill_round(Rect::new(preview_rect.right() - 7.0, ty, 4.0, thumb), 2.0, pal.text.alpha(0.25 * indicator));
                }
                if st.mode == Mode::Split {
                    p.fill_rect(Rect::new(preview_rect.x, preview_rect.y, 1.0, preview_rect.h), pal.separator);
                }
            }
            // 工具栏
            p.fill_rect(Rect::new(0.0, 0.0, size.w, TOOLBAR_H), bar_bg);
            p.fill_rect(Rect::new(0.0, TOOLBAR_H - 1.0, size.w, 1.0), pal.separator);
            st.ia.begin_frame();
            ui::segmented(p, &mut st.ia, &pal, K_MODE, Rect::new(16.0, 11.0, 228.0, 30.0), &["预览", "编辑", "并排"], st.mode_spring.value, Font::ui(13.0, 500));
            let mut x = size.w - 16.0 - 34.0;
            for (kind, icon, tip) in [(K_OPEN, Icon::ArrowOut, "用默认程序打开"), (K_REVEAL, Icon::Folder, "在资源管理器中显示"), (K_COPY, Icon::Copy, "复制文件")] {
                ui::icon_button(p, &mut st.ia, &pal, ui::id(kind, 0), Rect::new(x, 9.0, 34.0, 34.0), icon, tip, pal.secondary);
                x -= 38.0;
            }
            if let Some(status) = &st.status {
                p.text(status, Rect::new(260.0, 0.0, (x - 270.0).max(0.0), TOOLBAR_H), Font::ui(12.5, 400), pal.secondary, TextOpts::LINE);
            } else if st.dirty {
                p.text("正在编辑…", Rect::new(260.0, 0.0, (x - 270.0).max(0.0), TOOLBAR_H), Font::ui(12.5, 400), pal.tertiary, TextOpts::LINE);
            }
        });
        st.surface = surface;
    }

    fn frame(&self) {
        let moving = {
            let mut st = self.st.borrow_mut();
            let dt = st.timer.tick();
            let mut moving = st.mode_spring.step(dt) | st.scroll.step(dt) | st.ia.step(dt);
            if crate::util::now_seconds() < st.indicator_until + 0.4 {
                moving = true;
            }
            moving
        };
        self.paint();
        if moving {
            anim::request_frame(self.hwnd.get());
        } else {
            self.st.borrow_mut().timer.reset();
        }
    }

    fn click(&self, id: ui::Id) {
        let path = self.st.borrow().path.clone();
        match ui::kind_of(id) {
            K_MODE => self.set_mode(match ui::index_of(id) {
                0 => Mode::Preview,
                1 => Mode::Edit,
                _ => Mode::Split,
            }),
            K_COPY => {
                self.save_now();
                crate::clipboard::write_file(crate::app::app().hwnd.get(), &path);
                self.st.borrow_mut().status = Some("文件已复制，Ctrl+V 发送".into());
                unsafe {
                    SetTimer(Some(self.hwnd.get()), TIMER_STATUS, 2200, None);
                }
            }
            K_REVEAL => crate::notes::reveal(&path),
            K_OPEN => {
                self.save_now();
                crate::notes::open_path(&path, None);
            }
            _ => {}
        }
        window::invalidate(self.hwnd.get());
    }
}

impl Handler for Preview {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => {
                self.paint();
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_SIZE => {
                self.layout_children();
                self.paint();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                let s = window::scale(hwnd);
                info.ptMinTrackSize = POINT { x: (480.0 * s) as i32, y: (320.0 * s) as i32 };
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let r = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                window::track_mouse_leave(hwnd);
                let (x, y) = window::mouse_dip(hwnd, lp);
                let changed = self.st.borrow_mut().ia.mouse_move(x, y);
                if changed {
                    ui::hide_tooltip();
                    if self.st.borrow().ia.wants_tip_timer() {
                        unsafe {
                            SetTimer(Some(hwnd), TIMER_TIP, 600, None);
                        }
                    }
                    anim::request_frame(hwnd);
                }
                // 鼠标在链接上显示小手
                let mode = self.st.borrow().mode;
                let pr = self.preview_rect(mode);
                if mode != Mode::Edit && pr.contains(x, y) {
                    let st = self.st.borrow();
                    let content_w = (pr.w - 64.0).clamp(200.0, 760.0);
                    let cx = pr.x + ((pr.w - content_w) / 2.0).max(24.0);
                    if st.doc.link_at(x - cx, y - (pr.y + 24.0 - st.scroll.value)).is_some() {
                        window::set_cursor(IDC_HAND);
                    }
                }
                Some(LRESULT(0))
            }
            windows::Win32::UI::Controls::WM_MOUSELEAVE => {
                ui::hide_tooltip();
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
                window::invalidate(hwnd);
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
                    let mode = self.st.borrow().mode;
                    let pr = self.preview_rect(mode);
                    if mode != Mode::Edit && pr.contains(x, y) {
                        let link = {
                            let st = self.st.borrow();
                            let content_w = (pr.w - 64.0).clamp(200.0, 760.0);
                            let cx = pr.x + ((pr.w - content_w) / 2.0).max(24.0);
                            st.doc.link_at(x - cx, y - (pr.y + 24.0 - st.scroll.value))
                        };
                        if let Some(url) = link.filter(|u| u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")) {
                            crate::notes::open_path(Path::new(&url), None);
                        }
                    }
                }
                window::invalidate(hwnd);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = crate::util::wheel_delta(wp) as f32;
                {
                    let mut st = self.st.borrow_mut();
                    let mode = st.mode;
                    drop(st);
                    let pr = self.preview_rect(mode);
                    st = self.st.borrow_mut();
                    let max = (st.doc.height + 48.0 - pr.h).max(0.0);
                    let target = (st.scroll.target - delta / 120.0 * 72.0).clamp(0.0, max);
                    st.scroll.set(target);
                    st.indicator_until = crate::util::now_seconds() + 1.0;
                }
                anim::request_frame(hwnd);
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                match wp.0 as u16 {
                    0x31 if ctrl => self.set_mode(Mode::Preview),
                    0x32 if ctrl => self.set_mode(Mode::Edit),
                    0x33 if ctrl => self.set_mode(Mode::Split),
                    0x57 if ctrl => unsafe {
                        let _ = DestroyWindow(hwnd);
                    },
                    0x1B if self.st.borrow().mode == Mode::Preview => unsafe {
                        let _ = DestroyWindow(hwnd);
                    },
                    0x22 | 0x21 => {
                        let page = if wp.0 == 0x22 { 1.0 } else { -1.0 };
                        let h = self.preview_rect(Mode::Preview).h;
                        let mut st = self.st.borrow_mut();
                        let max = (st.doc.height + 48.0 - h).max(0.0);
                        let t = (st.scroll.target + page * h * 0.85).clamp(0.0, max);
                        st.scroll.set(t);
                        drop(st);
                        anim::request_frame(hwnd);
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_COMMAND => {
                // 编辑框内容变了：停止输入 0.7 秒后保存，并排时同步刷新预览
                if crate::util::loword(wp.0) as usize == EDITOR_ID && crate::util::hiword(wp.0) as u32 == EN_CHANGE {
                    self.st.borrow_mut().dirty = true;
                    self.st.borrow_mut().status = None;
                    unsafe {
                        SetTimer(Some(hwnd), TIMER_SAVE, 700, None);
                    }
                    window::invalidate(hwnd);
                }
                Some(LRESULT(0))
            }
            WM_TIMER => {
                match wp.0 {
                    TIMER_SAVE => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_SAVE);
                        }
                        self.save_now();
                    }
                    TIMER_STATUS => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_STATUS);
                        }
                        self.st.borrow_mut().status = None;
                        window::invalidate(hwnd);
                    }
                    TIMER_TIP => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_TIP);
                        }
                        let tip = self.st.borrow_mut().ia.pending_tip();
                        if let Some((text, rect)) = tip {
                            ui::show_tooltip(&text, ui::dip_rect_to_screen(hwnd, rect));
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            anim::WM_FRAME => {
                self.frame();
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                // 回到窗口时，如果文件被别的程序改过（又没在编辑），重新读一遍
                if crate::util::loword(wp.0) != 0 {
                    let (dirty, path) = {
                        let st = self.st.borrow();
                        (st.dirty, st.path.clone())
                    };
                    if !dirty {
                        if let Ok(text) = std::fs::read_to_string(&path) {
                            let changed = self.st.borrow().editor.as_ref().map(|e| e.text() != text.replace("\r\n", "\n")).unwrap_or(false);
                            if changed {
                                let mut st = self.st.borrow_mut();
                                if let Some(editor) = &st.editor {
                                    editor.set_text(&text);
                                }
                                st.doc.set_source(&text);
                                drop(st);
                                window::invalidate(hwnd);
                            }
                        }
                    }
                }
                None
            }
            WM_CLOSE => {
                self.save_now();
                None
            }
            WM_DESTROY => {
                ui::hide_tooltip();
                self.save_now();
                self.st.borrow_mut().surface.release();
                WINDOWS.with(|w| w.borrow_mut().retain(|p| p.hwnd.get() != hwnd));
                None
            }
            _ => None,
        }
    }
}
