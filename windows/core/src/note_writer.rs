//! md 文件的读写。全部是同步调用，由调用方放到后台线程执行。

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::note_namer;
use crate::time::LocalDate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedNote {
    pub path: PathBuf,
    pub title: String,
    /// 文件的创建时间（Unix 秒）
    pub date: i64,
    pub character_count: usize,
}

impl SavedNote {
    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// 存成新文件，同名时加上 " 2"、" 3"
pub fn save(text: &str, folder: &Path, date: LocalDate) -> io::Result<SavedNote> {
    fs::create_dir_all(folder)?;
    let title = note_namer::title(text);
    let mut path = folder.join(note_namer::file_name(&title, date, None));
    let mut suffix = 2;
    while path.exists() {
        path = folder.join(note_namer::file_name(&title, date, Some(suffix)));
        suffix += 1;
    }

    let mut body = text.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    write_atomically(&path, body.as_bytes())?;
    Ok(SavedNote {
        path,
        title,
        date: crate::time::now_unix(),
        character_count: text.chars().count(),
    })
}

/// 先写到临时文件再改名，写一半断电也不会留下坏文件
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("md.clipmd-tmp");
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// 编辑后保存。直接改写原文件，保留创建时间，免得改过的文件在列表里跳到最前面
pub fn overwrite(text: &str, path: &Path) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).truncate(true).open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// 最近保存的几条，按创建时间倒序。只读取最终要显示的那几个文件的内容来数字数。
/// `hidden` 是从列表里移除、但文件还在的文件名。
pub fn recent_notes(folder: &Path, limit: usize, hidden: &HashSet<String>) -> Vec<SavedNote> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut dated: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || hidden.contains(&name) {
                return None;
            }
            if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let created = meta.created().or_else(|_| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            Some((path, created))
        })
        .collect();

    dated.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    dated
        .into_iter()
        .take(limit)
        .map(|(path, created)| {
            let count = fs::read_to_string(&path)
                .map(|text| text.trim_matches(['\n', '\r']).chars().count())
                .unwrap_or(0);
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            SavedNote {
                title: display_title(&stem),
                date: crate::time::unix_of(created),
                character_count: count,
                path,
            }
        })
        .collect()
}

/// 去掉文件名里的 `2026-10-24 ` 日期前缀
pub fn display_title(name: &str) -> String {
    let b = name.as_bytes();
    let has_prefix = b.len() > 11
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b' ';
    if has_prefix { name[11..].to_string() } else { name.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_folder(tag: &str) -> PathBuf {
        let id = crate::clip_history::new_id();
        std::env::temp_dir().join(format!("ClipMD{tag}-{id}"))
    }

    const DATE: LocalDate = LocalDate { year: 2026, month: 10, day: 24 };

    #[test]
    fn saves_unique_files_and_lists_newest_first() {
        let folder = temp_folder("Tests");
        let first = save("# 同名\n一", &folder, DATE).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = save("# 同名\n二二", &folder, DATE).unwrap();

        assert_ne!(first.path, second.path);
        assert!(second.file_name().ends_with("同名 2.md"));
        assert_eq!(fs::read_to_string(&first.path).unwrap(), "# 同名\n一\n");

        let recent = recent_notes(&folder, 5, &HashSet::new());
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|n| n.title.starts_with("同名")));
        assert_eq!(recent[0].title, "同名 2");
        assert_eq!(recent[0].character_count, "# 同名\n二二".chars().count());
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn missing_folder_lists_nothing() {
        assert!(recent_notes(&temp_folder("Missing"), 5, &HashSet::new()).is_empty());
    }

    #[test]
    fn hidden_notes_are_skipped_and_backfilled() {
        let folder = temp_folder("EditTests");
        let a = save("# 甲", &folder, DATE).unwrap();
        save("# 乙", &folder, DATE).unwrap();
        let hidden: HashSet<String> = [a.file_name()].into();
        let recent = recent_notes(&folder, 5, &hidden);
        assert_eq!(recent.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["乙"]);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn overwrite_keeps_creation_date() {
        let folder = temp_folder("Overwrite");
        let note = save("# 原文", &folder, DATE).unwrap();
        let before = fs::metadata(&note.path).unwrap().created().ok();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        overwrite("# 改过了\n", &note.path).unwrap();
        let after = fs::metadata(&note.path).unwrap().created().ok();
        assert!(before.is_some());
        assert_eq!(before, after);
        assert_eq!(fs::read_to_string(&note.path).unwrap(), "# 改过了\n");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn display_title_drops_date() {
        assert_eq!(display_title("2026-10-24 标题"), "标题");
        assert_eq!(display_title("随便起的名字"), "随便起的名字");
    }
}
