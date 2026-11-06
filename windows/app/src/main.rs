//! ClipMD 的 Windows 版：托盘图标 + 悬浮窗，复制 AI 的回复一键存成 Markdown，顺手截图、标注、识别文字。
//!
//! 界面全部用 Win32 + Direct2D/DirectWrite 自己画（DirectComposition 交换链，Windows 11 上背后是系统的亚克力材质），
//! 不嵌浏览器、不用跨平台界面库，常驻内存和 macOS 版一样控制在几十 MB 以内。
//! 平台无关的逻辑（HTML 转 Markdown、命名、记录规则）在 `clipmd-core` 里，和 macOS 版共用同一套测试用例。

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// 消息常量没导入时 match 分支会变成"匹配一切"的变量，必须报错
#![deny(unreachable_patterns, non_snake_case)]

#[cfg(windows)]
mod anim;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod clip_preview;
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod dragdrop;
#[cfg(windows)]
mod editor;
#[cfg(windows)]
mod gfx;
#[cfg(windows)]
mod history;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod hud;
#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod images;
#[cfg(windows)]
mod markdown_view;
#[cfg(windows)]
mod math_render;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod notes;
#[cfg(windows)]
mod ocr;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod pin;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod selftest;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod settings_window;
#[cfg(windows)]
mod text;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod util;
#[cfg(windows)]
mod watcher;
#[cfg(windows)]
mod window;

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("这是 ClipMD 的 Windows 版，只能在 Windows 上运行。macOS 版在仓库根目录。");
}
