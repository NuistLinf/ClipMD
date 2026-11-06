//! 程序的中枢：启动、主消息窗口（托盘、快捷键、剪贴板变化都发到这里）、所有用户操作。
//!
//! 界面全部在主线程。耗时的活（编码 PNG、识别文字、写文件）放到后台线程，做完用 `run_on_main` 回到主线程。

use std::cell::{Cell, OnceCell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use clipmd_core::clipboard_markdown;
use clipmd_core::note_writer::SavedNote;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, COPYDATASTRUCT, RemoveClipboardFormatListener};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::clipboard::{self, Hint};
use crate::history::ClipHistory;
use crate::hotkeys::{self, Action};
use crate::hud::{self, Toast};
use crate::notes::{self, NoteStore};
use crate::panel::{Panel, Tab};
use crate::settings::Settings;
use crate::tray::Tray;
use crate::watcher::Watcher;
use crate::window::{self, Handler};
use crate::{log, theme};

pub const WM_TRAY: u32 = WM_APP + 2;
pub const WM_NOTES_CHANGED: u32 = WM_APP + 3;
pub const WM_RUN: u32 = WM_APP + 4;
pub const WM_CLIP_RETRY: u32 = WM_APP + 5;

const TIMER_UNDO: usize = 1;
const TIMER_CLIP_RETRY: usize = 2;

/// 可以撤销的操作（悬浮窗底部的"撤销"条）
#[derive(Clone, Debug)]
pub enum Undo {
    TrashNote { path: PathBuf, title: String },
    HideNote { name: String, title: String },
    RemoveClip { title: String },
    ClearClips { count: usize },
    /// 连点两次复制自动保存的
    AutoSave { path: PathBuf, text: String, html: Option<String> },
}

impl Undo {
    pub fn message(&self) -> String {
        match self {
            Undo::TrashNote { title, .. } => format!("已移到回收站：{title}"),
            Undo::HideNote { title, .. } => format!("已从列表移除：{title}"),
            Undo::RemoveClip { title } => format!("已删除：{title}"),
            Undo::ClearClips { count } => format!("已清空 {count} 条记录"),
            Undo::AutoSave { .. } => "连点两次复制，已自动保存".to_string(),
        }
    }
}

pub struct App {
    pub hwnd: Cell<HWND>,
    pub settings: RefCell<Settings>,
    pub notes: RefCell<NoteStore>,
    pub history: RefCell<ClipHistory>,
    pub panel: RefCell<Option<Rc<Panel>>>,
    tray: RefCell<Option<Tray>>,
    pub watcher: RefCell<Watcher>,
    pub undo: RefCell<Option<Undo>>,
    pub failed_hotkeys: RefCell<Vec<Action>>,
    listening: Cell<bool>,
    taskbar_created: u32,
}

thread_local! {
    static APP: OnceCell<Rc<App>> = const { OnceCell::new() };
}

pub fn app() -> Rc<App> {
    APP.with(|a| a.get().expect("App 还没创建").clone())
}

pub fn try_app() -> Option<Rc<App>> {
    APP.with(|a| a.get().cloned())
}

// MARK: 回到主线程

type Job = Box<dyn FnOnce() + Send>;
static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static MAIN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 后台线程做完活后，把结果交回主线程处理
pub fn run_on_main(job: impl FnOnce() + Send + 'static) {
    JOBS.lock().unwrap().push(Box::new(job));
    let hwnd = MAIN_HWND.load(std::sync::atomic::Ordering::SeqCst);
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_RUN, WPARAM(0), LPARAM(0));
    }
}

/// 开一个初始化了 COM 的后台线程（WIC、识别文字要用）
pub fn background(name: &str, job: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        job();
    });
}

// MARK: 启动

pub fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--ocr") {
        std::process::exit(crate::ocr::run_helper(args.get(2).map(String::as_str)));
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let selftest = args.iter().position(|a| a == "--selftest").map(|i| PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(|| ".".into())));
    if let Some(dir) = &selftest {
        crate::selftest::prepare(dir);
    }

    // 只开一个：已经在运行时，把悬浮窗叫出来就退出
    let mutex_name = if selftest.is_some() { w!("Local\\ClipMD.SelfTest") } else { w!("Local\\ClipMD.SingleInstance") };
    let _mutex = unsafe { CreateMutexW(None, true, mutex_name) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        send_to_running(if args.iter().any(|a| a == "--background") { "noop" } else { "show" });
        return;
    }

    unsafe {
        let _ = OleInitialize(None);
    }
    log!("ClipMD {} starting (Windows build {})", env!("CARGO_PKG_VERSION"), window::os_build());

    let settings = Settings::load();
    theme::set_appearance(settings.appearance);
    crate::menu::apply_theme(theme::is_dark());
    let notes = NoteStore::new(settings.folder(), &settings.hidden_notes);
    let history = ClipHistory::load();
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let app = Rc::new(App {
        hwnd: Cell::new(HWND::default()),
        settings: RefCell::new(settings),
        notes: RefCell::new(notes),
        history: RefCell::new(history),
        panel: RefCell::new(None),
        tray: RefCell::new(None),
        watcher: RefCell::new(Watcher::default()),
        undo: RefCell::new(None),
        failed_hotkeys: RefCell::new(Vec::new()),
        listening: Cell::new(false),
        taskbar_created,
    });
    APP.with(|a| {
        let _ = a.set(app.clone());
    });

    // 隐藏的主窗口：普通顶层窗口（不是只收消息的窗口），这样能收到 TaskbarCreated 广播、能当菜单的主人
    let main = Rc::new(MainWindow);
    let hwnd = window::create("ClipMD.Main", WS_POPUP, WS_EX_TOOLWINDOW, (0, 0, 0, 0), None, main).expect("创建主窗口失败");
    app.hwnd.set(hwnd);
    MAIN_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);

    *app.tray.borrow_mut() = Tray::new(hwnd, WM_TRAY);
    app.register_hotkeys();
    app.update_listening();
    app.notes.borrow().watch(hwnd, WM_NOTES_CHANGED);

    let panel = Panel::create();
    *app.panel.borrow_mut() = Some(panel.clone());
    let hidden = app.settings.borrow().panel_hidden;
    if !hidden || selftest.is_some() {
        panel.show();
    }
    app.refresh_hint();

    let failed = app.failed_hotkeys.borrow().clone();
    if !failed.is_empty() && selftest.is_none() {
        let names: Vec<&str> = failed.iter().map(|a| a.title()).collect();
        hud::show(Toast::warning("有快捷键被别的程序占用了", &format!("{}：可以在设置里换一个", names.join("、"))));
    }

    if let Some(dir) = selftest {
        crate::selftest::start(dir);
    }

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    app.shutdown();
}

/// 把命令发给已经在运行的 ClipMD
fn send_to_running(command: &str) {
    unsafe {
        let Ok(hwnd) = FindWindowW(window::CLASS, w!("ClipMD.Main")) else { return };
        let bytes = command.as_bytes();
        let data = COPYDATASTRUCT { dwData: 0x434D, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
        // 让对方可以把自己的窗口设成前台
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid);
        SendMessageW(hwnd, WM_COPYDATA, Some(WPARAM(0)), Some(LPARAM(&data as *const _ as isize)));
    }
}

struct MainWindow;

impl Handler for MainWindow {
    fn handle(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let app = try_app()?;
        match msg {
            WM_HOTKEY => {
                if let Some(action) = Action::from_id(wp.0) {
                    app.perform_hotkey(action);
                }
                Some(LRESULT(0))
            }
            WM_CLIPBOARDUPDATE => {
                Watcher::on_clipboard_update(&app, 0);
                Some(LRESULT(0))
            }
            WM_CLIP_RETRY => {
                Watcher::on_clipboard_update(&app, wp.0 as u32);
                Some(LRESULT(0))
            }
            WM_TRAY => {
                let event = crate::util::loword(lp.0 as usize) as u32;
                let x = (wp.0 & 0xFFFF) as i16 as i32;
                let y = ((wp.0 >> 16) & 0xFFFF) as i16 as i32;
                app.on_tray(event, POINT { x, y });
                Some(LRESULT(0))
            }
            WM_NOTES_CHANGED => {
                app.notes.borrow_mut().refresh();
                app.refresh_panel();
                Some(LRESULT(0))
            }
            WM_RUN => {
                let jobs: Vec<Job> = std::mem::take(&mut *JOBS.lock().unwrap());
                for job in jobs {
                    job();
                }
                Some(LRESULT(0))
            }
            WM_TIMER => {
                match wp.0 {
                    TIMER_UNDO => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_UNDO);
                        }
                        app.commit_undo();
                    }
                    TIMER_CLIP_RETRY => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_CLIP_RETRY);
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE => {
                let changed = unsafe { (lp.0 as *const u16).as_ref().map(|_| windows::core::PCWSTR(lp.0 as *const u16).to_string().unwrap_or_default()) };
                if changed.as_deref() == Some("ImmersiveColorSet") {
                    theme::system_changed();
                    app.appearance_changed();
                }
                None
            }
            WM_DISPLAYCHANGE => {
                if let Some(panel) = app.panel() {
                    panel.ensure_on_screen();
                }
                None
            }
            WM_COPYDATA => {
                let data = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
                let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
                let command = String::from_utf8_lossy(bytes).into_owned();
                app.handle_command(&command);
                Some(LRESULT(1))
            }
            WM_QUERYENDSESSION => Some(LRESULT(1)),
            WM_ENDSESSION => {
                if wp.0 != 0 {
                    app.shutdown();
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ if msg == app.taskbar_created && msg != 0 => {
                // 资源管理器重启了，托盘图标要重新加
                if let Some(tray) = app.tray.borrow_mut().as_mut() {
                    tray.readd();
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

// MARK: 操作

impl App {
    pub fn panel(&self) -> Option<Rc<Panel>> {
        self.panel.borrow().clone()
    }

    pub fn refresh_panel(&self) {
        if let Some(panel) = self.panel() {
            panel.refresh();
        }
    }

    fn shutdown(&self) {
        self.notes.borrow_mut().commit_trash();
        self.history.borrow_mut().forget_undo();
        self.save_settings();
        hotkeys::unregister_all(self.hwnd.get());
        if let Some(mut tray) = self.tray.borrow_mut().take() {
            tray.remove();
        }
    }

    pub fn quit(&self) {
        self.shutdown();
        unsafe {
            let _ = DestroyWindow(self.hwnd.get());
        }
    }

    pub fn save_settings(&self) {
        let mut settings = self.settings.borrow_mut();
        settings.hidden_notes = self.notes.borrow().hidden_list();
        settings.save();
    }

    pub fn register_hotkeys(&self) {
        let shortcuts = {
            let s = self.settings.borrow();
            vec![(Action::Save, s.save_shortcut), (Action::Screenshot, s.screenshot_shortcut), (Action::Ocr, s.ocr_shortcut)]
        };
        *self.failed_hotkeys.borrow_mut() = hotkeys::register_all(self.hwnd.get(), &shortcuts);
    }

    /// 关掉"监听复制"后完全不再查看剪贴板
    pub fn update_listening(&self) {
        let want = self.settings.borrow().double_copy;
        if want == self.listening.get() {
            return;
        }
        unsafe {
            if want {
                let _ = AddClipboardFormatListener(self.hwnd.get());
            } else {
                let _ = RemoveClipboardFormatListener(self.hwnd.get());
            }
        }
        self.listening.set(want);
        self.watcher.borrow_mut().reset();
    }

    fn perform_hotkey(&self, action: Action) {
        match action {
            Action::Save => self.save_clipboard(false),
            Action::Screenshot => crate::capture::start(false),
            Action::Ocr => crate::capture::start(true),
        }
    }

    pub fn handle_command(&self, command: &str) {
        log!("command: {command}");
        match command {
            "show" => {
                if let Some(panel) = self.panel() {
                    panel.show();
                    panel.attention();
                }
            }
            "save" => self.save_clipboard(false),
            "screenshot" => crate::capture::start(false),
            "ocr" => crate::capture::start(true),
            "settings" => crate::settings_window::open(),
            "quit" => self.quit(),
            _ => {}
        }
    }

    fn on_tray(&self, event: u32, at: POINT) {
        const NIN_SELECT: u32 = WM_USER;
        const NIN_KEYSELECT: u32 = WM_USER + 1;
        match event {
            // 版本 4 的托盘图标：单击会同时收到 WM_LBUTTONUP 和 NIN_SELECT，只认后者
            NIN_SELECT | NIN_KEYSELECT => self.toggle_panel(),
            WM_CONTEXTMENU => self.tray_menu(at),
            _ => {}
        }
    }

    fn tray_menu(&self, at: POINT) {
        use crate::menu::{self, Entry};
        let (visible, listening, save, shot, ocr) = {
            let s = self.settings.borrow();
            let visible = self.panel().is_some_and(|p| p.is_visible());
            (visible, s.double_copy, s.save_shortcut.label(), s.screenshot_shortcut.label(), s.ocr_shortcut.label())
        };
        let entries = vec![
            menu::item(1, if visible { "隐藏悬浮窗" } else { "显示悬浮窗" }),
            Entry::Separator,
            menu::item(2, &format!("保存剪贴板\t{save}")),
            menu::item(3, &format!("截图\t{shot}")),
            menu::item(4, &format!("截图并识别文字\t{ocr}")),
            Entry::Separator,
            menu::checked(5, "监听复制", listening),
            menu::item(6, "打开保存文件夹"),
            menu::item(7, "设置…"),
            Entry::Separator,
            menu::item(9, "退出 ClipMD"),
        ];
        match menu::popup(self.hwnd.get(), &entries, at) {
            Some(1) => self.toggle_panel(),
            Some(2) => self.save_clipboard(false),
            Some(3) => crate::capture::start(false),
            Some(4) => crate::capture::start(true),
            Some(5) => {
                let on = !self.settings.borrow().double_copy;
                self.set_listening(on);
            }
            Some(6) => self.open_folder(),
            Some(7) => crate::settings_window::open(),
            Some(9) => self.quit(),
            _ => {}
        }
    }

    pub fn set_listening(&self, on: bool) {
        self.settings.borrow_mut().double_copy = on;
        self.save_settings();
        self.update_listening();
        self.refresh_panel();
        crate::settings_window::refresh();
    }

    pub fn toggle_panel(&self) {
        let Some(panel) = self.panel() else { return };
        let show = !panel.is_visible();
        if show {
            panel.show();
        } else {
            panel.hide();
        }
        self.settings.borrow_mut().panel_hidden = !show;
        self.save_settings();
    }

    pub fn appearance_changed(&self) {
        crate::menu::apply_theme(theme::is_dark());
        self.refresh_panel();
        if let Some(panel) = self.panel() {
            panel.restyle();
        }
        if let Some(tray) = self.tray.borrow_mut().as_mut() {
            tray.update_icon();
        }
        crate::settings_window::restyle();
        crate::preview::restyle_all();
        crate::clip_preview::restyle();
    }

    /// 剪贴板里大概是什么，更新悬浮窗的提示
    pub fn refresh_hint(&self) {
        let hint = clipboard::hint();
        if let Some(panel) = self.panel() {
            panel.set_hint(hint, false);
        }
    }

    // MARK: 保存

    /// 把剪贴板存成 md，文件放回剪贴板。`auto` 表示是连点两次复制触发的
    pub fn save_clipboard(&self, auto: bool) {
        let hint = clipboard::hint();
        let text = match hint {
            Hint::Text { .. } => clipboard::read_text().filter(|t| t.chars().any(|c| !c.is_whitespace())),
            _ => None,
        };
        let Some(text) = text else {
            if let Some(panel) = self.panel() {
                panel.set_hint(hint, false);
                panel.shake();
            }
            if !auto {
                hud::show(Toast::warning("没有可保存的文字", hint.message(self.settings.borrow().convert_html)));
            }
            return;
        };
        let html = if self.settings.borrow().convert_html { clipboard::read_html() } else { None };
        self.save_text(&text, html.as_deref(), auto);
    }

    /// 存一段文字（剪贴板、拖进来的文字、剪贴板记录都走这里）
    pub fn save_text(&self, plain: &str, html: Option<&str>, auto: bool) -> Option<SavedNote> {
        let (markdown, source) = clipboard_markdown::best(plain, html);
        let result = self.notes.borrow_mut().save(&markdown);
        match result {
            Ok(note) => {
                log!("saved {} ({} chars, {:?})", note.file_name(), note.character_count, source);
                let copied = clipboard::write_file(self.hwnd.get(), &note.path);
                if let Some(panel) = self.panel() {
                    panel.did_save(copied);
                }
                if auto {
                    self.set_undo(Undo::AutoSave { path: note.path.clone(), text: plain.to_string(), html: html.map(str::to_string) });
                    hud::show(Toast::success("已自动保存为 Markdown", &format!("{} · 文件已复制，Ctrl+V 发送", note.title)));
                } else if !self.panel().is_some_and(|p| p.is_visible()) {
                    hud::show(Toast::success("已保存为 Markdown", &format!("{} · 文件已复制，Ctrl+V 发送", note.title)));
                }
                Some(note)
            }
            Err(error) => {
                log!("save failed: {error}");
                hud::show(Toast::warning("保存失败", &error.to_string()));
                None
            }
        }
    }

    pub fn copy_note(&self, note: &SavedNote) {
        if !note.path.exists() {
            self.notes.borrow_mut().refresh();
            self.refresh_panel();
            hud::show(Toast::warning("文件不见了", "可能已经被移动或删除"));
            return;
        }
        clipboard::write_file(self.hwnd.get(), &note.path);
        if let Some(panel) = self.panel() {
            panel.mark_copied(Tab::Saved, note.file_name());
        }
    }

    pub fn preview_note(&self, note: &SavedNote) {
        let app_path = self.settings.borrow().preview_app.clone();
        match app_path {
            Some(path) if path.exists() => notes::open_path(&note.path, Some(&path)),
            _ => crate::preview::open(&note.path),
        }
    }

    pub fn reveal_note(&self, note: &SavedNote) {
        notes::reveal(&note.path);
    }

    pub fn hide_note(&self, note: &SavedNote) {
        self.notes.borrow_mut().hide(note);
        self.save_settings();
        self.set_undo(Undo::HideNote { name: note.file_name(), title: note.title.clone() });
        self.refresh_panel();
    }

    pub fn trash_note(&self, note: &SavedNote) {
        crate::preview::close_for(&note.path);
        self.notes.borrow_mut().trash_later(note);
        self.set_undo(Undo::TrashNote { path: note.path.clone(), title: note.title.clone() });
        self.refresh_panel();
    }

    pub fn open_folder(&self) {
        let folder = self.notes.borrow().folder().to_path_buf();
        let _ = std::fs::create_dir_all(&folder);
        notes::open_path(&folder, None);
    }

    pub fn set_folder(&self, folder: PathBuf) {
        self.notes.borrow_mut().commit_trash();
        self.settings.borrow_mut().save_folder = Some(folder.clone());
        self.notes.borrow_mut().set_folder(folder);
        self.notes.borrow().watch(self.hwnd.get(), WM_NOTES_CHANGED);
        self.save_settings();
        self.refresh_panel();
    }

    // MARK: 剪贴板记录

    pub fn copy_clip(&self, id: &str) {
        let item = self.history.borrow().find(id).cloned();
        let Some(item) = item else { return };
        if item.is_image() {
            let path = self.history.borrow().image_path(&item);
            if let Some(path) = path {
                let png = std::fs::read(&path).ok();
                if let (Some(png), Ok(image)) = (png.as_ref(), crate::images::decode_file(&path, None)) {
                    clipboard::write_image(self.hwnd.get(), &image, Some(png));
                }
            }
        } else {
            clipboard::write_text(self.hwnd.get(), &item.text, item.html.as_deref());
        }
        self.history.borrow_mut().touch(id);
        if let Some(panel) = self.panel() {
            panel.mark_copied(Tab::Clipboard, item.id.clone());
        }
        self.refresh_panel();
    }

    pub fn save_clip(&self, id: &str) {
        let item = self.history.borrow().find(id).cloned();
        let Some(item) = item else { return };
        if item.is_image() {
            self.ocr_clip(id);
            return;
        }
        let html = if self.settings.borrow().convert_html { item.html.clone() } else { None };
        if let Some(note) = self.save_text(&item.text, html.as_deref(), false) {
            self.history.borrow_mut().mark_saved(id, &note.file_name());
            self.refresh_panel();
        }
    }

    pub fn ocr_clip(&self, id: &str) {
        let item = self.history.borrow().find(id).cloned();
        let Some(path) = item.as_ref().and_then(|i| self.history.borrow().image_path(i)) else { return };
        self.recognize_file(path, "剪贴板记录");
    }

    /// 识别一张图片里的文字，结果放进剪贴板
    pub fn recognize_file(&self, path: PathBuf, source: &'static str) {
        hud::show_after(0.4, Toast::working("正在识别文字…", "识别完会放进剪贴板"));
        background("ocr", move || {
            let result = crate::ocr::recognize_in_helper(&path);
            run_on_main(move || {
                let app = app();
                match result {
                    Ok(text) if !text.trim().is_empty() => {
                        clipboard::write_text(app.hwnd.get(), &text, None);
                        app.history.borrow_mut().add_text(&text, None, Some(source));
                        app.refresh_panel();
                        let lines = text.lines().count();
                        hud::show(Toast::success("已识别并复制文字", &format!("{lines} 行 · Ctrl+V 粘贴")));
                    }
                    Ok(_) => hud::show(Toast::warning("没有识别到文字", "换一块区域试试")),
                    Err(message) => hud::show(Toast::warning("识别失败", &message)),
                }
            });
        });
    }

    pub fn remove_clip(&self, id: &str) {
        let removed = self.history.borrow_mut().remove(id);
        if let Some(item) = removed {
            self.set_undo(Undo::RemoveClip { title: item.title() });
            self.refresh_panel();
        }
    }

    pub fn clear_clips(&self) {
        let count = self.history.borrow().items.len();
        if count == 0 {
            return;
        }
        self.history.borrow_mut().clear();
        self.set_undo(Undo::ClearClips { count });
        self.refresh_panel();
    }

    pub fn reveal_clip(&self, id: &str) {
        let item = self.history.borrow().find(id).cloned();
        if let Some(path) = item.and_then(|i| self.history.borrow().image_path(&i)) {
            notes::reveal(&path);
        }
    }

    /// 截图完成：放进剪贴板，同时记进剪贴板记录
    pub fn add_captured_image(&self, image: crate::images::Image, png: Vec<u8>) {
        clipboard::write_image(self.hwnd.get(), &image, Some(&png));
        if self.settings.borrow().history {
            self.history.borrow_mut().add_image(&image, &png, Some("截图"));
        }
        self.refresh_panel();
    }

    // MARK: 撤销

    pub fn set_undo(&self, undo: Undo) {
        // 上一个撤销机会作废，先把它落实
        self.commit_undo();
        if let Some(panel) = self.panel() {
            panel.show_undo(&undo.message());
        }
        *self.undo.borrow_mut() = Some(undo);
        unsafe {
            SetTimer(Some(self.hwnd.get()), TIMER_UNDO, 6000, None);
        }
    }

    pub fn perform_undo(&self) {
        let undo = self.undo.borrow_mut().take();
        unsafe {
            let _ = KillTimer(Some(self.hwnd.get()), TIMER_UNDO);
        }
        match undo {
            Some(Undo::TrashNote { path, .. }) => self.notes.borrow_mut().undo_trash(&path),
            Some(Undo::HideNote { name, .. }) => {
                self.notes.borrow_mut().unhide(&name);
                self.save_settings();
            }
            Some(Undo::RemoveClip { .. }) | Some(Undo::ClearClips { .. }) => self.history.borrow_mut().undo(),
            Some(Undo::AutoSave { path, text, html }) => {
                notes::move_to_recycle_bin(&path);
                self.notes.borrow_mut().refresh();
                clipboard::write_text(self.hwnd.get(), &text, html.as_deref());
                hud::show(Toast::success("已撤销", "文件已移到回收站，剪贴板恢复成原来的文字"));
            }
            None => {}
        }
        if let Some(panel) = self.panel() {
            panel.hide_undo();
        }
        self.refresh_panel();
    }

    pub fn commit_undo(&self) {
        let undo = self.undo.borrow_mut().take();
        match undo {
            Some(Undo::TrashNote { .. }) => self.notes.borrow_mut().commit_trash(),
            Some(Undo::RemoveClip { .. }) | Some(Undo::ClearClips { .. }) => self.history.borrow_mut().forget_undo(),
            _ => {}
        }
        if let Some(panel) = self.panel() {
            panel.hide_undo();
        }
    }

    pub fn note_by_name(&self, name: &str) -> Option<SavedNote> {
        self.notes.borrow().notes.iter().find(|n| n.file_name() == name).cloned()
    }

    pub fn folder(&self) -> PathBuf {
        self.notes.borrow().folder().to_path_buf()
    }

    pub fn is_note_path(&self, path: &Path) -> bool {
        path.parent() == Some(self.notes.borrow().folder())
    }
}
