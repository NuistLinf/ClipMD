//! 绘制底座：Direct3D 11 + Direct2D + DirectComposition。
//!
//! 每个窗口一条带透明通道的合成交换链（`Surface`），所有窗口共用一个 Direct2D 设备上下文（界面全在主线程画）。
//! 窗口不用 GDI 的重定向位图（WS_EX_NOREDIRECTIONBITMAP），透明的地方直接露出 DWM 画的亚克力背景。

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

use crate::log;

// MARK: 基本类型

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const CLEAR: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    pub const fn hex(rgb: u32) -> Color {
        Color {
            r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
            g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
            b: (rgb & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }

    pub const fn alpha(self, a: f32) -> Color {
        Color { a: self.a * a, ..self }
    }

    pub fn mix(self, other: Color, t: f32) -> Color {
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    pub fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
}

/// 设备无关像素（DIP）的矩形
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn mid_x(&self) -> f32 {
        self.x + self.w / 2.0
    }

    pub fn mid_y(&self) -> f32 {
        self.y + self.h / 2.0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn centered(cx: f32, cy: f32, w: f32, h: f32) -> Rect {
        Rect::new(cx - w / 2.0, cy - h / 2.0, w, h)
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

pub fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

// MARK: 全局设备

pub struct Gfx {
    pub d3d: ID3D11Device,
    pub dxgi_device: IDXGIDevice,
    pub dxgi_factory: IDXGIFactory2,
    pub factory: ID2D1Factory1,
    pub ctx: ID2D1DeviceContext,
    pub dcomp: IDCompositionDevice,
    /// 设备丢失重建后加一，各窗口据此重建交换链
    pub generation: u32,
    round_stroke: ID2D1StrokeStyle,
}

thread_local! {
    static GFX: RefCell<Option<Rc<Gfx>>> = const { RefCell::new(None) };
    static GENERATION: Cell<u32> = const { Cell::new(0) };
}

pub fn gfx() -> Rc<Gfx> {
    GFX.with(|cell| {
        if let Some(g) = cell.borrow().as_ref() {
            return g.clone();
        }
        let generation = GENERATION.with(|g| {
            g.set(g.get() + 1);
            g.get()
        });
        let created = Rc::new(create_gfx(generation).expect("创建 Direct3D/Direct2D 设备失败"));
        *cell.borrow_mut() = Some(created.clone());
        created
    })
}

/// 显卡驱动更新、远程桌面切换等情况会让设备失效，扔掉重建
pub fn reset_device() {
    log!("graphics device lost, recreating");
    GFX.with(|cell| *cell.borrow_mut() = None);
}

fn create_d3d(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_9_3]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))
}

fn create_gfx(generation: u32) -> Result<Gfx> {
    unsafe {
        // 没有可用显卡（虚拟机、远程桌面的某些配置）时用软件渲染
        let d3d = create_d3d(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create_d3d(D3D_DRIVER_TYPE_WARP))?;
        let dxgi_device: IDXGIDevice = d3d.cast()?;
        let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
        let options = D2D1_FACTORY_OPTIONS { debugLevel: D2D1_DEBUG_LEVEL_NONE };
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options))?;
        let device = factory.CreateDevice(&dxgi_device)?;
        let ctx = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
        let round_stroke = factory.CreateStrokeStyle(
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
        )?;
        Ok(Gfx { d3d, dxgi_device, dxgi_factory, factory, ctx, dcomp, generation, round_stroke: round_stroke.cast()? })
    }
}

// MARK: 窗口的交换链

pub struct Surface {
    hwnd: HWND,
    /// true：普通的窗口交换链（窗口里有编辑框这类子控件时用，子控件才能正常显示）
    direct: bool,
    swapchain: Option<IDXGISwapChain1>,
    target: Option<IDCompositionTarget>,
    visual: Option<IDCompositionVisual>,
    size: (u32, u32),
    generation: u32,
}

impl Surface {
    pub fn new(hwnd: HWND) -> Surface {
        Surface { hwnd, direct: false, swapchain: None, target: None, visual: None, size: (0, 0), generation: 0 }
    }

    /// 不透明、带子控件的窗口用
    pub fn new_direct(hwnd: HWND) -> Surface {
        Surface { hwnd, direct: true, swapchain: None, target: None, visual: None, size: (0, 0), generation: 0 }
    }

    fn ensure(&mut self, g: &Gfx, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        if self.generation != g.generation {
            self.swapchain = None;
            self.target = None;
            self.visual = None;
        }
        unsafe {
            if self.swapchain.is_none() && self.direct {
                let desc = DXGI_SWAP_CHAIN_DESC1 {
                    Width: width,
                    Height: height,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    Stereo: false.into(),
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 1,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                    AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
                    Flags: 0,
                };
                let swapchain = g.dxgi_factory.CreateSwapChainForHwnd(&g.d3d, self.hwnd, &desc, None, None)?;
                self.swapchain = Some(swapchain);
                self.size = (width, height);
                self.generation = g.generation;
            } else if self.swapchain.is_none() {
                let desc = DXGI_SWAP_CHAIN_DESC1 {
                    Width: width,
                    Height: height,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    Stereo: false.into(),
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                    Flags: 0,
                };
                let swapchain = g.dxgi_factory.CreateSwapChainForComposition(&g.d3d, &desc, None)?;
                let target = g.dcomp.CreateTargetForHwnd(self.hwnd, true)?;
                let visual = g.dcomp.CreateVisual()?;
                visual.SetContent(&swapchain)?;
                target.SetRoot(&visual)?;
                g.dcomp.Commit()?;
                self.swapchain = Some(swapchain);
                self.target = Some(target);
                self.visual = Some(visual);
                self.size = (width, height);
                self.generation = g.generation;
            } else if self.size != (width, height) {
                if let Some(swapchain) = &self.swapchain {
                    swapchain.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
                }
                self.size = (width, height);
            }
        }
        Ok(())
    }

    /// 画一帧。`width`、`height` 是像素，`dpi` 决定 DIP 和像素的比例
    pub fn render(&mut self, width: u32, height: u32, dpi: f32, dark: bool, draw: impl FnOnce(&Painter)) {
        // 窗口还没建好（创建过程中的 WM_SIZE）时不画
        if self.hwnd.0.is_null() || width == 0 || height == 0 {
            return;
        }
        let g = gfx();
        let result = (|| -> Result<()> {
            self.ensure(&g, width, height)?;
            let swapchain = self.swapchain.as_ref().unwrap();
            unsafe {
                let surface: IDXGISurface = swapchain.GetBuffer(0)?;
                let alpha = if self.direct { D2D1_ALPHA_MODE_IGNORE } else { D2D1_ALPHA_MODE_PREMULTIPLIED };
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: alpha },
                    dpiX: dpi,
                    dpiY: dpi,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    colorContext: std::mem::ManuallyDrop::new(None),
                };
                let bitmap = g.ctx.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
                g.ctx.SetTarget(&bitmap);
                g.ctx.SetDpi(dpi, dpi);
                g.ctx.BeginDraw();
                g.ctx.SetTransform(&Matrix3x2::identity());
                g.ctx.Clear(Some(&Color::CLEAR.d2d()));
                {
                    let painter = Painter::new(&g, dark)?;
                    draw(&painter);
                }
                let end = g.ctx.EndDraw(None, None);
                g.ctx.SetTarget(None);
                end?;
                swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            log!("render failed: {error:?}");
            self.swapchain = None;
            self.target = None;
            self.visual = None;
            if error.code() == windows::Win32::Foundation::D2DERR_RECREATE_TARGET || error.code() == DXGI_ERROR_DEVICE_REMOVED || error.code() == DXGI_ERROR_DEVICE_RESET {
                reset_device();
            }
        }
    }

    /// 关掉窗口前释放显存
    pub fn release(&mut self) {
        self.swapchain = None;
        self.visual = None;
        self.target = None;
        self.size = (0, 0);
        if let Some(g) = GFX.with(|c| c.borrow().clone()) {
            unsafe {
                let _ = g.dcomp.Commit();
            }
        }
    }
}

// MARK: 画笔

/// 一帧里用的画图工具：一支纯色画刷换颜色用，外加整体透明度
pub struct Painter<'a> {
    pub ctx: &'a ID2D1DeviceContext,
    pub g: &'a Gfx,
    brush: ID2D1SolidColorBrush,
    alpha: Cell<f32>,
    pub dark: bool,
}

impl<'a> Painter<'a> {
    pub fn new(g: &'a Gfx, dark: bool) -> Result<Painter<'a>> {
        let brush = unsafe { g.ctx.CreateSolidColorBrush(&Color::BLACK.d2d(), None)? };
        Ok(Painter { ctx: &g.ctx, g, brush, alpha: Cell::new(1.0), dark })
    }

    /// 设备上下文覆盖了一些老接口的方法（参数更多），要用老版本时通过它调用
    pub fn rt(&self) -> &ID2D1RenderTarget {
        self.ctx
    }

    pub fn brush(&self, color: Color) -> &ID2D1SolidColorBrush {
        let c = color.alpha(self.alpha.get());
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    pub fn alpha(&self) -> f32 {
        self.alpha.get()
    }

    /// 在整体透明度 `alpha` 下画
    pub fn with_alpha<R>(&self, alpha: f32, f: impl FnOnce() -> R) -> R {
        let old = self.alpha.get();
        self.alpha.set(old * alpha.clamp(0.0, 1.0));
        let r = f();
        self.alpha.set(old);
        r
    }

    pub fn translate<R>(&self, dx: f32, dy: f32, f: impl FnOnce() -> R) -> R {
        unsafe {
            let mut old = Matrix3x2::identity();
            self.ctx.GetTransform(&mut old);
            let moved = Matrix3x2 { M31: old.M31 + dx, M32: old.M32 + dy, ..old };
            self.ctx.SetTransform(&moved);
            let r = f();
            self.ctx.SetTransform(&old);
            r
        }
    }

    pub fn transform<R>(&self, m: Matrix3x2, f: impl FnOnce() -> R) -> R {
        unsafe {
            let mut old = Matrix3x2::identity();
            self.ctx.GetTransform(&mut old);
            self.ctx.SetTransform(&(m * old));
            let r = f();
            self.ctx.SetTransform(&old);
            r
        }
    }

    pub fn clip<R>(&self, rect: Rect, f: impl FnOnce() -> R) -> R {
        unsafe {
            self.ctx.PushAxisAlignedClip(&rect.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let r = f();
            self.ctx.PopAxisAlignedClip();
            r
        }
    }

    /// 圆角裁剪（用图层，开销比矩形裁剪大，只在需要时用）
    pub fn clip_round<R>(&self, rect: Rect, radius: f32, f: impl FnOnce() -> R) -> R {
        unsafe {
            let geometry = match self.g.factory.CreateRoundedRectangleGeometry(&round(rect, radius)) {
                Ok(g) => g,
                Err(_) => return f(),
            };
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: rect.d2d(),
                geometricMask: std::mem::ManuallyDrop::new(Some(geometry.cast().unwrap())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.rt().PushLayer(&params, None);
            let r = f();
            self.ctx.PopLayer();
            std::mem::ManuallyDrop::into_inner(params.geometricMask);
            r
        }
    }

    /// 上下边缘渐隐（列表滚动时内容在边缘淡出）。`top`、`bottom` 是渐隐带的高度
    pub fn fade_layer<R>(&self, rect: Rect, top: f32, bottom: f32, f: impl FnOnce() -> R) -> R {
        if top <= 0.0 && bottom <= 0.0 {
            return self.clip(rect, f);
        }
        unsafe {
            let h = rect.h.max(1.0);
            let t = (top / h).clamp(0.0, 0.45);
            let b = (1.0 - bottom / h).clamp(0.55, 1.0);
            let white = |a: f32| Color::WHITE.alpha(a).d2d();
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: white(if top > 0.0 { 0.0 } else { 1.0 }) },
                D2D1_GRADIENT_STOP { position: t, color: white(1.0) },
                D2D1_GRADIENT_STOP { position: b, color: white(1.0) },
                D2D1_GRADIENT_STOP { position: 1.0, color: white(if bottom > 0.0 { 0.0 } else { 1.0 }) },
            ];
            let Ok(collection) = self.rt().CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return f() };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(rect.x, rect.y), endPoint: v2(rect.x, rect.bottom()) };
            let Ok(brush) = self.ctx.CreateLinearGradientBrush(&props, None, &collection) else { return f() };
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: rect.d2d(),
                geometricMask: std::mem::ManuallyDrop::new(None),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(brush.cast().ok()),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.rt().PushLayer(&params, None);
            let r = f();
            self.ctx.PopLayer();
            std::mem::ManuallyDrop::into_inner(params.opacityBrush);
            r
        }
    }

    pub fn fill_rect(&self, rect: Rect, color: Color) {
        unsafe { self.ctx.FillRectangle(&rect.d2d(), self.brush(color)) };
    }

    pub fn stroke_rect(&self, rect: Rect, color: Color, width: f32) {
        unsafe { self.ctx.DrawRectangle(&rect.d2d(), self.brush(color), width, None) };
    }

    pub fn fill_round(&self, rect: Rect, radius: f32, color: Color) {
        unsafe { self.ctx.FillRoundedRectangle(&round(rect, radius), self.brush(color)) };
    }

    pub fn stroke_round(&self, rect: Rect, radius: f32, color: Color, width: f32) {
        unsafe { self.ctx.DrawRoundedRectangle(&round(rect, radius), self.brush(color), width, None) };
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, color: Color) {
        self.fill_ellipse(cx, cy, r, r, color);
    }

    pub fn fill_ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32, color: Color) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: rx, radiusY: ry };
        unsafe { self.ctx.FillEllipse(&e, self.brush(color)) };
    }

    pub fn stroke_ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32, color: Color, width: f32) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: rx, radiusY: ry };
        unsafe { self.ctx.DrawEllipse(&e, self.brush(color), width, None) };
    }

    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, color: Color, width: f32) {
        unsafe { self.ctx.DrawLine(v2(x1, y1), v2(x2, y2), self.brush(color), width, &self.g.round_stroke) };
    }

    /// 上下渐变的圆角矩形
    pub fn gradient_round(&self, rect: Rect, radius: f32, top: Color, bottom: Color) {
        unsafe {
            let a = self.alpha.get();
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: top.alpha(a).d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: bottom.alpha(a).d2d() },
            ];
            let Ok(collection) = self.rt().CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(rect.x, rect.y), endPoint: v2(rect.x, rect.bottom()) };
            if let Ok(brush) = self.ctx.CreateLinearGradientBrush(&props, None, &collection) {
                self.ctx.FillRoundedRectangle(&round(rect, radius), &brush);
            }
        }
    }

    /// 一串点连成的折线（圆头圆角）
    pub fn polyline(&self, points: &[(f32, f32)], color: Color, width: f32, closed: bool) {
        if let Some(geometry) = self.path(points, closed) {
            unsafe { self.ctx.DrawGeometry(&geometry, self.brush(color), width, &self.g.round_stroke) };
        }
    }

    pub fn fill_polygon(&self, points: &[(f32, f32)], color: Color) {
        if let Some(geometry) = self.path(points, true) {
            unsafe { self.ctx.FillGeometry(&geometry, self.brush(color), None) };
        }
    }

    fn path(&self, points: &[(f32, f32)], closed: bool) -> Option<ID2D1PathGeometry> {
        if points.len() < 2 {
            return None;
        }
        unsafe {
            let geometry = self.g.factory.CreatePathGeometry().ok()?;
            let sink = geometry.Open().ok()?;
            sink.BeginFigure(v2(points[0].0, points[0].1), if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
            for &(x, y) in &points[1..] {
                sink.AddLine(v2(x, y));
            }
            sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            sink.Close().ok()?;
            Some(geometry.cast().ok()?)
        }
    }

    /// 二次贝塞尔曲线组成的路径（画标志的取景框角）
    pub fn stroke_path(&self, build: impl FnOnce(&ID2D1GeometrySink), color: Color, width: f32) {
        unsafe {
            let Ok(geometry) = self.g.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geometry.Open() else { return };
            build(&sink);
            if sink.Close().is_ok() {
                self.ctx.DrawGeometry(&geometry, self.brush(color), width, &self.g.round_stroke);
            }
        }
    }

    pub fn stroke_path_gradient(&self, build: impl FnOnce(&ID2D1GeometrySink), rect: Rect, top: Color, bottom: Color, width: f32) {
        unsafe {
            let Ok(geometry) = self.g.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geometry.Open() else { return };
            build(&sink);
            if sink.Close().is_err() {
                return;
            }
            let a = self.alpha.get();
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: top.alpha(a).d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: bottom.alpha(a).d2d() },
            ];
            let Ok(collection) = self.rt().CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(rect.x, rect.y), endPoint: v2(rect.x, rect.bottom()) };
            if let Ok(brush) = self.ctx.CreateLinearGradientBrush(&props, None, &collection) {
                self.ctx.DrawGeometry(&geometry, &brush, width, &self.g.round_stroke);
            }
        }
    }

    pub fn bitmap(&self, bitmap: &ID2D1Bitmap, dest: Rect, opacity: f32, smooth: bool) {
        let mode = if smooth { D2D1_BITMAP_INTERPOLATION_MODE_LINEAR } else { D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR };
        unsafe { self.rt().DrawBitmap(bitmap, Some(&dest.d2d()), opacity * self.alpha.get(), mode, None) };
    }

    pub fn bitmap_part(&self, bitmap: &ID2D1Bitmap, dest: Rect, source: Rect, opacity: f32, smooth: bool) {
        let mode = if smooth { D2D1_BITMAP_INTERPOLATION_MODE_LINEAR } else { D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR };
        unsafe { self.rt().DrawBitmap(bitmap, Some(&dest.d2d()), opacity * self.alpha.get(), mode, Some(&source.d2d())) };
    }

    /// 柔和的阴影：几层逐渐变大、变淡的圆角矩形
    pub fn soft_shadow(&self, rect: Rect, radius: f32, color: Color, spread: f32, dy: f32) {
        let steps = 6;
        for i in 0..steps {
            let t = (i + 1) as f32 / steps as f32;
            let grow = spread * t;
            let r = Rect::new(rect.x - grow, rect.y - grow + dy, rect.w + grow * 2.0, rect.h + grow * 2.0);
            self.fill_round(r, radius + grow, color.alpha((1.0 - t) * 0.35));
        }
    }
}

pub fn round(rect: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    D2D1_ROUNDED_RECT { rect: rect.d2d(), radiusX: r, radiusY: r }
}

/// 从 BGRA（预乘）像素建一张位图
pub fn bitmap_from_bgra(width: u32, height: u32, stride: u32, pixels: &[u8]) -> Option<ID2D1Bitmap> {
    let g = gfx();
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    unsafe {
        let rt: &ID2D1RenderTarget = &g.ctx;
        rt.CreateBitmap(D2D_SIZE_U { width, height }, Some(pixels.as_ptr() as *const _), stride, &props)
            .inspect_err(|e| log!("CreateBitmap {width}x{height} failed: {e:?}"))
            .ok()
    }
}

/// 离屏画一张图，读回 BGRA 像素（自检截图、导出标注后的截图用）
pub fn render_offscreen(width: u32, height: u32, dpi: f32, dark: bool, draw: impl FnOnce(&Painter)) -> Option<Vec<u8>> {
    let g = gfx();
    unsafe {
        let format = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
        let target_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: format,
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let size = D2D_SIZE_U { width, height };
        let target = g.ctx.CreateBitmap(size, None, 0, &target_props).ok()?;
        let read_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: format,
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let readback = g.ctx.CreateBitmap(size, None, 0, &read_props).ok()?;
        g.ctx.SetTarget(&target);
        g.ctx.SetDpi(dpi, dpi);
        g.ctx.BeginDraw();
        g.ctx.SetTransform(&Matrix3x2::identity());
        g.ctx.Clear(Some(&Color::CLEAR.d2d()));
        {
            let painter = Painter::new(&g, dark).ok()?;
            draw(&painter);
        }
        let end = g.ctx.EndDraw(None, None);
        g.ctx.SetTarget(None);
        end.ok()?;
        readback.CopyFromBitmap(None, &target, None).ok()?;
        let mapped = readback.Map(D2D1_MAP_OPTIONS_READ).ok()?;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for row in 0..height as usize {
            let src = std::slice::from_raw_parts(mapped.bits.add(row * mapped.pitch as usize), width as usize * 4);
            pixels[row * width as usize * 4..(row + 1) * width as usize * 4].copy_from_slice(src);
        }
        let _ = readback.Unmap();
        Some(pixels)
    }
}
