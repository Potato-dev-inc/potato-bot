//! Background input for lent windows. Only PostMessage / SendMessageTimeout: never the real
//! mouse, keyboard, pointer or focus (no SendInput, SetFocus, AttachThreadInput).


use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, MapWindowPoints};
use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW};
use windows::Win32::UI::WindowsAndMessaging::{
    CWP_SKIPDISABLED, CWP_SKIPINVISIBLE, ChildWindowFromPointEx, EnumChildWindows, GetClassNameW,
    PostMessageW, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

use super::win::{hwnd, is_alive, os_err};
use super::{Hwnd, LendError, Result};

/// MK_LBUTTON (winuser.h).
const MK_LBUTTON: usize = 0x0001;
/// Pause between typed characters so slow controls keep up.
const CHAR_GAP: std::time::Duration = std::time::Duration::from_millis(5);

/// Click at client point (x, y) of the lent window.
pub fn click(h: Hwnd, x: i32, y: i32) -> Result<()> {
    let (t, tx, ty) = target_at(h, x, y)?;
    let lp = make_lparam(tx, ty);
    post(t, WM_MOUSEMOVE, WPARAM(0), lp)?;
    post(t, WM_LBUTTONDOWN, WPARAM(MK_LBUTTON), lp)?;
    post(t, WM_LBUTTONUP, WPARAM(0), lp)?;
    Ok(())
}

/// Double click at client point (x, y).
pub fn double_click(h: Hwnd, x: i32, y: i32) -> Result<()> {
    let (t, tx, ty) = target_at(h, x, y)?;
    let lp = make_lparam(tx, ty);
    post(t, WM_MOUSEMOVE, WPARAM(0), lp)?;
    post(t, WM_LBUTTONDOWN, WPARAM(MK_LBUTTON), lp)?;
    post(t, WM_LBUTTONUP, WPARAM(0), lp)?;
    post(t, WM_LBUTTONDBLCLK, WPARAM(MK_LBUTTON), lp)?;
    post(t, WM_LBUTTONUP, WPARAM(0), lp)?;
    Ok(())
}

/// The second half of a double click: the first click was already sent on its own, so Windows sees down, up, DBLCLK, up.
pub fn double_click_second(h: Hwnd, x: i32, y: i32) -> Result<()> {
    let (t, tx, ty) = target_at(h, x, y)?;
    let lp = make_lparam(tx, ty);
    post(t, WM_LBUTTONDBLCLK, WPARAM(MK_LBUTTON), lp)?;
    post(t, WM_LBUTTONUP, WPARAM(0), lp)?;
    Ok(())
}

/// Type text into the lent window's edit control (or the window itself if none).
/// One WM_CHAR per UTF-16 unit; newlines become Enter.
pub fn type_text(h: Hwnd, text: &str) -> Result<()> {
    ensure_alive(h)?;
    let t = edit_target(hwnd(h));
    for unit in text_units(text) {
        post(t, WM_CHAR, WPARAM(unit as usize), LPARAM(1))?;
        std::thread::sleep(CHAR_GAP);
    }
    Ok(())
}

/// Press and release a virtual key (VK_*). Enter and Backspace also post their WM_CHAR,
/// as the keyboard does, because edit controls act on the character message.
pub fn key(h: Hwnd, vk: u16) -> Result<()> {
    ensure_alive(h)?;
    let t = edit_target(hwnd(h));
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) };
    let (down, up) = key_lparams(scan, is_extended(vk));
    post(t, WM_KEYDOWN, WPARAM(vk as usize), LPARAM(down))?;
    if let Some(ch) = char_for_key(vk) {
        post(t, WM_CHAR, WPARAM(ch as usize), LPARAM(down))?;
    }
    post(t, WM_KEYUP, WPARAM(vk as usize), LPARAM(up))?;
    Ok(())
}

/// Mouse wheel at client point (x, y). `delta` in wheel units (120 = one notch; negative = down).
pub fn scroll(h: Hwnd, x: i32, y: i32, delta: i32) -> Result<()> {
    let (t, _, _) = target_at(h, x, y)?;
    // WM_MOUSEWHEEL carries screen coordinates in lParam.
    let mut pt = POINT { x, y };
    if !unsafe { ClientToScreen(hwnd(h), &mut pt) }.as_bool() {
        return Err(LendError::Os("ClientToScreen failed".to_string()));
    }
    post(t, WM_MOUSEWHEEL, WPARAM(wheel_wparam(delta)), make_lparam(pt.x, pt.y))?;
    Ok(())
}

fn ensure_alive(h: Hwnd) -> Result<()> {
    if is_alive(h) { Ok(()) } else { Err(LendError::Gone) }
}

fn post(t: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Result<()> {
    unsafe { PostMessageW(Some(t), msg, w, l) }.map_err(os_err)
}

/// Deepest visible, enabled child under client point (x, y) of the window.
/// Returns that child and the point in its client coordinates.
fn target_at(h: Hwnd, x: i32, y: i32) -> Result<(HWND, i32, i32)> {
    ensure_alive(h)?;
    let flags = CWP_SKIPINVISIBLE | CWP_SKIPDISABLED;
    let mut cur = hwnd(h);
    let (mut px, mut py) = (x, y);
    loop {
        let child = unsafe { ChildWindowFromPointEx(cur, POINT { x: px, y: py }, flags) };
        if child.0.is_null() || child == cur {
            break;
        }
        let mut pts = [POINT { x: px, y: py }];
        unsafe { MapWindowPoints(Some(cur), Some(child), &mut pts) };
        cur = child;
        px = pts[0].x;
        py = pts[0].y;
    }
    Ok((cur, px, py))
}

unsafe extern "system" fn collect_cb(h: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    list.push(h);
    BOOL(1)
}

/// First descendant whose class contains "edit" (covers "Edit" and "RichEdit*"), else the window.
fn edit_target(h: HWND) -> HWND {
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumChildWindows(Some(h), Some(collect_cb), LPARAM(&mut all as *mut Vec<HWND> as isize));
    }
    all.into_iter()
        .find(|&c| {
            let mut buf = [0u16; 256];
            let n = unsafe { GetClassNameW(c, &mut buf) };
            String::from_utf16_lossy(&buf[..n.max(0) as usize])
                .to_ascii_lowercase()
                .contains("edit")
        })
        .unwrap_or(h)
}

/// Pure: MAKELPARAM. Low word x, high word y, each 16-bit (negatives wrap).
pub(super) fn make_lparam(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u16 as u32) << 16) | (x as u16 as u32)) as isize)
}

/// Pure: WPARAM for WM_MOUSEWHEEL: wheel delta in the high word, 16-bit signed.
pub(super) fn wheel_wparam(delta: i32) -> usize {
    (delta as i16 as u16 as usize) << 16
}

/// Pure: text as the UTF-16 units to post. "\r\n" and "\n" become one "\r"; NUL is dropped.
pub(super) fn text_units(text: &str) -> Vec<u16> {
    text.replace("\r\n", "\n")
        .encode_utf16()
        .filter(|&u| u != 0)
        .map(|u| if u == 0x0A { 0x0D } else { u })
        .collect()
}

/// Pure: WM_KEYDOWN and WM_KEYUP lParams. Bits: 0-15 repeat count, 16-23 scan code,
/// 24 extended key, 30 previous state, 31 transition.
pub(super) fn key_lparams(scan: u32, extended: bool) -> (isize, isize) {
    let ext: u32 = if extended { 1 << 24 } else { 0 };
    let down: u32 = 1 | ((scan & 0xFF) << 16) | ext;
    let up: u32 = down | (1 << 30) | (1 << 31);
    (down as isize, up as isize)
}

/// Pure: extended-key virtual keys (arrows, Home, End, PageUp/Down, Insert, Delete).
pub(super) fn is_extended(vk: u16) -> bool {
    (0x21..=0x28).contains(&vk) || vk == 0x2D || vk == 0x2E
}

/// Pure: the WM_CHAR a key also produces, for keys edit controls act on.
pub(super) fn char_for_key(vk: u16) -> Option<u16> {
    match vk {
        0x0D => Some(0x0D), // Enter
        0x08 => Some(0x08), // Backspace
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lparam_packs_x_low_y_high() {
        assert_eq!(make_lparam(0x0102, 0x0304).0, 0x0304_0102);
        assert_eq!(make_lparam(0, 0).0, 0);
    }

    #[test]
    fn lparam_negative_coordinates_wrap_to_16_bits() {
        assert_eq!(make_lparam(-1, -1).0 as u32, 0xFFFF_FFFF);
        assert_eq!(make_lparam(-2, 5).0 as u32, 0x0005_FFFE);
    }

    #[test]
    fn wheel_delta_in_high_word() {
        assert_eq!(wheel_wparam(120), 0x0078usize << 16);
        assert_eq!(wheel_wparam(-120), 0xFF88usize << 16);
        assert_eq!(wheel_wparam(0), 0);
    }

    #[test]
    fn text_splits_into_utf16_units() {
        assert_eq!(text_units("a"), vec![0x61]);
        // U+1F600 is a surrogate pair.
        assert_eq!(text_units("a\u{1F600}"), vec![0x61, 0xD83D, 0xDE00]);
    }

    #[test]
    fn newlines_become_enter() {
        assert_eq!(text_units("a\nb"), vec![0x61, 0x0D, 0x62]);
        assert_eq!(text_units("a\r\nb"), vec![0x61, 0x0D, 0x62]);
        assert_eq!(text_units("x\0y"), vec![0x78, 0x79]);
        assert!(text_units("").is_empty());
    }

    #[test]
    fn key_flags_for_enter() {
        let (down, up) = key_lparams(0x1C, false);
        assert_eq!(down, 0x001C_0001);
        assert_eq!(up, 0xC01C_0001u32 as isize);
    }

    #[test]
    fn extended_keys_set_bit_24() {
        assert!(is_extended(0x25)); // Left
        assert!(is_extended(0x2E)); // Delete
        assert!(!is_extended(0x0D));
        assert!(!is_extended(0x41)); // A
        let (down, _) = key_lparams(0x4B, true);
        assert_eq!(down & (1 << 24), 1 << 24);
    }

    #[test]
    fn enter_and_backspace_post_chars() {
        assert_eq!(char_for_key(0x0D), Some(0x0D));
        assert_eq!(char_for_key(0x08), Some(0x08));
        assert_eq!(char_for_key(0x09), None); // Tab
        assert_eq!(char_for_key(0x1B), None); // Escape
    }
}
