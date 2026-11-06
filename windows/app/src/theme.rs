//! 配色：浅色、深色两套，和 macOS 版的观感对齐。
//!
//! Windows 11 上窗口背后是系统的亚克力材质，我们在上面再叠一层半透明的底色；
//! Windows 10 没有亚克力，用不透明的底色。

#![allow(dead_code)]

use std::cell::Cell;

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

use crate::gfx::Color;

/// 品牌蓝，和图标一样
pub const BLUE: Color = Color::hex(0x2F7BF5);
pub const BLUE_LIGHT: Color = Color::hex(0x5EA8FF);
pub const GREEN: Color = Color::hex(0x2FB861);
pub const RED: Color = Color::hex(0xE5484D);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn title(self) -> &'static str {
        match self {
            Appearance::System => "跟随系统",
            Appearance::Light => "浅色",
            Appearance::Dark => "深色",
        }
    }
}

thread_local! {
    static APPEARANCE: Cell<Appearance> = const { Cell::new(Appearance::System) };
    static SYSTEM_DARK: Cell<Option<bool>> = const { Cell::new(None) };
}

pub fn set_appearance(appearance: Appearance) {
    APPEARANCE.with(|a| a.set(appearance));
}

/// 系统主题变了（WM_SETTINGCHANGE "ImmersiveColorSet"）时清掉缓存
pub fn system_changed() {
    SYSTEM_DARK.with(|c| c.set(None));
}

fn read_dword(name: windows::core::PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(value)
}

/// 应用用深色吗（系统设置里的"应用模式"）
pub fn system_dark() -> bool {
    SYSTEM_DARK.with(|c| {
        if let Some(v) = c.get() {
            return v;
        }
        let dark = read_dword(w!("AppsUseLightTheme")).map(|v| v == 0).unwrap_or(false);
        c.set(Some(dark));
        dark
    })
}

/// 任务栏是深色吗（托盘图标用白色还是黑色）
pub fn taskbar_dark() -> bool {
    read_dword(w!("SystemUsesLightTheme")).map(|v| v == 0).unwrap_or(true)
}

pub fn is_dark() -> bool {
    match APPEARANCE.with(|a| a.get()) {
        Appearance::System => system_dark(),
        Appearance::Light => false,
        Appearance::Dark => true,
    }
}

/// 一套颜色
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// 叠在亚克力上的底色
    pub tint: Color,
    /// 没有亚克力时的底色
    pub solid: Color,
    pub text: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub separator: Color,
    /// 小块背景（列表图标底、标签栏的槽）
    pub fill: Color,
    pub fill_strong: Color,
    /// 鼠标移上去的高亮
    pub hover: Color,
    pub pressed: Color,
    /// 标签栏选中的那块
    pub tab_selected: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub card: Color,
    pub border: Color,
    pub code_bg: Color,
    pub shadow: Color,
}

pub fn palette() -> Palette {
    palette_for(is_dark())
}

pub fn palette_for(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            tint: Color::hex(0x1E1F22).alpha(0.62),
            solid: Color::hex(0x2A2B2E),
            text: Color::WHITE.alpha(0.93),
            secondary: Color::WHITE.alpha(0.58),
            tertiary: Color::WHITE.alpha(0.36),
            separator: Color::WHITE.alpha(0.08),
            fill: Color::WHITE.alpha(0.07),
            fill_strong: Color::WHITE.alpha(0.12),
            hover: Color::WHITE.alpha(0.07),
            pressed: Color::WHITE.alpha(0.12),
            tab_selected: Color::hex(0x101113).alpha(0.75),
            accent: BLUE_LIGHT,
            accent_text: Color::hex(0x7DB6FF),
            card: Color::hex(0x2C2D31),
            border: Color::WHITE.alpha(0.10),
            code_bg: Color::WHITE.alpha(0.06),
            shadow: Color::BLACK.alpha(0.45),
        }
    } else {
        Palette {
            dark,
            tint: Color::hex(0xFAFAFC).alpha(0.66),
            solid: Color::hex(0xF7F7F9),
            text: Color::BLACK.alpha(0.88),
            secondary: Color::BLACK.alpha(0.52),
            tertiary: Color::BLACK.alpha(0.32),
            separator: Color::BLACK.alpha(0.07),
            fill: Color::BLACK.alpha(0.045),
            fill_strong: Color::BLACK.alpha(0.08),
            hover: Color::BLACK.alpha(0.045),
            pressed: Color::BLACK.alpha(0.08),
            tab_selected: Color::WHITE,
            accent: BLUE,
            accent_text: Color::hex(0x1F63E6),
            card: Color::WHITE,
            border: Color::BLACK.alpha(0.09),
            code_bg: Color::BLACK.alpha(0.04),
            shadow: Color::BLACK.alpha(0.18),
        }
    }
}
