//! Hidden mode for a lent window: see-through (alpha 1) and click-through, still drawing.
//! The original extended style is saved so `restore` puts it back exactly. Windows only.

use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetLayeredWindowAttributes, GetWindowLongPtrW, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, LWA_ALPHA,
    SET_WINDOW_POS_FLAGS, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_LAYERED, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};

use super::win::{hwnd, is_alive, os_err};
use super::{Hwnd, LendError, Result};

/// What `make_see_through` changed, so `restore` can undo it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedStyle {
    pub ex_style: isize,
    pub was_layered: bool,
    /// Alpha the window had when it was already layered (None if not layered or unreadable).
    pub alpha: Option<u8>,
    /// The window was already always-on-top, so `restore` leaves it there.
    pub was_topmost: bool,
}

/// Make the window see-through and click-through. Returns what to restore later.
pub fn make_see_through(h: Hwnd) -> Result<SavedStyle> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let hw = hwnd(h);
    let ex = unsafe { GetWindowLongPtrW(hw, GWL_EXSTYLE) };
    let was_layered = is_layered_style(ex);
    let alpha = if was_layered { read_alpha(h) } else { None };
    let saved = SavedStyle { ex_style: ex, was_layered, alpha, was_topmost: is_topmost_style(ex) };

    unsafe { SetWindowLongPtrW(hw, GWL_EXSTYLE, see_through_style(ex)) };
    if let Err(e) = unsafe { SetLayeredWindowAttributes(hw, COLORREF(0), 1, LWA_ALPHA) } {
        // Roll the style back so a failed call leaves the window as it was.
        unsafe { SetWindowLongPtrW(hw, GWL_EXSTYLE, ex) };
        return Err(os_err(e));
    }
    // Always on top, so other windows cannot cover it (apps that stop drawing when covered keep drawing). The window is
    // invisible and click-through, so it blocks nothing. Best effort.
    let _ = unsafe { SetWindowPos(hw, Some(HWND_TOPMOST), 0, 0, 0, 0, TOPMOST_FLAGS) };
    Ok(saved)
}

/// Undo `make_see_through`. `Gone` if the window has closed (nothing to restore).
pub fn restore(h: Hwnd, saved: SavedStyle) -> Result<()> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let hw = hwnd(h);
    unsafe { SetWindowLongPtrW(hw, GWL_EXSTYLE, saved.ex_style) };
    if !saved.was_topmost {
        let _ = unsafe { SetWindowPos(hw, Some(HWND_NOTOPMOST), 0, 0, 0, 0, TOPMOST_FLAGS) };
    }
    if saved.was_layered {
        if let Some(a) = saved.alpha {
            unsafe { SetLayeredWindowAttributes(hw, COLORREF(0), a, LWA_ALPHA) }.map_err(os_err)?;
        }
    }
    Ok(())
}

/// Never activates, moves, resizes or reorders owners.
const TOPMOST_FLAGS: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOACTIVATE.0 | SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOOWNERZORDER.0);

/// Is the window minimized right now? Read-only.
pub fn is_iconic(h: Hwnd) -> bool {
    is_alive(h) && unsafe { IsIconic(hwnd(h)) }.as_bool()
}

/// Bring a minimized window back to its normal place WITHOUT activating it (SW_SHOWNOACTIVATE; never SW_RESTORE or SW_SHOW).
pub fn show_no_activate(h: Hwnd) -> Result<()> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let _ = unsafe { ShowWindow(hwnd(h), SW_SHOWNOACTIVATE) };
    Ok(())
}

fn read_alpha(h: Hwnd) -> Option<u8> {
    let mut alpha: u8 = 0;
    let ok = unsafe { GetLayeredWindowAttributes(hwnd(h), None, Some(&mut alpha as *mut u8), None) };
    ok.ok().map(|_| alpha)
}

/// Pure: does this extended style already have WS_EX_LAYERED?
pub(super) fn is_layered_style(ex_style: isize) -> bool {
    (ex_style as u32) & WS_EX_LAYERED.0 != 0
}

/// Pure: does this extended style already have WS_EX_TOPMOST?
pub(super) fn is_topmost_style(ex_style: isize) -> bool {
    (ex_style as u32) & WS_EX_TOPMOST.0 != 0
}

/// Pure: the extended style for see-through: layered + transparent (click-through), other bits kept.
pub(super) fn see_through_style(ex_style: isize) -> isize {
    ex_style | (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn see_through_adds_layered_and_transparent_and_keeps_other_bits() {
        let plain = 0x0000_0100isize; // WS_EX_WINDOWEDGE, unrelated to our bits
        let out = see_through_style(plain);
        assert!(is_layered_style(out));
        assert_eq!(out as u32 & WS_EX_TRANSPARENT.0, WS_EX_TRANSPARENT.0);
        assert_eq!(out & plain, plain);
    }

    #[test]
    fn see_through_is_idempotent() {
        let once = see_through_style(0x0000_0008);
        assert_eq!(see_through_style(once), once);
    }

    #[test]
    fn layered_detection() {
        assert!(!is_layered_style(0));
        assert!(is_layered_style(WS_EX_LAYERED.0 as isize));
    }

    #[test]
    fn saved_style_round_trip_fields() {
        let s = SavedStyle { ex_style: 0x40, was_layered: true, alpha: Some(200), was_topmost: false };
        assert_eq!(s, SavedStyle { ex_style: 0x40, was_layered: true, alpha: Some(200), was_topmost: false });
        assert_ne!(s.alpha, None);
    }

    #[test]
    fn topmost_detection() {
        assert!(!is_topmost_style(0));
        assert!(!is_topmost_style(see_through_style(0)));
        assert!(is_topmost_style(WS_EX_TOPMOST.0 as isize | 0x100));
    }

    #[test]
    fn topmost_flags_never_activate_move_or_resize() {
        let f = TOPMOST_FLAGS.0;
        for flag in [SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOOWNERZORDER] {
            assert_eq!(f & flag.0, flag.0);
        }
    }
}
