//! 按代码页解码（GB18030、GBK、Windows-1252……）。
//!
//! 用系统自带的转换：Windows 上是 MultiByteToWideChar，macOS（开发、跑测试时）是 CoreFoundation。
//! 不引入 encoding_rs，省下近 1 MB 的编码表。

/// GB18030（兼容 GBK、GB2312），中文 Windows 上老文本文件常见的编码
pub const GB18030: u32 = 54936;

/// 严格解码：有非法字节就返回 None
pub fn decode(codepage: u32, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    imp::decode(codepage, bytes)
}

pub fn decode_utf16le(bytes: &[u8]) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).ok()
}

pub fn decode_utf16be(bytes: &[u8]) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).ok()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};

    pub fn decode(codepage: u32, bytes: &[u8]) -> Option<String> {
        let len = i32::try_from(bytes.len()).ok()?;
        // 有的代码页（如 54936 以外的一些）不支持 MB_ERR_INVALID_CHARS，失败时退回宽松模式
        let flags = MB_ERR_INVALID_CHARS;
        unsafe {
            let mut needed = MultiByteToWideChar(codepage, flags, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
            let strict = needed > 0;
            if !strict {
                if codepage == super::GB18030 {
                    return None;
                }
                needed = MultiByteToWideChar(codepage, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
                if needed <= 0 {
                    return None;
                }
            }
            let mut buffer = vec![0u16; needed as usize];
            let written = MultiByteToWideChar(codepage, if strict { flags } else { 0 }, bytes.as_ptr(), len, buffer.as_mut_ptr(), needed);
            if written <= 0 {
                return None;
            }
            buffer.truncate(written as usize);
            String::from_utf16(&buffer).ok()
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringConvertWindowsCodepageToEncoding(codepage: u32) -> u32;
        fn CFStringCreateWithBytes(alloc: *const c_void, bytes: *const u8, len: isize, encoding: u32, external: u8) -> *const c_void;
        fn CFStringGetLength(s: *const c_void) -> isize;
        fn CFStringGetCharacters(s: *const c_void, range: CFRange, buffer: *mut u16);
        fn CFRelease(p: *const c_void);
    }

    pub fn decode(codepage: u32, bytes: &[u8]) -> Option<String> {
        unsafe {
            let encoding = CFStringConvertWindowsCodepageToEncoding(codepage);
            if encoding == 0xFFFF_FFFF {
                return None;
            }
            let s = CFStringCreateWithBytes(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, encoding, 0);
            if s.is_null() {
                return None;
            }
            let len = CFStringGetLength(s);
            let mut buffer = vec![0u16; len as usize];
            CFStringGetCharacters(s, CFRange { location: 0, length: len }, buffer.as_mut_ptr());
            CFRelease(s);
            String::from_utf16(&buffer).ok()
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    /// 其他平台只认 ASCII（ClipMD 只发布 Windows 和 macOS 版）
    pub fn decode(_codepage: u32, bytes: &[u8]) -> Option<String> {
        bytes.is_ascii().then(|| String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn gb18030() {
        let bytes = hex("d6d0cec42057696e646f777320c9cfb4e6b5c4cec4b1be");
        assert_eq!(decode(GB18030, &bytes).as_deref(), Some("中文 Windows 上存的文本"));
        assert_eq!(decode(936, &hex("b4f8")).as_deref(), Some("带"));
    }

    #[test]
    fn utf16() {
        assert_eq!(decode_utf16le(&[0x2d, 0x4e, 0x87, 0x65]).as_deref(), Some("中文"));
        assert_eq!(decode_utf16be(&[0x4e, 0x2d]).as_deref(), Some("中"));
    }
}
