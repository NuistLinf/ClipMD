//! 剪贴板记录：复制过的文字和图片，最多 50 条，存在 `%LOCALAPPDATA%\ClipMD\` 里。

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use clipmd_core::clip_history::{self, ClipItem};

use crate::images::Image;

pub struct ClipHistory {
    pub items: Vec<ClipItem>,
    /// 删除、清空后可以撤销：记下删之前的列表
    undo: Option<Vec<ClipItem>>,
}

fn index_path() -> PathBuf {
    crate::util::data_dir().join("history.json")
}

pub fn images_dir() -> PathBuf {
    crate::util::data_dir().join("history-images")
}

/// 写文件放到后台线程。记下写到第几版，晚到的旧版本不覆盖新版本
static WRITTEN: Mutex<u64> = Mutex::new(0);
static VERSION: AtomicU64 = AtomicU64::new(0);

impl ClipHistory {
    pub fn load() -> ClipHistory {
        let items = std::fs::read(index_path()).map(|data| clip_history::decode(&data)).unwrap_or_default();
        let history = ClipHistory { items, undo: None };
        history.clean_images();
        history
    }

    pub fn image_path(&self, item: &ClipItem) -> Option<PathBuf> {
        item.image_file.as_ref().map(|f| images_dir().join(f))
    }

    pub fn find(&self, id: &str) -> Option<&ClipItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// 记一段文字。返回是否真的记下了（空白、太长的不记）
    pub fn add_text(&mut self, text: &str, html: Option<&str>, source: Option<&str>) -> bool {
        let Some(item) = clip_history::make_item(text, html, source, clipmd_core::time::now_unix()) else { return false };
        self.items = clip_history::adding(item, &self.items);
        self.undo = None;
        self.persist();
        true
    }

    /// 记一张图：PNG 存进 history-images 文件夹
    pub fn add_image(&mut self, image: &Image, png: &[u8], source: Option<&str>) -> Option<String> {
        let hash = image.fingerprint();
        if let Some(existing) = self.items.iter().find(|i| i.image_hash.as_deref() == Some(hash.as_str())) {
            // 同一张图再复制一次：挪到最前面
            let item = existing.clone();
            let id = item.id.clone();
            let mut moved = item;
            moved.date = clipmd_core::time::now_unix();
            self.items = clip_history::adding(moved, &self.items);
            self.persist();
            return Some(id);
        }
        let _ = std::fs::create_dir_all(images_dir());
        let file = format!("{}.png", clip_history::new_id());
        std::fs::write(images_dir().join(&file), png).ok()?;
        let item = ClipItem::new_image(file, image.width, image.height, hash, clipmd_core::time::now_unix(), source.map(str::to_string));
        let id = item.id.clone();
        self.items = clip_history::adding(item, &self.items);
        self.undo = None;
        self.persist();
        self.clean_images();
        Some(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<ClipItem> {
        let index = self.items.iter().position(|i| i.id == id)?;
        self.undo = Some(self.items.clone());
        let removed = self.items.remove(index);
        self.persist();
        Some(removed)
    }

    pub fn clear(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.undo = Some(std::mem::take(&mut self.items));
        self.persist();
    }

    pub fn undo(&mut self) {
        if let Some(items) = self.undo.take() {
            self.items = items;
            self.persist();
        }
    }

    /// 撤销的机会过了，删掉不再用的图片文件
    pub fn forget_undo(&mut self) {
        self.undo = None;
        self.clean_images();
    }

    pub fn mark_saved(&mut self, id: &str, file_name: &str) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.saved_file_name = Some(file_name.to_string());
            self.persist();
        }
    }

    /// 挪到最前面（放回剪贴板时）
    pub fn touch(&mut self, id: &str) {
        if let Some(index) = self.items.iter().position(|i| i.id == id) {
            let mut item = self.items.remove(index);
            item.date = clipmd_core::time::now_unix();
            self.items.insert(0, item);
            self.persist();
        }
    }

    fn persist(&self) {
        let Ok(data) = clip_history::encode(&self.items) else { return };
        let version = VERSION.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            let Ok(mut written) = WRITTEN.lock() else { return };
            if *written > version {
                return;
            }
            *written = version;
            let path = index_path();
            let temp = path.with_extension("json.tmp");
            if std::fs::write(&temp, data).is_ok() {
                let _ = std::fs::rename(&temp, &path);
            }
        });
    }

    /// 删掉记录里已经不用的图片文件（撤销期间先留着）
    fn clean_images(&self) {
        let mut keep = clip_history::referenced_images(&self.items);
        if let Some(undo) = &self.undo {
            keep.extend(clip_history::referenced_images(undo));
        }
        std::thread::spawn(move || {
            let Ok(entries) = std::fs::read_dir(images_dir()) else { return };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !keep.contains(&name) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        });
    }
}
