//! 按 Markdown 排版显示：标题、段落（粗体、斜体、行内代码、删除线、链接）、列表、任务列表、引用、
//! 代码块、表格、分隔线、公式（行内和独立成行）。预览窗口和剪贴板记录的预览共用。
//!
//! 每块排成一个 DirectWrite 的文字排版；行内公式用"内嵌对象"在文字里占好位置，画完文字再把公式画上去。

use clipmd_core::markdown::{self, Align as CellAlign, BlockKind, Inline, MarkdownDocument};
use clipmd_core::math_extractor::{INLINE_CLOSE, INLINE_OPEN};
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{Result, implement};

use crate::gfx::{Color, Painter, Rect};
use crate::icons::{self, Icon};
use crate::math_render::{self, MathLayout};
use crate::text::{self, Font, TextOpts};
use crate::theme::Palette;

#[implement(IDWriteInlineObject)]
struct Spacer {
    width: f32,
    height: f32,
    baseline: f32,
}

impl IDWriteInlineObject_Impl for Spacer_Impl {
    fn Draw(
        &self,
        _context: *const core::ffi::c_void,
        _renderer: windows::core::Ref<IDWriteTextRenderer>,
        _x: f32,
        _y: f32,
        _sideways: windows::core::BOOL,
        _rtl: windows::core::BOOL,
        _effect: windows::core::Ref<windows::core::IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn GetMetrics(&self) -> Result<DWRITE_INLINE_OBJECT_METRICS> {
        Ok(DWRITE_INLINE_OBJECT_METRICS { width: self.width, height: self.height, baseline: self.baseline, supportsSideways: false.into() })
    }

    fn GetOverhangMetrics(&self) -> Result<DWRITE_OVERHANG_METRICS> {
        Ok(DWRITE_OVERHANG_METRICS::default())
    }

    fn GetBreakConditions(&self, before: *mut DWRITE_BREAK_CONDITION, after: *mut DWRITE_BREAK_CONDITION) -> Result<()> {
        unsafe {
            *before = DWRITE_BREAK_CONDITION_NEUTRAL;
            *after = DWRITE_BREAK_CONDITION_NEUTRAL;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Code,
    Link,
}

struct Paragraph {
    layout: IDWriteTextLayout,
    ranges: Vec<(u32, u32, Role)>,
    math: Vec<(u32, MathLayout)>,
    links: Vec<(u32, u32, String)>,
}

enum Kind {
    Text { para: Paragraph, marker: Option<String>, task: Option<bool>, heading: Option<u8> },
    Code { layout: IDWriteTextLayout, language: Option<String> },
    Rule,
    Math { layout: Option<MathLayout>, latex: String },
    Table { cols: Vec<f32>, rows: Vec<(f32, f32, Vec<Paragraph>)>, align: Vec<CellAlign> },
}

struct Item {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    quote: usize,
    kind: Kind,
}

pub struct DocView {
    doc: MarkdownDocument,
    base: f32,
    width: f32,
    items: Vec<Item>,
    pub height: f32,
}

const PAD: f32 = 2.0;

impl DocView {
    pub fn new(source: &str, base: f32) -> DocView {
        DocView { doc: markdown::parse(source), base, width: 0.0, items: Vec::new(), height: 0.0 }
    }

    pub fn set_source(&mut self, source: &str) {
        self.doc = markdown::parse(source);
        self.width = 0.0;
    }

    /// 按可用宽度排版（宽度没变就不重排）
    pub fn layout(&mut self, width: f32) {
        if (width - self.width).abs() < 0.5 && !self.items.is_empty() {
            return;
        }
        self.width = width;
        self.items.clear();
        let base = self.base;
        let mut y = 0.0;
        let mut previous: Option<BlockKind> = None;
        let blocks = self.doc.blocks.clone();
        for (index, block) in blocks.iter().enumerate() {
            let quote_indent = block.quote_depth as f32 * 16.0;
            let list_indent = if block.list_depth > 0 { block.list_depth as f32 * 22.0 + 4.0 } else { 0.0 };
            let x = PAD + quote_indent + list_indent;
            let w = (width - x - PAD).max(40.0);
            // 块之间的间距
            let gap = match (&previous, &block.kind) {
                (None, _) => 0.0,
                (_, BlockKind::Heading(level)) => if *level <= 2 { base * 1.3 } else { base * 1.0 },
                (Some(BlockKind::Heading(_)), _) => base * 0.45,
                _ if block.list_depth > 0 && blocks.get(index.wrapping_sub(1)).is_some_and(|b| b.list_depth > 0) => base * 0.3,
                _ => base * 0.75,
            };
            y += gap;
            let item = match &block.kind {
                BlockKind::Paragraph | BlockKind::Heading(_) => {
                    let heading = if let BlockKind::Heading(level) = block.kind { Some(level) } else { None };
                    let font = match heading {
                        Some(1) => Font::ui(base * 1.7, 700),
                        Some(2) => Font::ui(base * 1.4, 700),
                        Some(3) => Font::ui(base * 1.2, 650),
                        Some(_) => Font::ui(base * 1.05, 650),
                        None => Font::ui(base, 400),
                    };
                    let para = self.paragraph(&block.text, font, w);
                    let h = para.layout_height();
                    Item { x, y, w, h, quote: block.quote_depth, kind: Kind::Text { para, marker: block.list_marker.clone(), task: block.task, heading } }
                }
                BlockKind::CodeBlock(language) => {
                    let code = block.code();
                    let l = text::layout(&code, Font::mono(base * 0.87), w - 24.0, 100_000.0, TextOpts::WRAP);
                    let layout = l.map(|l| l.layout);
                    let h = layout.as_ref().map(height_of).unwrap_or(base) + 24.0;
                    match layout {
                        Some(layout) => Item { x, y, w, h, quote: block.quote_depth, kind: Kind::Code { layout, language: language.clone() } },
                        None => continue,
                    }
                }
                BlockKind::ThematicBreak => Item { x, y, w, h: base * 0.8, quote: block.quote_depth, kind: Kind::Rule },
                BlockKind::Math(i) => {
                    let latex = self.doc.math.get(*i).map(|m| m.latex.clone()).unwrap_or_default();
                    let layout = math_render::layout(&latex, base * 1.1, true);
                    let h = layout.as_ref().map(|m| m.height()).unwrap_or(base * 1.4) + base * 0.8;
                    Item { x, y, w, h, quote: block.quote_depth, kind: Kind::Math { layout, latex } }
                }
                BlockKind::Table => {
                    let (cols, rows, h) = self.table(&block.table_rows, w);
                    Item { x, y, w, h, quote: block.quote_depth, kind: Kind::Table { cols, rows, align: block.table_align.clone() } }
                }
            };
            y += item.h;
            previous = Some(block.kind.clone());
            self.items.push(item);
        }
        self.height = y + PAD;
    }

    /// 一段带格式的文字
    fn paragraph(&self, inline: &Inline, font: Font, width: f32) -> Paragraph {
        let mut text = String::new();
        let mut spans: Vec<(u32, u32, &markdown::SpanStyle)> = Vec::new();
        let mut offset = 0u32;
        for span in inline {
            let len = span.text.encode_utf16().count() as u32;
            spans.push((offset, len, &span.style));
            text.push_str(&span.text);
            offset += len;
        }
        let layout = text::layout(&text, font, width, 100_000.0, TextOpts::WRAP).map(|l| l.layout).unwrap_or_else(|| {
            text::layout(" ", font, width, 100.0, TextOpts::WRAP).unwrap().layout
        });
        let mut ranges = Vec::new();
        let mut links = Vec::new();
        for (start, len, style) in spans {
            if len == 0 {
                continue;
            }
            let weight = if style.bold { Some(if font.weight >= 650 { 800 } else { 700 }) } else { None };
            text::set_range_font(&layout, start, len, weight, style.italic, style.code, style.strike, if style.code { Some(font.size * 0.9) } else { None });
            if style.code {
                ranges.push((start, len, Role::Code));
            }
            if let Some(url) = &style.link {
                text::set_range_underline(&layout, start, len);
                ranges.push((start, len, Role::Link));
                links.push((start, len, url.clone()));
            }
        }
        // 行内公式：占位符换成内嵌对象
        let mut math = Vec::new();
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut i = 0;
        while i < units.len() {
            if units[i] == INLINE_OPEN as u16 {
                if let Some(end) = units[i..].iter().position(|&u| u == INLINE_CLOSE as u16).map(|e| e + i) {
                    let index: Option<usize> = String::from_utf16_lossy(&units[i + 1..end]).parse().ok();
                    if let Some(snippet) = index.and_then(|n| self.doc.math.get(n)) {
                        let size = font.size * if snippet.display { 1.05 } else { 1.0 };
                        let m = math_render::layout(&snippet.latex, size, snippet.display).unwrap_or_else(|| fallback_math(&snippet.latex, size));
                        let spacer: IDWriteInlineObject = Spacer { width: m.width + 2.0, height: m.height(), baseline: m.ascent }.into();
                        unsafe {
                            let _ = layout.SetInlineObject(&spacer, DWRITE_TEXT_RANGE { startPosition: i as u32, length: (end - i + 1) as u32 });
                        }
                        math.push((i as u32, m));
                    }
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
        Paragraph { layout, ranges, math, links }
    }

    fn table(&self, rows: &[Vec<Inline>], width: f32) -> (Vec<f32>, Vec<(f32, f32, Vec<Paragraph>)>, f32) {
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return (vec![], vec![], 0.0);
        }
        let font = Font::ui(self.base * 0.93, 400);
        let pad = 10.0;
        let mut natural = vec![40.0f32; cols];
        for row in rows {
            for (c, cell) in row.iter().enumerate() {
                let w = text::measure(&markdown::plain_text(cell), font).min(320.0) + pad * 2.0;
                natural[c] = natural[c].max(w);
            }
        }
        let total: f32 = natural.iter().sum();
        let widths: Vec<f32> = if total > width { natural.iter().map(|w| (w / total * width).max(48.0)).collect() } else { natural };
        let mut y = 0.0;
        let mut out = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let f = if r == 0 { Font::ui(font.size, 650) } else { font };
            let paras: Vec<Paragraph> = (0..cols)
                .map(|c| {
                    let empty = Vec::new();
                    let cell = row.get(c).unwrap_or(&empty);
                    self.paragraph(cell, f, widths[c] - pad * 2.0)
                })
                .collect();
            let h = paras.iter().map(|p| p.layout_height()).fold(0.0, f32::max) + 14.0;
            out.push((y, h, paras));
            y += h;
        }
        (widths, out, y)
    }

    /// 画在 `origin`（左上角），只画 `visible` 范围里的块
    pub fn draw(&self, p: &Painter, pal: &Palette, origin: (f32, f32), visible: Rect) {
        let base = self.base;
        for item in &self.items {
            let top = origin.1 + item.y;
            if top + item.h < visible.y - 4.0 || top > visible.bottom() + 4.0 {
                continue;
            }
            let x = origin.0 + item.x;
            // 引用的竖线
            for q in 0..item.quote {
                let qx = origin.0 + PAD + q as f32 * 16.0 + 3.0;
                p.fill_round(Rect::new(qx, top - base * 0.2, 3.0, item.h + base * 0.4), 1.5, pal.accent.alpha(0.35));
            }
            let text_color = if item.quote > 0 { pal.secondary } else { pal.text };
            match &item.kind {
                Kind::Text { para, marker, task, heading } => {
                    if let Some(checked) = task {
                        let r = Rect::new(x - 22.0, top + 2.0, 16.0, 16.0);
                        if *checked {
                            p.fill_round(r, 4.0, pal.accent);
                            icons::draw(p, Icon::Check, r, 12.0, Color::WHITE);
                        } else {
                            p.stroke_round(r.inset(0.75, 0.75), 4.0, pal.secondary, 1.5);
                        }
                    } else if let Some(marker) = marker {
                        let font = Font::ui(base, if marker.ends_with('.') { 500 } else { 700 });
                        p.text(marker, Rect::new(x - 40.0, top, 34.0, base * 1.5), font, pal.secondary, TextOpts { vcenter: false, ellipsis: false, ..TextOpts::TRAILING });
                    }
                    para.draw(p, pal, x, top, text_color);
                    if *heading == Some(1) || *heading == Some(2) {
                        let ly = top + item.h + base * 0.25;
                        p.fill_rect(Rect::new(x, ly, item.w, 1.0), pal.separator);
                    }
                }
                Kind::Code { layout, language } => {
                    let bg = Rect::new(x, top, item.w, item.h);
                    p.fill_round(bg, 8.0, pal.code_bg);
                    p.stroke_round(bg.inset(0.5, 0.5), 8.0, pal.separator, 1.0);
                    if let Some(lang) = language {
                        p.text(lang, Rect::new(bg.right() - 90.0, top + 4.0, 82.0, 16.0), Font::ui(11.0, 500), pal.tertiary, TextOpts::TRAILING);
                    }
                    p.draw_layout(layout, x + 12.0, top + 12.0, text_color);
                }
                Kind::Rule => {
                    p.fill_rect(Rect::new(x, top + item.h / 2.0, item.w, 1.0), pal.separator.alpha(2.0));
                }
                Kind::Math { layout, latex } => match layout {
                    Some(m) => {
                        let mx = x + ((item.w - m.width) / 2.0).max(0.0);
                        p.clip(Rect::new(x - 2.0, top, item.w + 4.0, item.h), || m.draw(p, mx, top + base * 0.4 + m.ascent, text_color));
                    }
                    None => {
                        let m = fallback_math(latex, base);
                        m.draw(p, x + ((item.w - m.width) / 2.0).max(0.0), top + base * 0.4 + m.ascent, pal.secondary);
                    }
                },
                Kind::Table { cols, rows, align } => {
                    let total: f32 = cols.iter().sum();
                    let table = Rect::new(x, top, total, item.h);
                    p.clip_round(table, 8.0, || {
                        for (r, (ry, rh, paras)) in rows.iter().enumerate() {
                            let row_rect = Rect::new(x, top + ry, total, *rh);
                            if r == 0 {
                                p.fill_rect(row_rect, pal.fill_strong);
                            } else if r % 2 == 0 {
                                p.fill_rect(row_rect, pal.fill.alpha(0.6));
                            }
                            let mut cx = x;
                            for (c, para) in paras.iter().enumerate() {
                                let w = cols[c];
                                let pw = para.width();
                                let dx = match align.get(c) {
                                    Some(CellAlign::Center) => (w - 20.0 - pw) / 2.0,
                                    Some(CellAlign::Right) => w - 20.0 - pw,
                                    _ => 0.0,
                                };
                                para.draw(p, pal, cx + 10.0 + dx.max(0.0), top + ry + 7.0, text_color);
                                cx += w;
                            }
                            if r > 0 {
                                p.fill_rect(Rect::new(x, top + ry, total, 1.0), pal.separator);
                            }
                        }
                        let mut cx = x;
                        for w in &cols[..cols.len().saturating_sub(1)] {
                            cx += w;
                            p.fill_rect(Rect::new(cx, top, 1.0, item.h), pal.separator);
                        }
                    });
                    p.stroke_round(table.inset(0.5, 0.5), 8.0, pal.border, 1.0);
                }
            }
        }
    }

    /// 点到的链接
    pub fn link_at(&self, x: f32, y: f32) -> Option<String> {
        for item in &self.items {
            if y < item.y || y > item.y + item.h {
                continue;
            }
            if let Kind::Text { para, .. } = &item.kind {
                let pos = text::hit_test(&para.layout, x - item.x, y - item.y);
                return para.links.iter().find(|(s, l, _)| pos >= *s && pos < s + l).map(|(_, _, u)| u.clone());
            }
        }
        None
    }

}

fn height_of(layout: &IDWriteTextLayout) -> f32 {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = layout.GetMetrics(&mut m);
    }
    m.height
}

impl Paragraph {
    fn layout_height(&self) -> f32 {
        height_of(&self.layout)
    }

    fn width(&self) -> f32 {
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = self.layout.GetMetrics(&mut m);
        }
        m.width
    }

    fn draw(&self, p: &Painter, pal: &Palette, x: f32, y: f32, color: Color) {
        // 行内代码的底色
        for &(start, len, role) in &self.ranges {
            if role == Role::Code {
                for r in range_rects(&self.layout, start, len) {
                    p.fill_round(Rect::new(x + r.x - 2.0, y + r.y + 1.0, r.w + 4.0, r.h - 2.0), 4.0, pal.code_bg.alpha(1.8));
                }
            }
            if role == Role::Link {
                text::set_range_color(p, &self.layout, start, len, pal.accent_text);
            }
        }
        p.draw_layout(&self.layout, x, y, color);
        if self.math.is_empty() {
            return;
        }
        // 公式画在内嵌对象占的位置上，和那一行的基线对齐
        let lines = line_metrics(&self.layout);
        for (pos, m) in &self.math {
            let mut hx = 0.0;
            let mut hy = 0.0;
            let mut metrics = DWRITE_HIT_TEST_METRICS::default();
            unsafe {
                let _ = self.layout.HitTestTextPosition(*pos, false, &mut hx, &mut hy, &mut metrics);
            }
            let mut top = 0.0;
            let mut baseline = m.ascent;
            for line in &lines {
                if metrics.top < top + line.height - 0.5 {
                    baseline = top + line.baseline;
                    break;
                }
                top += line.height;
            }
            m.draw(p, x + metrics.left + 1.0, y + baseline, color);
        }
    }
}

fn line_metrics(layout: &IDWriteTextLayout) -> Vec<DWRITE_LINE_METRICS> {
    unsafe {
        let mut count = 0u32;
        let _ = layout.GetLineMetrics(None, &mut count);
        let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
        let _ = layout.GetLineMetrics(Some(&mut lines), &mut count);
        lines
    }
}

fn range_rects(layout: &IDWriteTextLayout, start: u32, len: u32) -> Vec<Rect> {
    unsafe {
        let mut count = 0u32;
        let _ = layout.HitTestTextRange(start, len, 0.0, 0.0, None, &mut count);
        let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
        let _ = layout.HitTestTextRange(start, len, 0.0, 0.0, Some(&mut metrics), &mut count);
        metrics.iter().map(|m| Rect::new(m.left, m.top, m.width, m.height)).collect()
    }
}

/// 没有公式字体时用等宽字体显示原文
fn fallback_math(latex: &str, size: f32) -> MathLayout {
    math_render::source_layout(latex, size)
}
