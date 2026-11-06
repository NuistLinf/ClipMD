//! 图标：全部用矢量路径自己画（16×16 的设计网格，线宽 1.5），不依赖图标字体，
//! Windows 10、11 上一模一样，任何缩放比例都清晰。风格对齐 macOS 版用的 SF Symbols。
//! 还有 ClipMD 的标志（取景框 + M↓）：悬浮窗左上角的彩色版、托盘里的单色版。

#![allow(dead_code)]

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::{D2D1_QUADRATIC_BEZIER_SEGMENT, ID2D1GeometrySink};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

use crate::gfx::{Color, Painter, Rect, render_offscreen, v2};
use crate::text::{Font, TextOpts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Screenshot,
    Folder,
    Tuck,
    Settings,
    SaveDoc,
    Eye,
    TextScan,
    Close,
    Check,
    CheckCircle,
    TextLines,
    Trash,
    Copy,
    Clipboard,
    Pin,
    Rectangle,
    Ellipse,
    Mosaic,
    Undo,
    Plus,
    ChevronDown,
    Pencil,
    Columns,
    Document,
    Search,
    Photo,
    Download,
    ArrowOut,
    Palette,
}

/// 在 16×16 网格里画线的小工具
struct Pen<'a> {
    p: &'a Painter<'a>,
    ox: f32,
    oy: f32,
    s: f32,
    color: Color,
    width: f32,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> windows_numerics::Vector2 {
        v2(self.ox + x * self.s, self.oy + y * self.s)
    }

    fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let (a, b) = (self.pt(x1, y1), self.pt(x2, y2));
        self.p.line(a.X, a.Y, b.X, b.Y, self.color, self.width);
    }

    fn poly(&self, pts: &[(f32, f32)], closed: bool) {
        let mapped: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (self.ox + x * self.s, self.oy + y * self.s)).collect();
        self.p.polyline(&mapped, self.color, self.width, closed);
    }

    fn fill_poly(&self, pts: &[(f32, f32)]) {
        let mapped: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (self.ox + x * self.s, self.oy + y * self.s)).collect();
        self.p.fill_polygon(&mapped, self.color);
    }

    fn rrect(&self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let rect = Rect::new(self.ox + x * self.s, self.oy + y * self.s, w * self.s, h * self.s);
        self.p.stroke_round(rect, r * self.s, self.color, self.width);
    }

    fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32) {
        self.p.fill_rect(Rect::new(self.ox + x * self.s, self.oy + y * self.s, w * self.s, h * self.s), self.color);
    }

    fn circle(&self, cx: f32, cy: f32, r: f32) {
        let c = self.pt(cx, cy);
        self.p.stroke_ellipse(c.X, c.Y, r * self.s, r * self.s, self.color, self.width);
    }

    fn fill_circle(&self, cx: f32, cy: f32, r: f32) {
        let c = self.pt(cx, cy);
        self.p.fill_circle(c.X, c.Y, r * self.s, self.color);
    }

    /// 用路径画（可以有曲线）
    fn path(&self, build: impl FnOnce(&ID2D1GeometrySink, &dyn Fn(f32, f32) -> windows_numerics::Vector2)) {
        let map = |x: f32, y: f32| self.pt(x, y);
        self.p.stroke_path(|sink| build(sink, &map), self.color, self.width);
    }

    /// 取景框的四个角
    fn brackets(&self, x: f32, y: f32, size: f32, arm: f32) {
        let (r, b) = (x + size, y + size);
        self.poly(&[(x, y + arm), (x, y), (x + arm, y)], false);
        self.poly(&[(r - arm, y), (r, y), (r, y + arm)], false);
        self.poly(&[(r, b - arm), (r, b), (r - arm, b)], false);
        self.poly(&[(x + arm, b), (x, b), (x, b - arm)], false);
    }
}

fn quad(sink: &ID2D1GeometrySink, control: windows_numerics::Vector2, end: windows_numerics::Vector2) {
    unsafe { sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: control, point2: end }) };
}

/// 在 `rect` 里居中画一个图标，`size` 是图标边长（DIP）
pub fn draw(p: &Painter, icon: Icon, rect: Rect, size: f32, color: Color) {
    let s = size / 16.0;
    let pen = Pen { p, ox: rect.mid_x() - size / 2.0, oy: rect.mid_y() - size / 2.0, s, color, width: (1.5 * s).max(1.0) };
    match icon {
        Icon::Screenshot => {
            pen.brackets(1.5, 2.5, 13.0, 3.5);
            pen.circle(8.0, 9.0, 2.6);
        }
        Icon::Folder => {
            pen.poly(&[(1.5, 4.0), (1.5, 12.5), (14.5, 12.5), (14.5, 5.5), (7.5, 5.5), (6.0, 3.5), (1.5, 3.5), (1.5, 4.0)], true);
        }
        Icon::Tuck => {
            pen.line(2.0, 2.0, 6.5, 6.5);
            pen.poly(&[(3.0, 6.5), (6.5, 6.5), (6.5, 3.0)], false);
            pen.line(14.0, 14.0, 9.5, 9.5);
            pen.poly(&[(13.0, 9.5), (9.5, 9.5), (9.5, 13.0)], false);
        }
        Icon::Settings => {
            // 八个齿的齿轮
            let (cx, cy) = (8.0f32, 8.0f32);
            let mut pts = Vec::new();
            for i in 0..32 {
                let angle = i as f32 / 32.0 * std::f32::consts::TAU;
                let phase = i % 4;
                let r = if phase == 0 || phase == 1 { 6.6 } else { 5.0 };
                pts.push((cx + r * angle.cos(), cy + r * angle.sin()));
            }
            pen.poly(&pts, true);
            pen.circle(cx, cy, 2.2);
        }
        Icon::SaveDoc => {
            pen.poly(&[(3.0, 1.5), (9.5, 1.5), (13.0, 5.0), (13.0, 14.5), (3.0, 14.5)], true);
            pen.poly(&[(9.5, 1.5), (9.5, 5.0), (13.0, 5.0)], false);
            pen.line(8.0, 6.5, 8.0, 12.0);
            pen.poly(&[(5.8, 9.8), (8.0, 12.0), (10.2, 9.8)], false);
        }
        Icon::Download => {
            pen.line(8.0, 2.0, 8.0, 10.5);
            pen.poly(&[(4.5, 7.0), (8.0, 10.5), (11.5, 7.0)], false);
            pen.poly(&[(2.5, 11.0), (2.5, 14.0), (13.5, 14.0), (13.5, 11.0)], false);
        }
        Icon::Eye => {
            pen.path(|sink, m| unsafe {
                sink.BeginFigure(m(1.0, 8.0), D2D1_FIGURE_BEGIN_HOLLOW);
                quad(sink, m(8.0, 0.5), m(15.0, 8.0));
                quad(sink, m(8.0, 15.5), m(1.0, 8.0));
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            });
            pen.circle(8.0, 8.0, 2.4);
        }
        Icon::TextScan => {
            pen.brackets(1.5, 1.5, 13.0, 3.5);
            pen.line(5.0, 6.0, 11.0, 6.0);
            pen.line(5.0, 8.5, 11.0, 8.5);
            pen.line(5.0, 11.0, 9.0, 11.0);
        }
        Icon::Close => {
            pen.line(3.5, 3.5, 12.5, 12.5);
            pen.line(12.5, 3.5, 3.5, 12.5);
        }
        Icon::Check => {
            pen.poly(&[(2.5, 8.5), (6.2, 12.0), (13.5, 4.0)], false);
        }
        Icon::CheckCircle => {
            pen.circle(8.0, 8.0, 6.5);
            pen.poly(&[(5.0, 8.3), (7.2, 10.4), (11.0, 6.0)], false);
        }
        Icon::TextLines => {
            pen.line(3.0, 4.5, 13.0, 4.5);
            pen.line(3.0, 8.0, 13.0, 8.0);
            pen.line(3.0, 11.5, 9.5, 11.5);
        }
        Icon::Trash => {
            pen.line(2.0, 4.0, 14.0, 4.0);
            pen.poly(&[(6.0, 4.0), (6.5, 2.0), (9.5, 2.0), (10.0, 4.0)], false);
            pen.poly(&[(3.5, 4.0), (4.5, 14.5), (11.5, 14.5), (12.5, 4.0)], false);
            pen.line(6.7, 7.0, 6.9, 11.5);
            pen.line(9.3, 7.0, 9.1, 11.5);
        }
        Icon::Copy => {
            pen.rrect(5.0, 5.0, 9.5, 9.5, 1.8);
            pen.poly(&[(3.0, 11.0), (1.5, 11.0), (1.5, 1.5), (11.0, 1.5), (11.0, 3.0)], false);
        }
        Icon::Clipboard => {
            pen.rrect(2.5, 2.5, 11.0, 12.5, 2.0);
            pen.rrect(5.5, 1.0, 5.0, 3.0, 1.0);
        }
        Icon::Pin => {
            pen.poly(&[(5.5, 2.0), (10.5, 2.0), (10.0, 7.0), (12.5, 9.5), (3.5, 9.5), (6.0, 7.0)], true);
            pen.line(8.0, 9.5, 8.0, 15.0);
        }
        Icon::Rectangle => {
            pen.rrect(2.0, 3.5, 12.0, 9.0, 1.2);
        }
        Icon::Ellipse => {
            let c = pen.pt(8.0, 8.0);
            p.stroke_ellipse(c.X, c.Y, 6.2 * s, 4.8 * s, color, pen.width);
        }
        Icon::Mosaic => {
            for row in 0..3 {
                for col in 0..3 {
                    if (row + col) % 2 == 0 {
                        pen.fill_rect(2.0 + col as f32 * 4.0, 2.0 + row as f32 * 4.0, 4.0, 4.0);
                    }
                }
            }
            pen.rrect(2.0, 2.0, 12.0, 12.0, 1.0);
        }
        Icon::Undo => {
            pen.path(|sink, m| unsafe {
                sink.BeginFigure(m(3.5, 6.0), D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddLine(m(10.0, 6.0));
                quad(sink, m(14.0, 6.0), m(14.0, 9.5));
                quad(sink, m(14.0, 13.0), m(10.0, 13.0));
                sink.AddLine(m(6.0, 13.0));
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            });
            pen.poly(&[(6.5, 3.0), (3.5, 6.0), (6.5, 9.0)], false);
        }
        Icon::Plus => {
            pen.line(8.0, 2.5, 8.0, 13.5);
            pen.line(2.5, 8.0, 13.5, 8.0);
        }
        Icon::ChevronDown => {
            pen.poly(&[(3.5, 6.0), (8.0, 10.5), (12.5, 6.0)], false);
        }
        Icon::Pencil => {
            pen.poly(&[(2.5, 13.5), (3.0, 10.5), (11.0, 2.5), (13.5, 5.0), (5.5, 13.0)], true);
            pen.line(9.5, 4.0, 12.0, 6.5);
        }
        Icon::Columns => {
            pen.rrect(1.5, 2.5, 13.0, 11.0, 2.0);
            pen.line(8.0, 2.5, 8.0, 13.5);
        }
        Icon::Document => {
            pen.poly(&[(3.0, 1.5), (9.5, 1.5), (13.0, 5.0), (13.0, 14.5), (3.0, 14.5)], true);
            pen.poly(&[(9.5, 1.5), (9.5, 5.0), (13.0, 5.0)], false);
            pen.line(5.5, 8.5, 10.5, 8.5);
            pen.line(5.5, 11.0, 10.5, 11.0);
        }
        Icon::Search => {
            pen.circle(6.8, 6.8, 4.6);
            pen.line(10.2, 10.2, 14.0, 14.0);
        }
        Icon::Photo => {
            pen.rrect(1.5, 2.5, 13.0, 11.0, 2.0);
            pen.poly(&[(1.8, 12.0), (6.0, 7.5), (9.0, 10.5), (11.0, 8.5), (14.2, 11.5)], false);
            pen.fill_circle(10.5, 5.8, 1.2);
        }
        Icon::ArrowOut => {
            pen.poly(&[(7.0, 2.5), (2.5, 2.5), (2.5, 13.5), (13.5, 13.5), (13.5, 9.0)], false);
            pen.line(8.0, 8.0, 13.5, 2.5);
            pen.poly(&[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)], false);
        }
        Icon::Palette => {
            pen.circle(8.0, 8.0, 6.3);
            pen.fill_circle(5.5, 6.0, 1.1);
            pen.fill_circle(8.5, 4.6, 1.1);
            pen.fill_circle(11.0, 7.0, 1.1);
        }
    }
}

// MARK: ClipMD 的标志

/// 取景框四角（圆角），`arm`、`radius` 是相对边长的比例
fn glyph_brackets(sink: &ID2D1GeometrySink, r: Rect, arm: f32, radius: f32) {
    let side = r.w.min(r.h);
    let (arm, radius) = (side * arm, side * radius);
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    unsafe {
        // 左上
        sink.BeginFigure(v2(x0, y0 + arm), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(v2(x0, y0 + radius));
        quad(sink, v2(x0, y0), v2(x0 + radius, y0));
        sink.AddLine(v2(x0 + arm, y0));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        // 右上
        sink.BeginFigure(v2(x1 - arm, y0), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(v2(x1 - radius, y0));
        quad(sink, v2(x1, y0), v2(x1, y0 + radius));
        sink.AddLine(v2(x1, y0 + arm));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        // 右下
        sink.BeginFigure(v2(x1, y1 - arm), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(v2(x1, y1 - radius));
        quad(sink, v2(x1, y1), v2(x1 - radius, y1));
        sink.AddLine(v2(x1 - arm, y1));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        // 左下
        sink.BeginFigure(v2(x0 + arm, y1), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(v2(x0 + radius, y1));
        quad(sink, v2(x0, y1), v2(x0, y1 - radius));
        sink.AddLine(v2(x0, y1 - arm));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }
}

/// "M↓"：M 用粗体，箭头自己画
fn markdown_mark(p: &Painter, cx: f32, cy: f32, size: f32, color: Color) {
    let font = Font::ui(size, 800);
    let m_width = crate::text::measure("M", font);
    let arrow_w = size * 0.62;
    let gap = size * 0.06;
    let total = m_width + gap + arrow_w;
    let x = cx - total / 2.0;
    p.text("M", Rect::new(x - 2.0, cy - size, m_width + 4.0, size * 2.0), font, color, TextOpts { ellipsis: false, ..TextOpts::CENTER });
    let ax = x + m_width + gap + arrow_w / 2.0;
    let top = cy - size * 0.36;
    let bottom = cy + size * 0.36;
    let w = (size * 0.13).max(1.0);
    p.line(ax, top, ax, bottom, color, w);
    p.polyline(&[(ax - arrow_w * 0.42, bottom - arrow_w * 0.42), (ax, bottom), (ax + arrow_w * 0.42, bottom - arrow_w * 0.42)], color, w, false);
}

/// 彩色的小标志（悬浮窗左上角）。深色模式和应用图标一样是深色底；浅色模式换成浅底深字
pub fn app_glyph(p: &Painter, rect: Rect, dark: bool) {
    let size = rect.w.min(rect.h);
    let r = Rect::centered(rect.mid_x(), rect.mid_y(), size, size);
    let corner = size * 0.27;
    p.soft_shadow(r, corner, Color::BLACK.alpha(if dark { 0.35 } else { 0.12 }), size * 0.06, size * 0.04);
    if dark {
        p.gradient_round(r, corner, Color::hex(0x3A3F49), Color::hex(0x16181C));
        p.stroke_round(r.inset(0.25, 0.25), corner, Color::WHITE.alpha(0.16), 0.5);
    } else {
        p.gradient_round(r, corner, Color::WHITE, Color::hex(0xE4E9F1));
        p.stroke_round(r.inset(0.25, 0.25), corner, Color::BLACK.alpha(0.10), 0.5);
    }
    let inner = r.inset(size * 0.17, size * 0.17);
    let (top, bottom) = if dark { (Color::hex(0x7DB6FF), Color::hex(0x2F7BF5)) } else { (Color::hex(0x4C93FF), Color::hex(0x1F63E6)) };
    p.stroke_path_gradient(|sink| glyph_brackets(sink, inner, 0.3, 0.12), inner, top, bottom, (size * 0.075).max(1.2));
    markdown_mark(p, r.mid_x(), r.mid_y(), size * 0.3, if dark { Color::WHITE } else { Color::hex(0x1E2633) });
}

/// 托盘用的单色标志：只有取景框和 M↓（太小时只画 M）
pub fn tray_glyph(p: &Painter, rect: Rect, color: Color, pixel_size: u32) {
    let size = rect.w.min(rect.h);
    let inner = rect.inset(size * 0.06, size * 0.06);
    let width = (size / 11.0).max(1.0);
    p.stroke_path(|sink| glyph_brackets(sink, inner, 0.3, 0.16), color, width);
    if pixel_size >= 24 {
        markdown_mark(p, rect.mid_x(), rect.mid_y(), size * 0.4, color);
    } else {
        let font = Font::ui(size * 0.5, 800);
        p.text("M", rect, font, color, TextOpts { ellipsis: false, ..TextOpts::CENTER });
    }
}

/// 把画出来的图做成系统图标（托盘用）
pub fn make_icon(pixel_size: u32, draw: impl FnOnce(&Painter)) -> Option<HICON> {
    let mut pixels = render_offscreen(pixel_size, pixel_size, 96.0, false, draw)?;
    // 系统图标要非预乘的透明度
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    unsafe {
        let header = BITMAPV5HEADER {
            bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
            bV5Width: pixel_size as i32,
            bV5Height: -(pixel_size as i32),
            bV5Planes: 1,
            bV5BitCount: 32,
            bV5Compression: BI_BITFIELDS,
            bV5RedMask: 0x00FF_0000,
            bV5GreenMask: 0x0000_FF00,
            bV5BlueMask: 0x0000_00FF,
            bV5AlphaMask: 0xFF00_0000,
            ..Default::default()
        };
        let screen = GetDC(None);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(Some(screen), &header as *const _ as *const BITMAPINFO, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        ReleaseDC(None, screen);
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        let mask = CreateBitmap(pixel_size as i32, pixel_size as i32, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
