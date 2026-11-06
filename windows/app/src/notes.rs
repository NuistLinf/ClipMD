//! 已保存的 md 文件：列表、保存、移到回收站（可撤销）、在资源管理器里显示、监听文件夹变化。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use clipmd_core::note_writer::{self, SavedNote};
use windows::Win32::Foundation::{HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// 列表最多显示这么多条
pub const LIST_LIMIT: usize = 50;

pub struct NoteStore {
    pub notes: Vec<SavedNote>,
    folder: PathBuf,
    hidden: HashSet<String>,
    /// 删除后等几秒才真的移到回收站，这期间可以撤销
    pending_trash: Vec<PathBuf>,
    watch_generation: Arc<AtomicU32>,
}

impl NoteStore {
    pub fn new(folder: PathBuf, hidden: &[String]) -> NoteStore {
        let mut store = NoteStore {
            notes: Vec::new(),
            folder,
            hidden: hidden.iter().cloned().collect(),
            pending_trash: Vec::new(),
            watch_generation: Arc::new(AtomicU32::new(0)),
        };
        store.refresh();
        store
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn set_folder(&mut self, folder: PathBuf) {
        self.folder = folder;
        self.refresh();
    }

    pub fn hidden_list(&self) -> Vec<String> {
        let mut list: Vec<String> = self.hidden.iter().cloned().collect();
        list.sort();
        list
    }

    pub fn refresh(&mut self) {
        let mut hidden = self.hidden.clone();
        for path in &self.pending_trash {
            if let Some(name) = path.file_name() {
                hidden.insert(name.to_string_lossy().into_owned());
            }
        }
        self.notes = note_writer::recent_notes(&self.folder, LIST_LIMIT, &hidden);
    }

    pub fn save(&mut self, text: &str) -> std::io::Result<SavedNote> {
        let note = note_writer::save(text, &self.folder, crate::util::local_date())?;
        self.refresh();
        Ok(note)
    }

    /// 从列表里移除，文件保留
    pub fn hide(&mut self, note: &SavedNote) {
        self.hidden.insert(note.file_name());
        self.refresh();
    }

    pub fn unhide(&mut self, name: &str) {
        self.hidden.remove(name);
        self.refresh();
    }

    /// 先从列表里拿掉，几秒后（没撤销的话）再真的移到回收站
    pub fn trash_later(&mut self, note: &SavedNote) {
        self.pending_trash.push(note.path.clone());
        self.refresh();
    }

    pub fn undo_trash(&mut self, path: &Path) {
        self.pending_trash.retain(|p| p != path);
        self.refresh();
    }

    /// 撤销时间过了：真的移到回收站
    pub fn commit_trash(&mut self) {
        for path in std::mem::take(&mut self.pending_trash) {
            move_to_recycle_bin(&path);
        }
        self.refresh();
    }

    /// 监听保存文件夹：有文件增删改就给 `hwnd` 发 `msg`（自己用别的程序改了文件，列表也会跟着变）
    pub fn watch(&self, hwnd: HWND, msg: u32) {
        let generation = self.watch_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.watch_generation.clone();
        let folder = self.folder.clone();
        let target = hwnd.0 as isize;
        let _ = std::fs::create_dir_all(&folder);
        std::thread::Builder::new()
            .name("folder-watch".into())
            .spawn(move || unsafe {
                let wide = crate::util::h(&folder.to_string_lossy());
                let Ok(handle) = FindFirstChangeNotificationW(&wide, false, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE) else {
                    return;
                };
                while current.load(Ordering::SeqCst) == generation {
                    if WaitForSingleObject(handle, 1000) == WAIT_OBJECT_0 {
                        // 连续的变化合并成一次
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        let _ = PostMessageW(Some(HWND(target as *mut _)), msg, WPARAM(0), LPARAM(0));
                        if FindNextChangeNotification(handle).is_err() {
                            break;
                        }
                    }
                }
                let _ = FindCloseChangeNotification(handle);
            })
            .ok();
    }
}

/// 移到回收站（可以从回收站还原）
pub fn move_to_recycle_bin(path: &Path) -> bool {
    let from: Vec<u16> = path.to_string_lossy().encode_utf16().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    unsafe { SHFileOperationW(&mut op) == 0 && !op.fAnyOperationsAborted.as_bool() }
}

/// 在资源管理器里打开文件夹并选中文件
pub fn reveal(path: &Path) {
    unsafe {
        let wide = crate::util::h(&path.to_string_lossy());
        let pidl = ILCreateFromPathW(&wide);
        if !pidl.is_null() {
            let _ = SHOpenFolderAndSelectItems(pidl, None, 0);
            ILFree(Some(pidl));
        }
    }
}

/// 用默认程序（或指定的程序）打开
pub fn open_path(path: &Path, with: Option<&Path>) {
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        match with {
            Some(app) => {
                let app = crate::util::h(&app.to_string_lossy());
                let arg = crate::util::h(&format!("\"{}\"", path.to_string_lossy()));
                ShellExecuteW(None, windows::core::w!("open"), &app, &arg, None, SW_SHOWNORMAL);
            }
            None => {
                let file = crate::util::h(&path.to_string_lossy());
                ShellExecuteW(None, windows::core::w!("open"), &file, None, None, SW_SHOWNORMAL);
            }
        }
    }
}
