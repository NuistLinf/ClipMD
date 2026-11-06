//! 标注和导出：矩形、椭圆、马赛克，以及把选区连同标注画成一张图。
//!
//! 马赛克是一张"像素化"后的图，用粗笔刷沿着鼠标轨迹把它露出来。导出时先画马赛克、再画形状，
//! 和屏幕上看到的叠放顺序一样。

use windows::Win32::Graphics::Direct2D::*;
use windows::core::Interface;
use windows_numerics::Matrix3x2;

use crate::gfx::{self, Color, Painter, Rect};
use crate::images::Image;

/// 7 种颜色
pub const COLORS: [Color; 7] = [
    Color::hex(0xFF3B30),
    Color::hex(0xFF9500),
    Color::hex(0xFFCC00),
    Color::hex(0x34C759),
    Color::hex(0x0A84FF),
    Color::hex(0xBF5AF2),
    Color::hex(0xFFFFFF),
];
pub const LINE_WIDTHS: [f32; 3] = [2.0, 4.0, 7.0];
pub const BRUSH_WIDTHS: [f32; 3] = [12.0, 22.0, 36.0];
/// 马赛克的方块大小（DIP）
pub const MOSAIC_BLOCK: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Rectangle,
    Ellipse,
    Mosaic,
}

#[derive(Clone, Debug)]
pub enum Mark {
    Rectangle { rect: Rect, color: Color, width: f32 },
    Ellipse { rect: Rect, color: Color, width: f32 },
    Mosaic { points: Vec<(f32, f32)>, width: f32 },
}

impl Mark {
    pub fn is_mosaic(&self) -> bool {
        matches!(self, Mark::Mosaic { .. })
    }
}

/// 两个点围成的矩形
pub fn rect_from(a: (f32, f32), b: (f32, f32)) -> Rect {
    Rect::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

/// 像素化：按 `block` 像素一块取平均色
pub fn pixelate(image: &Image, block: u32) -> Image {
    let block = block.max(2);
    let (w, h) = (image.width, image.height);
    let mut out = image.pixels.clone();
    let mut by = 0;
    while by < h {
        let mut bx = 0;
        let bh = block.min(h - by);
        while bx < w {
            let bw = block.min(w - bx);
            let mut sum = [0u32; 4];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += image.pixels[i + c] as u32;
                    }
                }
            }
            let n = bw * bh;
            let avg = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, (sum[3] / n) as u8];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let i = ((y * w + x) * 4) as usize;
                    out[i..i + 4].copy_from_slice(&avg);
                }
            }
            bx += block;
        }
        by += block;
    }
    Image { width: w, height: h, pixels: out }
}

/// 平移、缩放标注：坐标变成 `(x - dx) * k`，线宽乘 `k`
pub fn transform_marks(marks: &[Mark], dx: f32, dy: f32, k: f32) -> Vec<Mark> {
    let r = |rect: &Rect| Rect::new((rect.x - dx) * k, (rect.y - dy) * k, rect.w * k, rect.h * k);
    marks
        .iter()
        .map(|m| match m {
            Mark::Rectangle { rect, color, width } => Mark::Rectangle { rect: r(rect), color: *color, width: width * k },
            Mark::Ellipse { rect, color, width } => Mark::Ellipse { rect: r(rect), color: *color, width: width * k },
            Mark::Mosaic { points, width } => Mark::Mosaic { points: points.iter().map(|(x, y)| ((x - dx) * k, (y - dy) * k)).collect(), width: width * k },
        })
        .collect()
}

/// 画标注。`mosaic` 是像素化后的图，`px_per_unit` 是这张图的像素和当前坐标单位的比例，
/// `origin` 是这张图左上角在当前坐标里的位置
pub fn draw_marks(p: &Painter, marks: &[Mark], mosaic: Option<&ID2D1Bitmap>, px_per_unit: f32, origin: (f32, f32)) {
    // 先马赛克，后形状
    if let Some(bitmap) = mosaic {
        let brush = unsafe {
            let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            };
            p.rt().CreateBitmapBrush(bitmap, Some(&props), None).ok()
        };
        if let Some(brush) = brush {
            unsafe {
                let m = Matrix3x2::scale(1.0 / px_per_unit, 1.0 / px_per_unit) * Matrix3x2::translation(origin.0, origin.1);
                brush.SetTransform(&m);
            }
            for mark in marks {
                if let Mark::Mosaic { points, width } = mark {
                    stroke_with(p, points, &brush.cast::<ID2D1Brush>().unwrap(), *width);
                }
            }
        }
    }
    for mark in marks {
        match mark {
            Mark::Rectangle { rect, color, width } => p.stroke_round(*rect, width * 0.6, *color, *width),
            Mark::Ellipse { rect, color, width } => p.stroke_ellipse(rect.mid_x(), rect.mid_y(), rect.w / 2.0, rect.h / 2.0, *color, *width),
            Mark::Mosaic { .. } => {}
        }
    }
}

fn stroke_with(p: &Painter, points: &[(f32, f32)], brush: &ID2D1Brush, width: f32) {
    if points.is_empty() {
        return;
    }
    unsafe {
        let Ok(geometry) = p.g.factory.CreatePathGeometry() else { return };
        let Ok(sink) = geometry.Open() else { return };
        let (x0, y0) = points[0];
        sink.BeginFigure(gfx::v2(x0, y0), windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_HOLLOW);
        if points.len() == 1 {
            sink.AddLine(gfx::v2(x0 + 0.01, y0));
        }
        for &(x, y) in &points[1..] {
            sink.AddLine(gfx::v2(x, y));
        }
        sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_OPEN);
        if sink.Close().is_err() {
            return;
        }
        let style = p
            .g
            .factory
            .CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                    transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
                },
                None,
            )
            .ok();
        p.ctx.DrawGeometry(&geometry, brush, width, style.as_ref().and_then(|s| s.cast::<ID2D1StrokeStyle>().ok()).as_ref());
    }
}

/// 把选区连同标注画成一张图。`px` 是选区在 frozen 里的像素矩形，`selection` 是同一块的 DIP 矩形，
/// 标注坐标是屏幕 DIP，`scale` 是像素/DIP
pub fn export(frozen: &Image, px: (u32, u32, u32, u32), selection: Rect, marks: &[Mark], scale: f32) -> Option<Image> {
    let (x, y, w, h) = px;
    let crop = frozen.crop(x, y, w, h);
    if crop.width == 0 || crop.height == 0 {
        return None;
    }
    let bitmap = gfx::bitmap_from_bgra(crop.width, crop.height, crop.stride(), &crop.pixels)?;
    let mosaic = if marks.iter().any(Mark::is_mosaic) {
        let pix = pixelate(&crop, (MOSAIC_BLOCK * scale).round() as u32);
        gfx::bitmap_from_bgra(pix.width, pix.height, pix.stride(), &pix.pixels)
    } else {
        None
    };
    let local = transform_marks(marks, selection.x, selection.y, scale);
    let pixels = gfx::render_offscreen(crop.width, crop.height, 96.0, false, |p| {
        p.bitmap(&bitmap, Rect::new(0.0, 0.0, crop.width as f32, crop.height as f32), 1.0, false);
        p.clip(Rect::new(0.0, 0.0, crop.width as f32, crop.height as f32), || {
            draw_marks(p, &local, mosaic.as_ref(), 1.0, (0.0, 0.0));
        });
    })?;
    Some(Image { width: crop.width, height: crop.height, pixels })
}
