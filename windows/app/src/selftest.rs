//! 自检：`ClipMD.exe --selftest <文件夹>`。
//!
//! 用单独的数据文件夹和示例数据把程序完整跑一遍：画各个窗口（离屏图 + 真实屏幕截图）、
//! 模拟复制网页内容再保存、检查生成的 md 文件……结果写进 report.txt，图片存在同一个文件夹。
//! GitHub Actions 的 Windows 机器上每次提交都会跑，用来在没有 Windows 电脑时检查界面和功能。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::app;
use crate::images::{self, Image};
use crate::log;

type Step = Box<dyn FnOnce()>;

thread_local! {
    static STEPS: RefCell<VecDeque<(u32, Step)>> = const { RefCell::new(VecDeque::new()) };
    static REPORT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<PathBuf> = RefCell::new(PathBuf::new());
}

/// 启动前：准备一个干净的数据文件夹和设置
pub fn prepare(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let data = dir.join("data");
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::create_dir_all(&data);
    unsafe { std::env::set_var("CLIPMD_DATA_DIR", &data) };
    let notes = dir.join("notes");
    let _ = std::fs::remove_dir_all(&notes);
    let settings = crate::settings::Settings { save_folder: Some(notes), welcomed: true, ..Default::default() };
    settings.save();
}

pub fn out_dir() -> PathBuf {
    OUT.with(|o| o.borrow().clone())
}

pub fn check(name: &str, ok: bool, detail: &str) {
    let line = if ok { format!("PASS {name}") } else { format!("FAIL {name}: {detail}") };
    log!("selftest {line}");
    REPORT.with(|r| r.borrow_mut().push(line));
}

pub fn note(line: &str) {
    REPORT.with(|r| r.borrow_mut().push(format!("     {line}")));
}

/// 加一步，`delay` 毫秒后执行（相对上一步）
pub fn step(delay: u32, f: impl FnOnce() + 'static) {
    STEPS.with(|s| s.borrow_mut().push_back((delay, Box::new(f))));
}

unsafe extern "system" fn on_timer(_hwnd: HWND, _msg: u32, id: usize, _time: u32) {
    unsafe {
        let _ = KillTimer(None, id);
    }
    let next = STEPS.with(|s| s.borrow_mut().pop_front());
    if let Some((_, f)) = next {
        f();
        schedule_next();
    }
}

fn schedule_next() {
    let delay = STEPS.with(|s| s.borrow().front().map(|(d, _)| *d));
    if let Some(delay) = delay {
        unsafe {
            SetTimer(None, 0, delay.max(10), Some(on_timer));
        }
    }
}

pub fn save_image(name: &str, image: &Image) {
    let path = out_dir().join(name);
    match images::write_png(image, &path) {
        Ok(()) => note(&format!("图片 {name} {}×{}", image.width, image.height)),
        Err(e) => check(&format!("写图片 {name}"), false, &e.to_string()),
    }
}

/// 整个虚拟屏幕的截图（真实的样子，包括亚克力背景）
pub fn screen_image() -> Option<Image> {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        crate::capture::grab_screen_rect(x, y, w, h)
    }
}

/// 截屏幕上某个窗口所在的那一块（四周多留一点，看得到阴影）
pub fn window_shot(name: &str, hwnd: HWND) {
    let r = crate::window::window_rect(hwnd);
    let pad = 24;
    if let Some(image) = crate::capture::grab_screen_rect(r.left - pad, r.top - pad, r.right - r.left + pad * 2, r.bottom - r.top + pad * 2) {
        save_image(name, &image);
    }
}

fn seed() {
    let a = app();
    let folder = a.folder();
    let _ = std::fs::create_dir_all(&folder);
    let samples = [
        "# 一、核心思路：先把剪贴板的内容存成文件\n\n复制以后按快捷键，文件放回剪贴板。",
        "下周三下午两点在 3 号会议室开会，带上电脑。",
        "## 训练结果总结\n\n| 模型 | 准确率 |\n|---|---|\n| A | 92.1% |\n| B | 94.3% |\n\n损失函数：\n\n$$\nL = \\frac{1}{n}\\sum_{i=1}^{n} (y_i - \\hat y_i)^2\n$$\n",
        "## 用 pandas 读取 CSV 并清洗数据\n\n```python\nimport pandas as pd\ndf = pd.read_csv(\"data.csv\")\ndf = df.dropna()\n```\n\n- 先看 `df.head()`\n- 再处理**缺失值**\n",
    ];
    for text in samples {
        let _ = a.notes.borrow_mut().save(text);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    {
        let mut history = a.history.borrow_mut();
        history.add_text("一、核心思路：先把剪贴板的内容存成文件，再把文件放回剪贴板。", None, Some("Gemini"));
        history.add_text("下周三下午两点在 3 号会议室开会", None, Some("微信"));
        history.add_text("## 训练结果总结\n准确率提升到 94.3%", None, Some("Claude"));
        // 一张示例图片
        let (w, h) = (900u32, 649u32);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for yy in 0..h {
            for xx in 0..w {
                let i = ((yy * w + xx) * 4) as usize;
                let tile = ((xx / 90) + (yy / 90)) % 2 == 0;
                pixels[i] = if tile { 0xF5 } else { 0x40 };
                pixels[i + 1] = (yy * 255 / h) as u8;
                pixels[i + 2] = (xx * 255 / w) as u8;
                pixels[i + 3] = 255;
            }
        }
        let image = Image { width: w, height: h, pixels };
        if let Ok(png) = images::encode_png(&image) {
            history.add_image(&image, &png, Some("截图"));
        }
        history.add_text("用 pandas 读取 CSV 文件，然后清洗缺失值、转换类型，最后按日期分组统计。", None, Some("ChatGPT"));
    }
    a.notes.borrow_mut().refresh();
    a.refresh_panel();
}

pub fn start(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    OUT.with(|o| *o.borrow_mut() = dir);
    note(&format!("Windows build {}，亚克力背景：{}", crate::window::os_build(), crate::window::backdrop_supported()));

    step(300, seed);
    step(900, || {
        let a = app();
        let panel = a.panel().unwrap();
        check("示例笔记", a.notes.borrow().notes.len() == 4, &format!("{} 条", a.notes.borrow().notes.len()));
        check("示例剪贴板记录", a.history.borrow().items.len() == 5, &format!("{} 条", a.history.borrow().items.len()));
        for (dark, name) in [(false, "panel-light.png"), (true, "panel-dark.png")] {
            match panel.render_image(dark, 2.0) {
                Some(image) => save_image(name, &image),
                None => check(name, false, "离屏绘制失败"),
            }
        }
        window_shot("screen-panel.png", panel.hwnd());
    });
    step(200, || {
        let panel = app().panel().unwrap();
        panel.select_tab(crate::panel::Tab::Clipboard);
    });
    step(800, || {
        let panel = app().panel().unwrap();
        for (dark, name) in [(false, "panel-clipboard-light.png"), (true, "panel-clipboard-dark.png")] {
            if let Some(image) = panel.render_image(dark, 2.0) {
                save_image(name, &image);
            }
        }
        window_shot("screen-panel-clipboard.png", panel.hwnd());
    });
    // 模拟从网页复制：剪贴板里同时有纯文本和网页格式
    step(200, || {
        let a = app();
        let html = "<h2>核心思路</h2><ul><li>第一项</li><li><strong>第二项</strong></li></ul><pre><code class=\"language-rust\">fn main() {}</code></pre>";
        crate::clipboard::write_text(HWND::default(), "核心思路\n\n第一项\n第二项\n\nfn main() {}", Some(html));
        a.save_clipboard(false);
    });
    step(600, || {
        let a = app();
        let newest = a.notes.borrow().notes.first().cloned();
        let content = newest.as_ref().and_then(|n| std::fs::read_to_string(&n.path).ok()).unwrap_or_default();
        let expected = "## 核心思路\n\n- 第一项\n- **第二项**\n\n```rust\nfn main() {}\n```\n";
        check("网页内容保存成 Markdown", content == expected, &format!("{content:?}"));
        check("文件放回剪贴板", crate::clipboard::read_files().first() == newest.as_ref().map(|n| &n.path), "剪贴板里不是刚存的文件");
        if let Some(n) = &newest {
            check("文件名", n.file_name().ends_with(" 核心思路.md"), &n.file_name());
        }
        let panel = a.panel().unwrap();
        window_shot("screen-panel-saved.png", panel.hwnd());
        if let Some(image) = panel.render_image(false, 2.0) {
            save_image("panel-saved-light.png", &image);
        }
    });
    more_steps();
    step(300, finish);
    schedule_next();
}

fn finish() {
    let report = REPORT.with(|r| r.borrow().join("\n"));
    let failed = report.lines().filter(|l| l.starts_with("FAIL")).count();
    let _ = std::fs::write(out_dir().join("report.txt"), format!("{report}\n\n{} 项失败\n", failed));
    log!("selftest finished, {failed} failed");
    app().quit();
    if failed > 0 {
        std::process::exit(1);
    }
}

#[allow(dead_code)]
pub fn screen_dc_ok() -> bool {
    unsafe { !GetDC(None).is_invalid() }
}

/// 预览、设置、截图、识别文字的检查
fn more_steps() {}
