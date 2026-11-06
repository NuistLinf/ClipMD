//! 抓屏：GDI 的 BitBlt 从屏幕复制像素（DWM 合成后的画面，和看到的一样）。

use windows::Win32::Graphics::Gdi::*;

use crate::images::Image;

/// 抓屏幕上一块区域（屏幕像素坐标）
pub fn grab_screen_rect(x: i32, y: i32, w: i32, h: i32) -> Option<Image> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        let mut result = None;
        if let Some(bitmap) = bitmap {
            let old = SelectObject(mem, bitmap.into());
            if BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok() {
                let len = (w * h * 4) as usize;
                let mut pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
                for px in pixels.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                result = Some(Image { width: w as u32, height: h as u32, pixels });
            }
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        result
    }
}
