//! 识别文字：用系统自带的 Windows.Media.Ocr，完全在本机。
//!
//! 和 macOS 版一样放在用完就退出的子进程里（`ClipMD.exe --ocr 图片`），识别引擎占的内存随子进程一起还给系统。
//! 优先用简体中文引擎（中英混排也认），没装中文识别包就用系统语言对应的引擎。

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

use crate::images::{self, Image};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 子进程的入口：识别图片，把文字以 UTF-8 写到标准输出
pub fn run_helper(path: Option<&str>) -> i32 {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let Some(path) = path else {
        eprintln!("用法：ClipMD.exe --ocr 图片路径");
        return 2;
    };
    let result = images::decode_file(Path::new(path), None).map_err(|e| e.to_string()).and_then(|image| recognize(&image));
    match result {
        Ok(text) => {
            let _ = std::io::stdout().write_all(text.as_bytes());
            0
        }
        Err(message) => {
            let _ = std::io::stderr().write_all(message.as_bytes());
            3
        }
    }
}

/// 在子进程里识别（主进程调用，会阻塞，放在后台线程）
pub fn recognize_in_helper(path: &Path) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let output = Command::new(exe)
        .arg("--ocr")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("启动识别进程失败：{e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if message.is_empty() { "识别失败".into() } else { message })
    }
}

fn engine() -> Result<OcrEngine, String> {
    for tag in ["zh-Hans-CN", "zh-Hans", "zh-CN"] {
        if let Ok(language) = Language::CreateLanguage(&HSTRING::from(tag)) {
            if OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                    return Ok(engine);
                }
            }
        }
    }
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Ok(engine);
    }
    let languages = OcrEngine::AvailableRecognizerLanguages().map_err(|e| e.to_string())?;
    if languages.Size().unwrap_or(0) > 0 {
        let language = languages.GetAt(0).map_err(|e| e.to_string())?;
        return OcrEngine::TryCreateFromLanguage(&language).map_err(|e| e.to_string());
    }
    Err("系统里没有可用的文字识别语言。在「设置 → 时间和语言 → 语言和区域」里给中文添加「光学字符识别」功能".into())
}

pub fn recognize(image: &Image) -> Result<String, String> {
    let engine = engine()?;
    // 太大的图先缩小；太小的图（一行小字）放大一点，识别更准
    let max = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(1000);
    let mut work = image.clone();
    if work.width.max(work.height) > max {
        work = scale(&work, max as f32 / work.width.max(work.height) as f32);
    } else if work.height < 40 {
        work = scale(&work, 2.0);
    }
    let writer = DataWriter::new().map_err(|e| e.to_string())?;
    writer.WriteBytes(&work.pixels).map_err(|e| e.to_string())?;
    let buffer = writer.DetachBuffer().map_err(|e| e.to_string())?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(&buffer, BitmapPixelFormat::Bgra8, work.width as i32, work.height as i32, BitmapAlphaMode::Premultiplied)
        .map_err(|e| e.to_string())?;
    let result = engine.RecognizeAsync(&bitmap).map_err(|e| e.to_string())?.join().map_err(|e| e.to_string())?;
    let lines = result.Lines().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..lines.Size().unwrap_or(0) {
        if let Ok(line) = lines.GetAt(i) {
            out.push(tidy(&line.Text().map(|t| t.to_string()).unwrap_or_default()));
        }
    }
    Ok(out.join("\n"))
}

fn scale(image: &Image, factor: f32) -> Image {
    let (w, h) = (((image.width as f32 * factor).round() as u32).max(1), ((image.height as f32 * factor).round() as u32).max(1));
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let sy = ((y as f32 + 0.5) / factor) as u32;
        let sy = sy.min(image.height - 1);
        for x in 0..w {
            let sx = (((x as f32 + 0.5) / factor) as u32).min(image.width - 1);
            let s = ((sy * image.width + sx) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            pixels[d..d + 4].copy_from_slice(&image.pixels[s..s + 4]);
        }
    }
    Image { width: w, height: h, pixels }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Windows 的识别结果在每个汉字之间都加了空格，去掉（中英之间的空格保留）
pub fn tidy(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' {
            let prev = out.chars().last();
            let next = chars.get(i + 1).copied();
            if let (Some(p), Some(n)) = (prev, next) {
                if is_cjk(p) || is_cjk(n) {
                    continue;
                }
            }
        }
        out.push(c);
    }
    out.trim().to_string()
}
