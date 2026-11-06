//! 图片编解码：用系统的 WIC（PNG、JPEG、BMP、GIF、WebP……都支持），不引入图片库。

use std::cell::OnceCell;
use std::path::Path;

use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::core::{GUID, Result};

/// 一张 BGRA 图（预乘透明度），每行紧挨着
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn stride(&self) -> u32 {
        self.width * 4
    }

    /// 内容指纹（剪贴板记录去重）：抽样算哈希，大图也很快
    pub fn fingerprint(&self) -> String {
        let step = (self.pixels.len() / 65_536).max(1) * 4;
        let mut sample = Vec::with_capacity(self.pixels.len() / step * 4 + 8);
        sample.extend_from_slice(&self.width.to_le_bytes());
        sample.extend_from_slice(&self.height.to_le_bytes());
        for chunk in self.pixels.chunks(step) {
            sample.extend_from_slice(&chunk[..chunk.len().min(4)]);
        }
        format!("{:016x}", clipmd_core::text::fnv1a(&sample))
    }

    /// 裁一块（像素坐标）
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Image {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = ((row * self.width + x) * 4) as usize;
            pixels.extend_from_slice(&self.pixels[start..start + (w * 4) as usize]);
        }
        Image { width: w, height: h, pixels }
    }
}

thread_local! {
    static WIC: OnceCell<IWICImagingFactory> = const { OnceCell::new() };
}

pub fn wic() -> IWICImagingFactory {
    WIC.with(|cell| {
        cell.get_or_init(|| unsafe {
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).expect("WIC 不可用")
        })
        .clone()
    })
}

fn read_source(source: &IWICBitmapSource, max_side: Option<u32>) -> Result<Image> {
    unsafe {
        let factory = wic();
        let (mut w, mut h) = (0u32, 0u32);
        source.GetSize(&mut w, &mut h)?;
        let mut source: IWICBitmapSource = source.clone();
        if let Some(max) = max_side {
            if w.max(h) > max {
                let ratio = max as f64 / w.max(h) as f64;
                let (nw, nh) = (((w as f64 * ratio).round() as u32).max(1), ((h as f64 * ratio).round() as u32).max(1));
                let scaler = factory.CreateBitmapScaler()?;
                scaler.Initialize(&source, nw, nh, WICBitmapInterpolationModeHighQualityCubic)?;
                source = windows::core::Interface::cast(&scaler)?;
                w = nw;
                h = nh;
            }
        }
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&source, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        Ok(Image { width: w, height: h, pixels })
    }
}

/// 解码图片文件。`max_side` 限制长边（缩略图、预览不用解出整张大图）
pub fn decode_file(path: &Path, max_side: Option<u32>) -> Result<Image> {
    unsafe {
        let factory = wic();
        let wide = crate::util::h(&path.to_string_lossy());
        let decoder = factory.CreateDecoderFromFilename(&wide, None, GENERIC_READ_ACCESS, WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        read_source(&windows::core::Interface::cast(&frame)?, max_side)
    }
}

const GENERIC_READ_ACCESS: windows::Win32::Foundation::GENERIC_ACCESS_RIGHTS = windows::Win32::Foundation::GENERIC_READ;

/// 解码内存里的图片（剪贴板里的 PNG）
pub fn decode_bytes(bytes: &[u8], max_side: Option<u32>) -> Result<Image> {
    unsafe {
        let factory = wic();
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        read_source(&windows::core::Interface::cast(&frame)?, max_side)
    }
}

/// 只读尺寸，不解码像素
pub fn size_of_file(path: &Path) -> Option<(u32, u32)> {
    unsafe {
        let factory = wic();
        let wide = crate::util::h(&path.to_string_lossy());
        let decoder = factory.CreateDecoderFromFilename(&wide, None, GENERIC_READ_ACCESS, WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).ok()?;
        Some((w, h))
    }
}

/// BGRA（预乘）→ PNG
pub fn encode_png(image: &Image) -> Result<Vec<u8>> {
    unsafe {
        let factory = wic();
        let stream: IStream = windows::Win32::UI::Shell::SHCreateMemStream(None).ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY))?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut bag: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut bag)?;
        let frame = frame.unwrap();
        frame.Initialize(bag.as_ref())?;
        frame.SetSize(image.width, image.height)?;
        // PNG 存非预乘的 BGRA：让 WIC 从预乘格式转
        let bitmap = factory.CreateBitmapFromMemory(image.width, image.height, &GUID_WICPixelFormat32bppPBGRA, image.stride(), &image.pixels)?;
        let mut format: GUID = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        if format == GUID_WICPixelFormat32bppBGRA {
            let converter = factory.CreateFormatConverter()?;
            converter.Initialize(&bitmap, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
            frame.WriteSource(&converter, std::ptr::null())?;
        } else {
            frame.WriteSource(&bitmap, std::ptr::null())?;
        }
        frame.Commit()?;
        encoder.Commit()?;
        read_stream(&stream)
    }
}

fn read_stream(stream: &IStream) -> Result<Vec<u8>> {
    unsafe {
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let size = stat.cbSize as usize;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut data = vec![0u8; size];
        let mut read = 0u32;
        stream.Read(data.as_mut_ptr() as *mut _, size as u32, Some(&mut read)).ok()?;
        data.truncate(read as usize);
        Ok(data)
    }
}

pub fn write_png(image: &Image, path: &Path) -> std::io::Result<()> {
    let data = encode_png(image).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(path, data)
}

/// 剪贴板里的 DIB（CF_DIB / CF_DIBV5）→ 图片
pub fn from_dib(data: &[u8]) -> Option<Image> {
    if data.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let i32_at = |o: usize| i32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let u16_at = |o: usize| u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
    let header_size = u32_at(0) as usize;
    let width = i32_at(4);
    let height = i32_at(8);
    let bits = u16_at(14);
    let compression = u32_at(16);
    let colors_used = u32_at(32) as usize;
    if width <= 0 || height == 0 || width > 30_000 || height.abs() > 30_000 {
        return None;
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    let top_down = height < 0;
    // BI_BITFIELDS 的三个掩码放在 40 字节的头后面
    let mut offset = header_size;
    if compression == 3 && header_size == 40 {
        offset += 12;
    }
    let palette_entries = if bits <= 8 { if colors_used > 0 { colors_used } else { 1 << bits } } else { 0 };
    let palette_offset = offset;
    offset += palette_entries * 4;
    let stride = (w as usize * bits as usize).div_ceil(32) * 4;
    if data.len() < offset + stride * h as usize || !(compression == 0 || compression == 3) {
        return None;
    }
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut any_alpha = false;
    for row in 0..h as usize {
        let src_row = if top_down { row } else { h as usize - 1 - row };
        let src = &data[offset + src_row * stride..offset + (src_row + 1) * stride];
        let dst = &mut pixels[row * w as usize * 4..(row + 1) * w as usize * 4];
        for x in 0..w as usize {
            let (b, g, r, a) = match bits {
                32 => (src[x * 4], src[x * 4 + 1], src[x * 4 + 2], src[x * 4 + 3]),
                24 => (src[x * 3], src[x * 3 + 1], src[x * 3 + 2], 255),
                8 | 4 | 1 => {
                    let index = match bits {
                        8 => src[x] as usize,
                        4 => ((src[x / 2] >> (if x % 2 == 0 { 4 } else { 0 })) & 0x0F) as usize,
                        _ => ((src[x / 8] >> (7 - x % 8)) & 1) as usize,
                    };
                    let p = palette_offset + index * 4;
                    if p + 3 >= data.len() {
                        return None;
                    }
                    (data[p], data[p + 1], data[p + 2], 255)
                }
                _ => return None,
            };
            if bits == 32 && a != 0 {
                any_alpha = true;
            }
            dst[x * 4..x * 4 + 4].copy_from_slice(&[b, g, r, a]);
        }
    }
    // 很多程序放的 32 位 DIB 透明度全是 0，其实是不透明的
    if bits == 32 && !any_alpha {
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 255;
        }
    } else if bits == 32 {
        // DIB 是非预乘的，转成预乘
        for px in pixels.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a < 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
    }
    Some(Image { width: w, height: h, pixels })
}

/// 图片 → CF_DIBV5（给只认位图的老程序，比如画图）
pub fn to_dibv5(image: &Image) -> Vec<u8> {
    let (w, h) = (image.width, image.height);
    let mut data = Vec::with_capacity(124 + (w * h * 4) as usize);
    let push32 = |d: &mut Vec<u8>, v: u32| d.extend_from_slice(&v.to_le_bytes());
    push32(&mut data, 124); // bV5Size
    data.extend_from_slice(&(w as i32).to_le_bytes());
    data.extend_from_slice(&(h as i32).to_le_bytes()); // 自下而上
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    push32(&mut data, 3); // BI_BITFIELDS
    push32(&mut data, w * h * 4);
    push32(&mut data, 2835);
    push32(&mut data, 2835);
    push32(&mut data, 0);
    push32(&mut data, 0);
    push32(&mut data, 0x00FF_0000);
    push32(&mut data, 0x0000_FF00);
    push32(&mut data, 0x0000_00FF);
    push32(&mut data, 0xFF00_0000);
    push32(&mut data, 0x7352_4742); // LCS_sRGB
    data.resize(124, 0);
    // bV5Intent = LCS_GM_IMAGES
    data[108..112].copy_from_slice(&4u32.to_le_bytes());
    for row in (0..h as usize).rev() {
        for px in image.pixels[row * w as usize * 4..(row + 1) * w as usize * 4].chunks_exact(4) {
            let a = px[3] as u32;
            let un = |c: u8| if a == 0 || a == 255 { c } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
            data.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), px[3]]);
        }
    }
    data
}

#[allow(dead_code)]
pub fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a == b
}
