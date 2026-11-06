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
        crate::clipboard::write_text(a.hwnd.get(), "核心思路\n\n第一项\n第二项\n\nfn main() {}", Some(html));
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
fn more_steps() {
    // 公式排版
    step(100, || {
        let ok = crate::math_render::available();
        check("Cambria Math 公式字体", ok, "系统里没有 Cambria Math");
        for (latex, display) in [("\\frac{a}{b}", false), ("L = \\frac{1}{n}\\sum_{i=1}^{n} (y_i - \\hat y_i)^2", true), ("\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}", true), ("x = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}", true)] {
            let m = crate::math_render::layout(latex, 18.0, display);
            check(&format!("公式 {latex}"), m.as_ref().is_some_and(|m| m.width > 10.0 && m.height() > 10.0), "排版结果为空");
        }
        // 公式样张
        let samples = [
            "x = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}",
            "\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}",
            "\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}",
            "\\left( \\begin{matrix} a & b \\\\ c & d \\end{matrix} \\right) \\vec{v} = \\lambda \\vec{v}",
            "f(x) = \\begin{cases} x^2 & x \\ge 0 \\\\ -x & x < 0 \\end{cases}",
            "\\mathbb{R}^n,\\ \\mathcal{L}(\\theta) = -\\log p_\\theta(x),\\ \\alpha \\le \\beta \\to \\infty",
        ];
        let pal = crate::theme::palette_for(false);
        let pixels = crate::gfx::render_offscreen(1400, 1100, 192.0, false, |p| {
            p.fill_rect(crate::gfx::Rect::new(0.0, 0.0, 700.0, 550.0), crate::gfx::Color::WHITE);
            let mut y = 30.0;
            for latex in samples {
                if let Some(m) = crate::math_render::layout(latex, 20.0, true) {
                    m.draw(p, 30.0, y + m.ascent, pal.text);
                    y += m.height() + 30.0;
                }
            }
        });
        if let Some(pixels) = pixels {
            save_image("math-samples.png", &Image { width: 1400, height: 1100, pixels });
        }
    });
    // 预览窗口
    step(200, || {
        let a = app();
        let note = a.notes.borrow().notes.iter().find(|n| n.title.contains("训练")).cloned();
        match note {
            Some(n) => crate::preview::open(&n.path),
            None => check("找到示例笔记", false, "没有「训练结果总结」"),
        }
    });
    step(1200, || {
        check("预览窗口打开", crate::preview::count() == 1, &format!("{} 个", crate::preview::count()));
        if let Some((hwnd, _)) = crate::preview::latest() {
            window_shot("screen-preview.png", hwnd);
        }
    });
    step(100, || {
        if let Some((_, set_mode)) = crate::preview::latest() {
            set_mode(crate::preview::Mode::Split);
        }
    });
    step(900, || {
        if let Some((hwnd, _)) = crate::preview::latest() {
            window_shot("screen-preview-split.png", hwnd);
        }
        crate::preview::close_all();
    });
    // 设置窗口
    step(300, crate::settings_window::open);
    step(1000, || {
        match crate::settings_window::hwnd() {
            Some(hwnd) => {
                window_shot("screen-settings.png", hwnd);
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            None => check("设置窗口打开", false, "没有窗口"),
        }
    });
    // 剪贴板记录预览：文字、图片
    step(300, || {
        let id = app().history.borrow().items.iter().find(|i| !i.is_image()).map(|i| i.id.clone());
        if let Some(id) = id {
            crate::clip_preview::toggle(&id);
        }
    });
    step(900, || {
        match crate::clip_preview::hwnd() {
            Some(hwnd) => window_shot("screen-clip-preview-text.png", hwnd),
            None => check("剪贴板预览打开", false, "没有窗口"),
        }
        crate::clip_preview::close();
        let id = app().history.borrow().items.iter().find(|i| i.is_image()).map(|i| i.id.clone());
        if let Some(id) = id {
            crate::clip_preview::toggle(&id);
        }
    });
    step(900, || {
        if let Some(hwnd) = crate::clip_preview::hwnd() {
            window_shot("screen-clip-preview-image.png", hwnd);
        }
        crate::clip_preview::close();
    });
    // 深色模式的真实悬浮窗
    step(200, || {
        crate::theme::set_appearance(crate::theme::Appearance::Dark);
        app().appearance_changed();
        app().panel().unwrap().select_tab(crate::panel::Tab::Saved);
    });
    step(900, || {
        window_shot("screen-panel-dark.png", app().panel().unwrap().hwnd());
        crate::theme::set_appearance(crate::theme::Appearance::System);
        app().appearance_changed();
    });
    // 截图：真实的遮罩窗口
    step(400, || {
        let session = crate::capture::begin(false, false);
        check("截图界面打开", session.is_some(), "begin 返回空");
    });
    step(900, || {
        if let Some(image) = screen_image() {
            save_image("screen-capture-choosing.png", &image);
        }
        let Some(session) = crate::capture::current() else { return };
        let size = session.screen_size(0);
        let sel = crate::gfx::Rect::new(size.w * 0.2, size.h * 0.2, size.w * 0.45, size.h * 0.42);
        session.test_select(0, sel);
        use crate::capture::{Mark, render};
        session.test_mark(Mark::Rectangle { rect: crate::gfx::Rect::new(sel.x + 20.0, sel.y + 20.0, 160.0, 90.0), color: render::COLORS[0], width: 4.0 });
        session.test_mark(Mark::Ellipse { rect: crate::gfx::Rect::new(sel.x + 220.0, sel.y + 40.0, 140.0, 80.0), color: render::COLORS[4], width: 4.0 });
        session.test_mark(Mark::Mosaic { points: (0..20).map(|i| (sel.x + 40.0 + i as f32 * 8.0, sel.y + sel.h - 60.0)).collect(), width: 22.0 });
        session.test_tool(Some(crate::capture::Tool::Rectangle));
    });
    step(700, || {
        if let Some(image) = screen_image() {
            save_image("screen-capture-editing.png", &image);
        }
        let Some(session) = crate::capture::current() else { return };
        if let Some(image) = session.render_image(0) {
            save_image("capture-overlay-render.png", &image);
        }
        let export = session.export();
        check("截图导出", export.as_ref().is_some_and(|i| i.width > 100 && i.height > 100), "导出失败");
        if let Some(image) = &export {
            save_image("capture-export.png", image);
        }
        session.finish(crate::capture::Outcome::Copy);
    });
    step(1200, || {
        check("截图关闭后释放", !crate::capture::is_active(), "还在");
        check("截图放进剪贴板", crate::clipboard::hint() == crate::clipboard::Hint::Image, &format!("{:?}", crate::clipboard::hint()));
        let first = app().history.borrow().items.first().cloned();
        check("截图记进剪贴板记录", first.as_ref().is_some_and(|i| i.is_image() && i.source_app.as_deref() == Some("截图")), "第一条不是截图");
    });
    // 钉图
    step(200, || {
        let image = crate::images::decode_file(&out_dir().join("capture-export.png"), None).ok();
        match image {
            Some(image) => crate::pin::open(image, 120, 120, 1.0),
            None => check("钉图", false, "读不到导出的截图"),
        }
    });
    step(800, || {
        check("钉在桌面", crate::pin::count() == 1, &format!("{} 个", crate::pin::count()));
        if let Some(hwnd) = crate::pin::hwnds().first() {
            window_shot("screen-pin.png", *hwnd);
        }
        crate::pin::zoom_latest(0.6);
    });
    step(600, || {
        if let Some(hwnd) = crate::pin::hwnds().first() {
            window_shot("screen-pin-zoomed.png", *hwnd);
        }
        crate::pin::close_all();
    });
    // 识别文字（子进程）
    step(200, || {
        let pixels = crate::gfx::render_offscreen(900, 200, 96.0, false, |p| {
            p.fill_rect(crate::gfx::Rect::new(0.0, 0.0, 900.0, 200.0), crate::gfx::Color::WHITE);
            p.text("ClipMD OCR Test 2026", crate::gfx::Rect::new(30.0, 20.0, 840.0, 70.0), crate::text::Font::ui(44.0, 600), crate::gfx::Color::BLACK, crate::text::TextOpts::LINE);
            p.text("识别文字测试", crate::gfx::Rect::new(30.0, 110.0, 840.0, 70.0), crate::text::Font::ui(44.0, 600), crate::gfx::Color::BLACK, crate::text::TextOpts::LINE);
        });
        let Some(pixels) = pixels else { return };
        let image = Image { width: 900, height: 200, pixels };
        let path = out_dir().join("ocr-input.png");
        save_image("ocr-input.png", &image);
        let started = crate::util::now_seconds();
        match crate::ocr::recognize_in_helper(&path) {
            Ok(text) => {
                note(&format!("识别结果（{:.0} ms）：{:?}", (crate::util::now_seconds() - started) * 1000.0, text));
                check("识别英文", text.to_lowercase().contains("clipmd"), &text);
                if text.contains("识别") {
                    check("识别中文", true, "");
                } else {
                    note("没有识别出中文（这台机器可能没装中文识别包）");
                }
            }
            Err(e) => note(&format!("识别文字不可用：{e}")),
        }
    });
    // 托盘图标、标志
    step(100, || {
        for (name, color) in [("tray-white.png", crate::gfx::Color::WHITE), ("tray-black.png", crate::gfx::Color::hex(0x1A1A1A))] {
            let pixels = crate::gfx::render_offscreen(64, 64, 96.0, false, |p| {
                p.fill_rect(crate::gfx::Rect::new(0.0, 0.0, 64.0, 64.0), if color.r > 0.5 { crate::gfx::Color::hex(0x202020) } else { crate::gfx::Color::hex(0xF3F3F3) });
                crate::icons::tray_glyph(p, crate::gfx::Rect::new(16.0, 16.0, 32.0, 32.0), color, 32);
            });
            if let Some(pixels) = pixels {
                save_image(name, &Image { width: 64, height: 64, pixels });
            }
        }
        let pixels = crate::gfx::render_offscreen(240, 120, 192.0, false, |p| {
            crate::icons::app_glyph(p, crate::gfx::Rect::new(14.0, 14.0, 32.0, 32.0), false);
            crate::icons::app_glyph(p, crate::gfx::Rect::new(74.0, 14.0, 32.0, 32.0), true);
        });
        if let Some(pixels) = pixels {
            save_image("glyphs.png", &Image { width: 240, height: 120, pixels });
        }
        crate::hud::show(crate::hud::Toast::success("已保存为 Markdown", "核心思路 · 文件已复制，Ctrl+V 发送"));
    });
    step(700, || {
        if let Some(image) = screen_image() {
            save_image("screen-hud.png", &image);
        }
    });
    // 内存
    step(300, || {
        note(&format!("内存（工作集）：{:.1} MB", crate::util::working_set_mb()));
    });
}
