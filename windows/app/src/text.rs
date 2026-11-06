//! 文字：DirectWrite 的字体格式缓存、排版、测量。
//!
//! 界面字体用 Windows 11 的 Segoe UI Variable（没有就用 Segoe UI），中文由系统按 zh-CN 回退到微软雅黑；
//! 图标用 Segoe Fluent Icons（Windows 10 上是 Segoe MDL2 Assets，码位基本一样）。

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{BOOL, HSTRING, Interface, w};

use crate::gfx::{Color, Painter, Rect, v2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Ui,
    Mono,
    Icon,
    Math,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub size: f32,
    pub weight: u16,
    pub family: Family,
    pub italic: bool,
}

impl Font {
    pub const fn ui(size: f32, weight: u16) -> Font {
        Font { size, weight, family: Family::Ui, italic: false }
    }

    pub const fn mono(size: f32) -> Font {
        Font { size, weight: 400, family: Family::Mono, italic: false }
    }

    pub const fn icon(size: f32) -> Font {
        Font { size, weight: 400, family: Family::Icon, italic: false }
    }

    pub const fn bold(self) -> Font {
        Font { weight: 700, ..self }
    }

    pub const fn italic(self) -> Font {
        Font { italic: true, ..self }
    }

    fn key(&self) -> (u32, u16, Family, bool) {
        ((self.size * 100.0) as u32, self.weight, self.family, self.italic)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Leading,
    Center,
    Trailing,
}

/// 一段文字怎么排
#[derive(Clone, Copy, Debug)]
pub struct TextOpts {
    pub align: Align,
    /// 在给定高度里垂直居中
    pub vcenter: bool,
    /// 自动换行；否则单行
    pub wrap: bool,
    /// 放不下时末尾显示省略号
    pub ellipsis: bool,
}

impl TextOpts {
    pub const LINE: TextOpts = TextOpts { align: Align::Leading, vcenter: true, wrap: false, ellipsis: true };
    pub const CENTER: TextOpts = TextOpts { align: Align::Center, vcenter: true, wrap: false, ellipsis: true };
    pub const TRAILING: TextOpts = TextOpts { align: Align::Trailing, vcenter: true, wrap: false, ellipsis: true };
    pub const WRAP: TextOpts = TextOpts { align: Align::Leading, vcenter: false, wrap: true, ellipsis: false };
    pub const WRAP_CENTER: TextOpts = TextOpts { align: Align::Center, vcenter: false, wrap: true, ellipsis: false };
}

struct Cached {
    format: IDWriteTextFormat,
    ellipsis: Option<IDWriteInlineObject>,
}

struct TextSystem {
    factory: IDWriteFactory,
    formats: HashMap<(u32, u16, Family, bool), Cached>,
    ui_family: HSTRING,
    mono_family: HSTRING,
    icon_family: HSTRING,
}

thread_local! {
    static TEXT: RefCell<Option<TextSystem>> = const { RefCell::new(None) };
}

fn with_text<R>(f: impl FnOnce(&mut TextSystem) -> R) -> R {
    TEXT.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(TextSystem::new());
        }
        f(slot.as_mut().unwrap())
    })
}

/// 等宽字体的名字（编辑框用）
pub fn mono_family() -> String {
    with_text(|t| t.mono_family.to_string())
}

pub fn dwrite() -> IDWriteFactory {
    with_text(|t| t.factory.clone())
}

impl TextSystem {
    fn new() -> TextSystem {
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).expect("DirectWrite 不可用") };
        let has = |name: &str| -> bool {
            unsafe {
                let mut collection = None;
                if factory.GetSystemFontCollection(&mut collection, false).is_err() {
                    return false;
                }
                let Some(collection) = collection else { return false };
                let mut index = 0u32;
                let mut exists = BOOL(0);
                collection.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists).is_ok() && exists.as_bool()
            }
        };
        let pick = |names: &[&str]| -> HSTRING {
            HSTRING::from(*names.iter().find(|n| has(n)).unwrap_or(&names[names.len() - 1]))
        };
        TextSystem {
            ui_family: pick(&["Segoe UI Variable Text", "Segoe UI"]),
            mono_family: pick(&["Cascadia Mono", "Consolas"]),
            icon_family: pick(&["Segoe Fluent Icons", "Segoe MDL2 Assets"]),
            factory,
            formats: HashMap::new(),
        }
    }

    fn format(&mut self, font: Font) -> &Cached {
        let key = font.key();
        if !self.formats.contains_key(&key) {
            let family = match font.family {
                Family::Ui => self.ui_family.clone(),
                Family::Mono => self.mono_family.clone(),
                Family::Icon => self.icon_family.clone(),
                Family::Math => HSTRING::from("Cambria Math"),
            };
            let format = unsafe {
                self.factory
                    .CreateTextFormat(
                        &family,
                        None,
                        DWRITE_FONT_WEIGHT(font.weight as i32),
                        if font.italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL },
                        DWRITE_FONT_STRETCH_NORMAL,
                        font.size,
                        w!("zh-CN"),
                    )
                    .expect("创建字体失败")
            };
            let ellipsis = unsafe { self.factory.CreateEllipsisTrimmingSign(&format).ok() };
            self.formats.insert(key, Cached { format, ellipsis });
        }
        &self.formats[&key]
    }
}

/// 排好版的一段文字，可以测量也可以画
pub struct Layout {
    pub layout: IDWriteTextLayout,
    pub width: f32,
    pub height: f32,
    /// 第一行的基线到顶部的距离
    pub baseline: f32,
    pub lines: u32,
}

/// 排版。`max_width`、`max_height` 是可用空间（DIP）
pub fn layout(text: &str, font: Font, max_width: f32, max_height: f32, opts: TextOpts) -> Option<Layout> {
    with_text(|system| {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let factory = system.factory.clone();
        let cached = system.format(font);
        unsafe {
            let layout = factory.CreateTextLayout(&wide, &cached.format, max_width.max(1.0), max_height.max(1.0)).ok()?;
            let _ = layout.SetWordWrapping(if opts.wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
            let _ = layout.SetTextAlignment(match opts.align {
                Align::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = layout.SetParagraphAlignment(if opts.vcenter { DWRITE_PARAGRAPH_ALIGNMENT_CENTER } else { DWRITE_PARAGRAPH_ALIGNMENT_NEAR });
            if opts.ellipsis {
                let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = layout.SetTrimming(&trimming, cached.ellipsis.as_ref());
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut metrics);
            let mut line = [DWRITE_LINE_METRICS::default(); 1];
            let mut count = 0;
            let _ = layout.GetLineMetrics(Some(&mut line), &mut count);
            Some(Layout {
                width: metrics.widthIncludingTrailingWhitespace,
                height: metrics.height,
                baseline: line[0].baseline,
                lines: metrics.lineCount,
                layout,
            })
        }
    })
}

/// 单行文字的宽度
pub fn measure(text: &str, font: Font) -> f32 {
    layout(text, font, 10_000.0, 1_000.0, TextOpts { ellipsis: false, ..TextOpts::LINE }).map(|l| l.width).unwrap_or(0.0)
}

/// 自动换行后的高度
pub fn measure_wrapped(text: &str, font: Font, width: f32) -> f32 {
    layout(text, font, width, 100_000.0, TextOpts::WRAP).map(|l| l.height).unwrap_or(0.0)
}

impl Painter<'_> {
    /// 在 `rect` 里画一段文字
    pub fn text(&self, text: &str, rect: Rect, font: Font, color: Color, opts: TextOpts) {
        if text.is_empty() || rect.w <= 0.0 {
            return;
        }
        if let Some(l) = layout(text, font, rect.w, rect.h, opts) {
            self.draw_layout(&l.layout, rect.x, rect.y, color);
        }
    }

    pub fn draw_layout(&self, layout: &IDWriteTextLayout, x: f32, y: f32, color: Color) {
        unsafe {
            self.ctx.DrawTextLayout(v2(x, y), layout, self.brush(color), D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
    }

    /// 图标字体里的一个字形，居中画在 `rect` 里
    pub fn icon(&self, glyph: char, rect: Rect, size: f32, color: Color) {
        let mut buf = [0u8; 4];
        self.text(glyph.encode_utf8(&mut buf), rect, Font::icon(size), color, TextOpts { ellipsis: false, ..TextOpts::CENTER });
    }
}

/// 给一段排好版的文字的某个范围设置格式（粗体、颜色由调用方用画刷效果处理）
pub fn set_range_font(layout: &IDWriteTextLayout, start: u32, len: u32, weight: Option<u16>, italic: bool, mono: bool, strike: bool, size: Option<f32>) {
    let range = DWRITE_TEXT_RANGE { startPosition: start, length: len };
    unsafe {
        if let Some(weight) = weight {
            let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT(weight as i32), range);
        }
        if italic {
            let _ = layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range);
        }
        if mono {
            let family = with_text(|t| t.mono_family.clone());
            let _ = layout.SetFontFamilyName(&family, range);
        }
        if strike {
            let _ = layout.SetStrikethrough(true, range);
        }
        if let Some(size) = size {
            let _ = layout.SetFontSize(size, range);
        }
    }
}

pub fn set_range_underline(layout: &IDWriteTextLayout, start: u32, len: u32) {
    unsafe {
        let _ = layout.SetUnderline(true, DWRITE_TEXT_RANGE { startPosition: start, length: len });
    }
}

/// 给范围设置单独的颜色（用一支纯色画刷当"绘制效果"）
pub fn set_range_color(painter: &Painter, layout: &IDWriteTextLayout, start: u32, len: u32, color: Color) {
    unsafe {
        if let Ok(brush) = painter.ctx.CreateSolidColorBrush(&color.d2d(), None) {
            if let Ok(unknown) = brush.cast::<windows::core::IUnknown>() {
                let _ = layout.SetDrawingEffect(&unknown, DWRITE_TEXT_RANGE { startPosition: start, length: len });
            }
        }
    }
}

/// 在某个点击位置是第几个字符（UTF-16 下标）
pub fn hit_test(layout: &IDWriteTextLayout, x: f32, y: f32) -> u32 {
    unsafe {
        let mut trailing = BOOL(0);
        let mut inside = BOOL(0);
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        if layout.HitTestPoint(x, y, &mut trailing, &mut inside, &mut metrics).is_ok() {
            metrics.textPosition + if trailing.as_bool() { metrics.length } else { 0 }
        } else {
            0
        }
    }
}
