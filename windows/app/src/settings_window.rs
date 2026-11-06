//! 设置窗口：保存位置、网页转 Markdown、快捷键、预览方式、监听复制和剪贴板记录、外观、悬浮窗、开机启动。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::anim::{self, FrameTimer, Spring};
use crate::app::app;
use crate::gfx::{Color, Painter, Rect, Surface};
use crate::hotkeys::{self, Action, Shortcut};
use crate::text::{self, Font, TextOpts};
use crate::theme::{self, Appearance, Palette};
use crate::ui::{self, Interaction};
use crate::window::{self, Handler};

const WIDTH: f32 = 580.0;
const TIMER_TIP: usize = 1;

const K_CHANGE_FOLDER: u32 = 1;
const K_OPEN_FOLDER: u32 = 2;
const K_CONVERT: u32 = 3;
const K_RECORD: u32 = 4;
const K_RESET: u32 = 5;
const K_PREVIEW: u32 = 6;
const K_PICK_APP: u32 = 7;
const K_LISTEN: u32 = 8;
const K_HISTORY: u32 = 9;
const K_CLEAR: u32 = 10;
const K_APPEARANCE: u32 = 11;
const K_PINNED: u32 = 12;
const K_LOGIN: u32 = 13;
const K_GITHUB: u32 = 14;
const K_DATA: u32 = 15;

struct State {
    surface: Surface,
    ia: Interaction,
    timer: FrameTimer,
    toggles: HashMap<u32, Spring>,
    appearance_slide: Spring,
    preview_slide: Spring,
    recording: Option<Action>,
    message: Option<(Action, String)>,
    scroll: Spring,
    content_h: f32,
}

pub struct SettingsWindow {
    hwnd: Cell<HWND>,
    st: RefCell<State>,
}

thread_local! {
    static WINDOW: RefCell<Option<Rc<SettingsWindow>>> = const { RefCell::new(None) };
}

fn current() -> Option<Rc<SettingsWindow>> {
    WINDOW.with(|w| w.borrow().clone())
}

pub fn open() {
    if let Some(existing) = current() {
        window::force_foreground(existing.hwnd.get());
        return;
    }
    let win = Rc::new(SettingsWindow {
        hwnd: Cell::new(HWND::default()),
        st: RefCell::new(State {
            surface: Surface::new_direct(HWND::default()),
            ia: Interaction::default(),
            timer: FrameTimer::new(),
            toggles: HashMap::new(),
            appearance_slide: Spring::with(appearance_index() as f32, 0.3, 0.9),
            preview_slide: Spring::with(if app().settings.borrow().preview_app.is_some() { 1.0 } else { 0.0 }, 0.3, 0.9),
            recording: None,
            message: None,
            scroll: Spring::with(0.0, 0.22, 1.0),
            content_h: 0.0,
        }),
    });
    let cursor = window::cursor_pos();
    let (work, _) = window::monitor_rects(cursor);
    let scale = window::monitor_dpi_at(cursor) as f32 / 96.0;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_THICKFRAME;
    // 先按客户区大小算出窗口大小
    let mut rect = RECT { left: 0, top: 0, right: (WIDTH * scale) as i32, bottom: ((860.0 * scale) as i32).min(work.bottom - work.top - 60) };
    unsafe {
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(&mut rect, style, false, WINDOW_EX_STYLE(0), (scale * 96.0) as u32);
    }
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.top + (work.bottom - work.top - h) / 2;
    let Ok(hwnd) = window::create("ClipMD 设置", style, WINDOW_EX_STYLE(0), (x, y, w, h), None, win.clone()) else { return };
    win.hwnd.set(hwnd);
    win.st.borrow_mut().surface = Surface::new_direct(hwnd);
    window::style_window(hwnd, theme::is_dark(), window::Backdrop::None, false);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    window::force_foreground(hwnd);
    WINDOW.with(|w| *w.borrow_mut() = Some(win));
}

pub fn refresh() {
    if let Some(win) = current() {
        window::invalidate(win.hwnd.get());
        anim::request_frame(win.hwnd.get());
    }
}

pub fn restyle() {
    if let Some(win) = current() {
        window::style_window(win.hwnd.get(), theme::is_dark(), window::Backdrop::None, false);
        window::invalidate(win.hwnd.get());
    }
}

pub fn hwnd() -> Option<HWND> {
    current().map(|w| w.hwnd.get())
}

fn appearance_index() -> usize {
    match app().settings.borrow().appearance {
        Appearance::System => 0,
        Appearance::Light => 1,
        Appearance::Dark => 2,
    }
}

/// 选文件夹
pub fn pick_folder(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        dialog.SetTitle(&crate::util::h(title)).ok()?;
        dialog.Show(Some(owner)).ok()?;
        result_path(&dialog)
    }
}

/// 选一个程序（.exe）
pub fn pick_app(owner: HWND) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filters = [COMDLG_FILTERSPEC { pszName: w!("程序"), pszSpec: w!("*.exe") }];
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetTitle(w!("选一个用来打开 md 文件的程序")).ok()?;
        dialog.Show(Some(owner)).ok()?;
        result_path(&dialog)
    }
}

unsafe fn result_path(dialog: &IFileOpenDialog) -> Option<PathBuf> {
    unsafe {
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = crate::util::from_pwstr(name);
        CoTaskMemFree(Some(name.0 as *const _));
        if path.is_empty() { None } else { Some(PathBuf::from(path)) }
    }
}

struct Layout<'a> {
    p: &'a Painter<'a>,
    pal: Palette,
    y: f32,
    x: f32,
    w: f32,
}

impl Layout<'_> {
    fn section(&mut self, title: &str) {
        self.y += 18.0;
        self.p.text(title, Rect::new(self.x + 4.0, self.y, self.w, 20.0), Font::ui(13.0, 600), self.pal.secondary, TextOpts::LINE);
        self.y += 26.0;
    }

    /// 一张卡片：`rows` 是每行的高度，画完背景和分隔线，返回每行的矩形
    fn card(&mut self, rows: &[f32]) -> Vec<Rect> {
        let h: f32 = rows.iter().sum();
        let card = Rect::new(self.x, self.y, self.w, h);
        self.p.fill_round(card, 10.0, self.pal.card);
        self.p.stroke_round(card.inset(0.5, 0.5), 10.0, self.pal.border, 1.0);
        let mut out = Vec::new();
        let mut y = self.y;
        for (i, rh) in rows.iter().enumerate() {
            if i > 0 {
                self.p.fill_rect(Rect::new(self.x + 16.0, y, self.w - 32.0, 1.0), self.pal.separator);
            }
            out.push(Rect::new(self.x, y, self.w, *rh));
            y += rh;
        }
        self.y = y;
        out
    }

    fn note(&mut self, text: &str) {
        let h = text::measure_wrapped(text, Font::ui(12.0, 400), self.w - 8.0);
        self.p.text(text, Rect::new(self.x + 4.0, self.y + 6.0, self.w - 8.0, h + 2.0), Font::ui(12.0, 400), self.pal.tertiary, TextOpts::WRAP);
        self.y += h + 8.0;
    }
}

/// 行的左边：标题和说明
fn row_label(p: &Painter, pal: &Palette, row: Rect, title: &str, detail: Option<&str>, right_reserved: f32) {
    let w = row.w - 32.0 - right_reserved;
    match detail {
        Some(detail) => {
            p.text(title, Rect::new(row.x + 16.0, row.y + 10.0, w, 20.0), Font::ui(14.0, 500), pal.text, TextOpts::LINE);
            p.text(detail, Rect::new(row.x + 16.0, row.y + 31.0, w, row.h - 36.0), Font::ui(12.0, 400), pal.secondary, TextOpts::WRAP);
        }
        None => p.text(title, Rect::new(row.x + 16.0, row.y, w, row.h), Font::ui(14.0, 500), pal.text, TextOpts::LINE),
    }
}

fn detail_height(text: &str, width: f32) -> f32 {
    (text::measure_wrapped(text, Font::ui(12.0, 400), width) + 44.0).max(52.0)
}

impl SettingsWindow {
    fn paint(&self) {
        let hwnd = self.hwnd.get();
        let (w, h) = window::client_size(hwnd);
        if w <= 0 || h <= 0 {
            return;
        }
        let dpi = window::dpi(hwnd) as f32;
        let dark = theme::is_dark();
        let pal = theme::palette();
        let a = app();
        let settings = a.settings.borrow().clone();
        let history_count = a.history.borrow().items.len();
        let failed = a.failed_hotkeys.borrow().clone();
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let mut surface = std::mem::replace(&mut st.surface, Surface::new_direct(hwnd));
        surface.render(w as u32, h as u32, dpi, dark, |p| {
            let s = dpi / 96.0;
            let size = Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s);
            p.fill_rect(size, if dark { Color::hex(0x1C1D20) } else { Color::hex(0xF3F4F6) });
            st.ia.begin_frame();
            let scroll = st.scroll.value;
            let content_w = (size.w - 48.0).min(560.0);
            let mut l = Layout { p, pal, y: 8.0 - scroll, x: (size.w - content_w) / 2.0, w: content_w };
            let ia = &mut st.ia;

            // 保存
            l.section("保存");
            let convert_detail = "在网页上选中复制时，保留标题、列表、代码块、表格和公式。AI 复制按钮给的本来就是 Markdown，不受影响";
            let rows = l.card(&[60.0, detail_height(convert_detail, content_w - 110.0)]);
            let folder = crate::util::friendly_path(&settings.folder());
            row_label(p, &pal, rows[0], "保存位置", None, 190.0);
            p.text(&folder, Rect::new(rows[0].x + 100.0, rows[0].y, rows[0].w - 300.0, rows[0].h), Font::ui(13.0, 400), pal.secondary, TextOpts::LINE);
            ui::pill_button(p, ia, &pal, ui::id(K_CHANGE_FOLDER, 0), Rect::new(rows[0].right() - 186.0, rows[0].mid_y() - 15.0, 92.0, 30.0), "更改…", false);
            ui::pill_button(p, ia, &pal, ui::id(K_OPEN_FOLDER, 0), Rect::new(rows[0].right() - 88.0, rows[0].mid_y() - 15.0, 72.0, 30.0), "打开", false);
            row_label(p, &pal, rows[1], "网页内容转成 Markdown", Some(convert_detail), 70.0);
            let v = toggle_spring(&mut st.toggles, K_CONVERT, settings.convert_html);
            ui::toggle(p, ia, &pal, ui::id(K_CONVERT, 0), Rect::new(rows[1].right() - 66.0, rows[1].mid_y() - 15.0, 50.0, 30.0), v);

            // 快捷键
            l.section("快捷键");
            let rows = l.card(&[54.0, 54.0, 54.0]);
            for (i, action) in Action::ALL.iter().enumerate() {
                let row = rows[i];
                let shortcut = match action {
                    Action::Save => settings.save_shortcut,
                    Action::Screenshot => settings.screenshot_shortcut,
                    Action::Ocr => settings.ocr_shortcut,
                };
                let recording = st.recording == Some(*action);
                let failed_here = failed.contains(action);
                let title = if failed_here { format!("{}（被别的程序占用了）", action.title()) } else { action.title().to_string() };
                p.text(&title, Rect::new(row.x + 16.0, row.y, row.w - 300.0, row.h), Font::ui(14.0, 500), if failed_here { theme::RED } else { pal.text }, TextOpts::LINE);
                let label = if recording { "按下新的组合键…".to_string() } else { shortcut.label() };
                let pill = Rect::new(row.right() - 16.0 - 170.0, row.mid_y() - 15.0, 170.0, 30.0);
                let id = ui::id(K_RECORD, i as u32);
                ia.add(id, pill);
                let hover = ia.hover(id);
                p.fill_round(pill, 8.0, if recording { pal.accent.alpha(0.16) } else { pal.fill.mix(pal.fill_strong, hover) });
                p.stroke_round(pill.inset(0.5, 0.5), 8.0, if recording { pal.accent } else { pal.border }, if recording { 1.5 } else { 1.0 });
                p.text(&label, pill, Font::ui(13.0, 600), if recording { pal.accent_text } else { pal.text }, TextOpts::CENTER);
                if shortcut != action.default_shortcut() && !recording {
                    let reset = Rect::new(pill.x - 86.0, row.mid_y() - 14.0, 80.0, 28.0);
                    ui::text_button(p, ia, &pal, ui::id(K_RESET, i as u32), reset, "恢复默认", Font::ui(12.5, 500), pal.accent_text);
                }
                if let Some((a, message)) = &st.message {
                    if a == action {
                        p.text(message, Rect::new(row.x + 16.0, row.bottom() - 18.0, row.w - 220.0, 16.0), Font::ui(11.5, 400), theme::RED, TextOpts::LINE);
                    }
                }
            }
            l.note("点一下再按新的组合键，至少要按住 Ctrl、Alt、Win 中的一个；按 Esc 取消。");

            // 预览
            l.section("预览");
            let preview_detail = match &settings.preview_app {
                Some(path) => format!("眼睛按钮会用 {} 打开文件", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
                None => "内置预览可以切换预览、编辑、并排，停止输入后自动保存，公式原生渲染".to_string(),
            };
            let rows = l.card(&[detail_height(&preview_detail, content_w - 290.0)]);
            row_label(p, &pal, rows[0], "点眼睛按钮时", Some(&preview_detail), 260.0);
            ui::segmented(p, ia, &pal, K_PREVIEW, Rect::new(rows[0].right() - 16.0 - 240.0, rows[0].mid_y() - 15.0, 240.0, 30.0), &["内置预览", "其他程序…"], st.preview_slide.value, Font::ui(13.0, 500));

            // 复制
            l.section("复制");
            let history_detail = "复制过的文字和图片先记在悬浮窗的「剪贴板」里，不存成文件，需要时再存成 md。最多 50 条，保存在本机；密码管理器复制的内容不记";
            let rows = l.card(&[54.0, detail_height(history_detail, content_w - 110.0), 54.0]);
            row_label(p, &pal, rows[0], "监听复制", None, 70.0);
            let v = toggle_spring(&mut st.toggles, K_LISTEN, settings.double_copy);
            ui::toggle(p, ia, &pal, ui::id(K_LISTEN, 0), Rect::new(rows[0].right() - 66.0, rows[0].mid_y() - 15.0, 50.0, 30.0), v);
            row_label(p, &pal, rows[1], "记录剪贴板", Some(history_detail), 70.0);
            let v = toggle_spring(&mut st.toggles, K_HISTORY, settings.history);
            ui::toggle(p, ia, &pal, ui::id(K_HISTORY, 0), Rect::new(rows[1].right() - 66.0, rows[1].mid_y() - 15.0, 50.0, 30.0), v);
            row_label(p, &pal, rows[2], &format!("已记录 {history_count} 条"), None, 120.0);
            ui::pill_button(p, ia, &pal, ui::id(K_CLEAR, 0), Rect::new(rows[2].right() - 16.0 - 100.0, rows[2].mid_y() - 15.0, 100.0, 30.0), "清空记录", false);
            l.note("同一段文字 3 秒内复制两次会自动保存；每次复制文字后，悬浮窗里的保存按钮会提示。关掉「监听复制」后完全不再查看剪贴板，剪贴板记录也会停止。");

            // 外观
            l.section("外观");
            let rows = l.card(&[56.0]);
            row_label(p, &pal, rows[0], "外观", None, 300.0);
            ui::segmented(p, ia, &pal, K_APPEARANCE, Rect::new(rows[0].right() - 16.0 - 270.0, rows[0].mid_y() - 15.0, 270.0, 30.0), &["跟随系统", "浅色", "深色"], st.appearance_slide.value, Font::ui(13.0, 500));

            // 悬浮窗
            l.section("悬浮窗");
            let rows = l.card(&[54.0, 54.0]);
            row_label(p, &pal, rows[0], "固定位置（不能拖动）", None, 70.0);
            let v = toggle_spring(&mut st.toggles, K_PINNED, settings.panel_pinned);
            ui::toggle(p, ia, &pal, ui::id(K_PINNED, 0), Rect::new(rows[0].right() - 66.0, rows[0].mid_y() - 15.0, 50.0, 30.0), v);
            row_label(p, &pal, rows[1], "开机自动启动", None, 70.0);
            let v = toggle_spring(&mut st.toggles, K_LOGIN, settings.launch_at_login);
            ui::toggle(p, ia, &pal, ui::id(K_LOGIN, 0), Rect::new(rows[1].right() - 66.0, rows[1].mid_y() - 15.0, 50.0, 30.0), v);

            // 关于
            l.section("关于");
            let rows = l.card(&[64.0]);
            crate::icons::app_glyph(p, Rect::new(rows[0].x + 16.0, rows[0].mid_y() - 16.0, 32.0, 32.0), dark);
            p.text(&format!("ClipMD {}（Windows 版）", env!("CARGO_PKG_VERSION")), Rect::new(rows[0].x + 60.0, rows[0].y + 12.0, 260.0, 20.0), Font::ui(14.0, 600), pal.text, TextOpts::LINE);
            p.text("MIT 许可 · 所有内容都在本机处理", Rect::new(rows[0].x + 60.0, rows[0].y + 33.0, 260.0, 18.0), Font::ui(12.0, 400), pal.secondary, TextOpts::LINE);
            ui::pill_button(p, ia, &pal, ui::id(K_DATA, 0), Rect::new(rows[0].right() - 16.0 - 96.0 - 104.0, rows[0].mid_y() - 15.0, 98.0, 30.0), "数据文件夹", false);
            ui::pill_button(p, ia, &pal, ui::id(K_GITHUB, 0), Rect::new(rows[0].right() - 16.0 - 96.0, rows[0].mid_y() - 15.0, 96.0, 30.0), "GitHub", false);
            l.y += 24.0;
            st.content_h = l.y + scroll;
        });
        st.surface = surface;
    }

    fn frame(&self) {
        let moving = {
            let mut st = self.st.borrow_mut();
            let dt = st.timer.tick();
            let mut moving = st.ia.step(dt) | st.appearance_slide.step(dt) | st.preview_slide.step(dt) | st.scroll.step(dt);
            for s in st.toggles.values_mut() {
                moving |= s.step(dt);
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

    fn animate(&self) {
        anim::request_frame(self.hwnd.get());
    }

    fn click(&self, id: ui::Id) {
        let a = app();
        let hwnd = self.hwnd.get();
        let index = ui::index_of(id) as usize;
        match ui::kind_of(id) {
            K_CHANGE_FOLDER => {
                if let Some(folder) = pick_folder(hwnd, "选择保存位置") {
                    a.set_folder(folder);
                }
            }
            K_OPEN_FOLDER => a.open_folder(),
            K_CONVERT => {
                let on = !a.settings.borrow().convert_html;
                a.settings.borrow_mut().convert_html = on;
                a.save_settings();
                a.refresh_panel();
            }
            K_RECORD => {
                let action = Action::ALL[index.min(2)];
                let mut st = self.st.borrow_mut();
                st.recording = if st.recording == Some(action) { None } else { Some(action) };
                st.message = None;
                let recording = st.recording.is_some();
                drop(st);
                if recording {
                    // 录制时先放开全局快捷键，免得按下去被它抢走
                    hotkeys::unregister_all(a.hwnd.get());
                    unsafe {
                        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd));
                    }
                } else {
                    a.register_hotkeys();
                }
            }
            K_RESET => {
                let action = Action::ALL[index.min(2)];
                self.set_shortcut(action, action.default_shortcut());
            }
            K_PREVIEW => {
                if index == 0 {
                    a.settings.borrow_mut().preview_app = None;
                    a.save_settings();
                    self.st.borrow_mut().preview_slide.set(0.0);
                } else if let Some(path) = pick_app(hwnd) {
                    a.settings.borrow_mut().preview_app = Some(path);
                    a.save_settings();
                    self.st.borrow_mut().preview_slide.set(1.0);
                }
            }
            K_LISTEN => {
                let on = !a.settings.borrow().double_copy;
                a.set_listening(on);
            }
            K_HISTORY => {
                let on = !a.settings.borrow().history;
                a.settings.borrow_mut().history = on;
                a.save_settings();
                a.refresh_panel();
            }
            K_CLEAR => a.clear_clips(),
            K_APPEARANCE => {
                let choice = [Appearance::System, Appearance::Light, Appearance::Dark][index.min(2)];
                a.settings.borrow_mut().appearance = choice;
                a.save_settings();
                theme::set_appearance(choice);
                self.st.borrow_mut().appearance_slide.set(index as f32);
                a.appearance_changed();
            }
            K_PINNED => {
                let on = !a.settings.borrow().panel_pinned;
                a.settings.borrow_mut().panel_pinned = on;
                a.save_settings();
                a.refresh_panel();
            }
            K_LOGIN => {
                let on = !a.settings.borrow().launch_at_login;
                a.settings.borrow_mut().launch_at_login = on;
                a.save_settings();
                crate::settings::set_launch_at_login(on);
            }
            K_GITHUB => crate::notes::open_path(std::path::Path::new("https://github.com/NuistLinf/ClipMD"), None),
            K_DATA => crate::notes::open_path(&crate::util::data_dir(), None),
            _ => {}
        }
        window::invalidate(hwnd);
        self.animate();
    }

    fn set_shortcut(&self, action: Action, shortcut: Shortcut) {
        let a = app();
        {
            let s = a.settings.borrow();
            let others = [(Action::Save, s.save_shortcut), (Action::Screenshot, s.screenshot_shortcut), (Action::Ocr, s.ocr_shortcut)];
            if others.iter().any(|(other, sc)| *other != action && *sc == shortcut) {
                drop(s);
                self.st.borrow_mut().message = Some((action, "和另一个快捷键重复了".into()));
                return;
            }
        }
        {
            let mut s = a.settings.borrow_mut();
            match action {
                Action::Save => s.save_shortcut = shortcut,
                Action::Screenshot => s.screenshot_shortcut = shortcut,
                Action::Ocr => s.ocr_shortcut = shortcut,
            }
        }
        a.save_settings();
        a.register_hotkeys();
        if a.failed_hotkeys.borrow().contains(&action) {
            self.st.borrow_mut().message = Some((action, "这个组合键被别的程序占用了，换一个试试".into()));
        } else {
            self.st.borrow_mut().message = None;
        }
        a.refresh_panel();
    }

    /// 录制快捷键时按下的键
    fn record_key(&self, vk: u16) -> bool {
        let Some(action) = self.st.borrow().recording else { return false };
        if vk == 0x1B {
            self.st.borrow_mut().recording = None;
            app().register_hotkeys();
        } else if !hotkeys::is_modifier(vk) {
            let shortcut = Shortcut::from_key(vk);
            if shortcut.is_valid() {
                self.st.borrow_mut().recording = None;
                self.set_shortcut(action, shortcut);
            } else {
                self.st.borrow_mut().message = Some((action, "至少要按住 Ctrl、Alt、Win 中的一个".into()));
            }
        }
        window::invalidate(self.hwnd.get());
        true
    }
}

/// 开关的动画值
fn toggle_spring(toggles: &mut HashMap<u32, Spring>, kind: u32, on: bool) -> f32 {
    let spring = toggles.entry(kind).or_insert_with(|| Spring::with(if on { 1.0 } else { 0.0 }, 0.25, 0.85));
    spring.set(if on { 1.0 } else { 0.0 });
    spring.value
}

impl Handler for SettingsWindow {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => {
                self.paint();
                window::validate(hwnd);
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_SIZE => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                let s = window::scale(hwnd);
                info.ptMinTrackSize = POINT { x: (520.0 * s) as i32, y: (360.0 * s) as i32 };
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
                if self.st.borrow_mut().ia.mouse_move(x, y) {
                    ui::hide_tooltip();
                    self.animate();
                }
                Some(LRESULT(0))
            }
            windows::Win32::UI::Controls::WM_MOUSELEAVE => {
                self.st.borrow_mut().ia.mouse_leave();
                self.animate();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = window::mouse_dip(hwnd, lp);
                self.st.borrow_mut().ia.mouse_down(x, y);
                unsafe {
                    SetCapture(hwnd);
                }
                self.animate();
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
                }
                self.animate();
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = crate::util::wheel_delta(wp) as f32;
                let (_, h) = window::client_size(hwnd);
                let view_h = h as f32 / window::scale(hwnd);
                let mut st = self.st.borrow_mut();
                let max = (st.content_h - view_h).max(0.0);
                let target = (st.scroll.target - delta / 120.0 * 60.0).clamp(0.0, max);
                st.scroll.set(target);
                drop(st);
                self.animate();
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if self.record_key(wp.0 as u16) {
                    return Some(LRESULT(0));
                }
                if wp.0 == 0x1B {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return Some(LRESULT(0));
                }
                None
            }
            WM_SYSKEYUP | WM_SYSCHAR if self.st.borrow().recording.is_some() => Some(LRESULT(0)),
            WM_KILLFOCUS => {
                if self.st.borrow_mut().recording.take().is_some() {
                    app().register_hotkeys();
                    window::invalidate(hwnd);
                }
                None
            }
            anim::WM_FRAME => {
                self.frame();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_TIP => Some(LRESULT(0)),
            WM_DESTROY => {
                if self.st.borrow_mut().recording.take().is_some() {
                    app().register_hotkeys();
                }
                self.st.borrow_mut().surface.release();
                WINDOW.with(|w| *w.borrow_mut() = None);
                None
            }
            _ => None,
        }
    }
}
