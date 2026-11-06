//! 编辑框：系统的 RichEdit 控件（纯文本模式）。中文输入法、撤销、查找替换快捷键这些都是现成的。

use std::sync::OnceLock;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::UI::Controls::RichEdit::*;
use windows::Win32::UI::Controls::{EM_EMPTYUNDOBUFFER, EM_SETMODIFY, EM_SETRECT, SetWindowTheme};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::gfx::{Color, Rect};

pub struct Editor {
    pub hwnd: HWND,
}

fn load_library() {
    static LOADED: OnceLock<()> = OnceLock::new();
    LOADED.get_or_init(|| unsafe {
        let _ = LoadLibraryW(w!("Msftedit.dll"));
    });
}

fn colorref(c: Color) -> COLORREF {
    COLORREF(((c.b * 255.0) as u32) << 16 | ((c.g * 255.0) as u32) << 8 | (c.r * 255.0) as u32)
}

impl Editor {
    pub fn new(parent: HWND, id: usize) -> Option<Editor> {
        load_library();
        unsafe {
            let style = WS_CHILD | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_NOHIDESEL | ES_WANTRETURN) as u32);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("RICHEDIT50W"),
                w!(""),
                style,
                0,
                0,
                100,
                100,
                Some(parent),
                Some(HMENU(id as *mut _)),
                Some(crate::window::instance()),
                None,
            )
            .ok()?;
            SendMessageW(hwnd, EM_SETTEXTMODE, Some(WPARAM((TM_PLAINTEXT.0 | TM_MULTILEVELUNDO.0 | TM_MULTICODEPAGE.0) as usize)), None);
            SendMessageW(hwnd, EM_SETEVENTMASK, None, Some(LPARAM(ENM_CHANGE as isize)));
            // 不限长度
            SendMessageW(hwnd, EM_EXLIMITTEXT, None, Some(LPARAM(64 * 1024 * 1024)));
            let editor = Editor { hwnd };
            editor.apply_theme(crate::theme::is_dark());
            Some(editor)
        }
    }

    pub fn apply_theme(&self, dark: bool) {
        let pal = crate::theme::palette_for(dark);
        let bg = if dark { Color::hex(0x1F2023) } else { Color::hex(0xFFFFFF) };
        let fg = if dark { Color::hex(0xE6E6E8) } else { Color::hex(0x1E1F22) };
        let _ = pal;
        unsafe {
            SendMessageW(self.hwnd, EM_SETBKGNDCOLOR, Some(WPARAM(0)), Some(LPARAM(colorref(bg).0 as isize)));
            let mut format = CHARFORMAT2W::default();
            format.Base.cbSize = std::mem::size_of::<CHARFORMAT2W>() as u32;
            format.Base.dwMask = CFM_FACE | CFM_SIZE | CFM_COLOR | CFM_CHARSET;
            format.Base.yHeight = 210; // 10.5 磅
            format.Base.crTextColor = colorref(fg);
            format.Base.bCharSet = windows::Win32::Graphics::Gdi::DEFAULT_CHARSET;
            let face: Vec<u16> = crate::text::mono_family().encode_utf16().take(31).collect();
            format.Base.szFaceName[..face.len()].copy_from_slice(&face);
            SendMessageW(self.hwnd, EM_SETCHARFORMAT, Some(WPARAM(SCF_ALL as usize)), Some(LPARAM(&format as *const _ as isize)));
            let _ = SetWindowTheme(self.hwnd, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, None);
        }
    }

    pub fn set_text(&self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\n', "\r\n");
        let wide = crate::util::wide(&normalized);
        unsafe {
            SendMessageW(self.hwnd, WM_SETTEXT, None, Some(LPARAM(wide.as_ptr() as isize)));
            SendMessageW(self.hwnd, EM_EMPTYUNDOBUFFER, None, None);
            SendMessageW(self.hwnd, EM_SETMODIFY, Some(WPARAM(0)), None);
        }
    }

    /// 文字（换行统一成 \n）
    pub fn text(&self) -> String {
        unsafe {
            let len = SendMessageW(self.hwnd, WM_GETTEXTLENGTH, None, None).0 as usize;
            let mut buf = vec![0u16; len + 1];
            SendMessageW(self.hwnd, WM_GETTEXT, Some(WPARAM(buf.len())), Some(LPARAM(buf.as_mut_ptr() as isize)));
            crate::util::from_wide(&buf).replace("\r\n", "\n").replace('\r', "\n")
        }
    }

    /// 放到父窗口里的位置（DIP 矩形，按缩放换成像素），四周留点边距
    pub fn place(&self, rect: Rect, scale: f32) {
        unsafe {
            let (x, y, w, h) = crate::window::dip_rect_to_px(rect, scale);
            let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            let pad = (14.0 * scale) as i32;
            let inner = RECT { left: pad, top: (10.0 * scale) as i32, right: w - pad, bottom: h - (10.0 * scale) as i32 };
            SendMessageW(self.hwnd, EM_SETRECT, Some(WPARAM(0)), Some(LPARAM(&inner as *const _ as isize)));
        }
    }

    pub fn show(&self, visible: bool) {
        unsafe {
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOW } else { SW_HIDE });
        }
    }

    pub fn focus(&self) {
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(self.hwnd));
        }
    }

}
