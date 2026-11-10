//! Window list and window info for lending. Windows only. Read-only: never moves,
//! activates or focuses a window.

use std::ffi::c_void;

use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, WS_EX_TOOLWINDOW,
};

use super::{Hwnd, LendError, Rect, Result, WinInfo};

/// Classes that are never offered: desktop, taskbar, and their helpers.
const SKIP_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

pub(super) fn os_err(e: impl std::fmt::Display) -> LendError {
    LendError::Os(e.to_string())
}

/// Build the `HWND` for a lend handle.
pub(super) fn hwnd(h: Hwnd) -> HWND {
    HWND(h as *mut c_void)
}

/// True while the window handle still names a live window.
pub fn is_alive(h: Hwnd) -> bool {
    unsafe { IsWindow(Some(hwnd(h))) }.as_bool()
}

/// Handle of the window that has focus right now (0 if none). Read-only.
#[allow(dead_code)] // kept for callers that need the focus owner (read-only)
pub fn foreground() -> Hwnd {
    unsafe { GetForegroundWindow() }.0 as isize
}

/// Process id that owns the window (0 if the handle is gone).
pub fn pid_of(h: Hwnd) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd(h), Some(&mut pid)) };
    pid
}

/// Full info for one window. `Gone` if the handle is no longer a window.
pub fn info(h: Hwnd) -> Result<WinInfo> {
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    Ok(build_info(hwnd(h)))
}

/// Visible, titled, user-facing top-level windows that belong to other processes.
pub fn candidates() -> Vec<WinInfo> {
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_cb), LPARAM(&mut found as *mut Vec<HWND> as isize));
    }
    let me = unsafe { GetCurrentProcessId() };
    found
        .into_iter()
        .filter(|&h| is_candidate(h, me))
        .map(build_info)
        .collect()
}

unsafe extern "system" fn collect_cb(h: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    list.push(h);
    BOOL(1)
}

fn is_candidate(h: HWND, me: u32) -> bool {
    if !unsafe { IsWindowVisible(h) }.as_bool() {
        return false;
    }
    if pid_of(h.0 as isize) == me {
        return false;
    }
    if is_cloaked(h) || is_tool_window(h) {
        return false;
    }
    if read_title(h).trim().is_empty() {
        return false;
    }
    !skip_class(&class_of(h))
}

fn is_cloaked(h: HWND) -> bool {
    let mut cloaked = 0u32;
    let ok = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

fn is_tool_window(h: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) };
    is_tool_style(ex)
}

/// Pure: does this extended style have WS_EX_TOOLWINDOW?
pub(super) fn is_tool_style(ex_style: isize) -> bool {
    (ex_style as u32) & WS_EX_TOOLWINDOW.0 != 0
}

/// Pure: classes that are never offered to lend.
pub(super) fn skip_class(class: &str) -> bool {
    SKIP_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(class))
}

pub(super) fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Window class name of a lend handle (empty when the window is gone). Read-only.
pub fn class_name(h: Hwnd) -> String {
    class_of(hwnd(h))
}

fn read_title(h: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(h) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn read_rect(h: HWND) -> Rect {
    let mut r = RECT::default();
    if unsafe { GetWindowRect(h, &mut r) }.is_ok() {
        rect_from_ltrb(r.left, r.top, r.right, r.bottom)
    } else {
        Rect { x: 0, y: 0, w: 0, h: 0 }
    }
}

fn build_info(h: HWND) -> WinInfo {
    let pid = pid_of(h.0 as isize);
    let (exe_path, elevated) = process_info(pid);
    let exe = exe_file_name(&exe_path);
    let description = if exe_path.is_empty() { None } else { file_description(&exe_path) };
    WinInfo {
        hwnd: h.0 as isize,
        pid,
        title: read_title(h),
        app_name: app_name_from(description.as_deref(), &exe),
        exe,
        rect: read_rect(h),
        elevated,
        minimized: unsafe { IsIconic(h) }.as_bool(),
    }
}

/// Full image path and elevation of a process. Empty path and `false` if it cannot be opened.
fn process_info(pid: u32) -> (String, bool) {
    if pid == 0 {
        return (String::new(), false);
    }
    unsafe {
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (String::new(), false);
        };
        let mut buf = vec![0u16; 4096];
        let mut size = buf.len() as u32;
        let path = if QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size)
            .is_ok()
        {
            String::from_utf16_lossy(&buf[..size as usize])
        } else {
            String::new()
        };
        let elevated = token_elevated(proc);
        let _ = CloseHandle(proc);
        (path, elevated)
    }
}

unsafe fn token_elevated(proc: HANDLE) -> bool {
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(proc, TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elev = TOKEN_ELEVATION::default();
    let mut ret = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut TOKEN_ELEVATION as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        )
    };
    let _ = unsafe { CloseHandle(token) };
    ok.is_ok() && elev.TokenIsElevated != 0
}

/// FileDescription from the executable's version resource, if it has one.
fn file_description(path: &str) -> Option<String> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            None,
            size,
            data.as_mut_ptr() as *mut c_void,
        )
        .ok()?;
        let block = data.as_ptr() as *const c_void;

        // First translation pair (language, codepage) from the version block.
        let mut tr: *mut c_void = std::ptr::null_mut();
        let mut tr_len = 0u32;
        let tr_key = wide_z("\\VarFileInfo\\Translation");
        if !VerQueryValueW(block, PCWSTR(tr_key.as_ptr()), &mut tr, &mut tr_len).as_bool()
            || tr.is_null()
            || tr_len < 4
        {
            return None;
        }
        let words = std::slice::from_raw_parts(tr as *const u16, 2);
        let key = wide_z(&string_info_path(words[0], words[1]));

        let mut val: *mut c_void = std::ptr::null_mut();
        let mut val_len = 0u32;
        if !VerQueryValueW(block, PCWSTR(key.as_ptr()), &mut val, &mut val_len).as_bool()
            || val.is_null()
            || val_len == 0
        {
            return None;
        }
        let units = std::slice::from_raw_parts(val as *const u16, val_len as usize);
        let text = String::from_utf16_lossy(units);
        let text = text.trim_end_matches('\0').trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}

fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Pure: the version-resource key for FileDescription in one language and codepage.
pub(super) fn string_info_path(lang: u16, codepage: u16) -> String {
    format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription")
}

/// Pure: name to show people. FileDescription when present, else the exe name without `.exe`.
pub(super) fn app_name_from(description: Option<&str>, exe: &str) -> String {
    match description.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => d.to_string(),
        None => exe_stem(exe),
    }
}

/// Pure: `LINE.exe` -> `LINE` (case-insensitive suffix).
pub(super) fn exe_stem(exe: &str) -> String {
    let len = exe.len();
    if len >= 4 && exe.is_char_boundary(len - 4) && exe[len - 4..].eq_ignore_ascii_case(".exe") {
        exe[..len - 4].to_string()
    } else {
        exe.to_string()
    }
}

/// Pure: last path component of a full path (`C:\X\LINE.exe` -> `LINE.exe`).
pub(super) fn exe_file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

/// Pure: RECT corners to position and size.
pub(super) fn rect_from_ltrb(l: i32, t: i32, r: i32, b: i32) -> Rect {
    Rect { x: l, y: t, w: r - l, h: b - t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_stem_strips_exe_only() {
        assert_eq!(exe_stem("LINE.exe"), "LINE");
        assert_eq!(exe_stem("notepad.EXE"), "notepad");
        assert_eq!(exe_stem("tool"), "tool");
        assert_eq!(exe_stem("a.exe.b"), "a.exe.b");
        assert_eq!(exe_stem(""), "");
    }

    #[test]
    fn exe_file_name_takes_last_component() {
        assert_eq!(exe_file_name(r"C:\Program Files\LINE\LINE.exe"), "LINE.exe");
        assert_eq!(exe_file_name("C:/x/a.exe"), "a.exe");
        assert_eq!(exe_file_name(""), "");
    }

    #[test]
    fn app_name_prefers_description() {
        assert_eq!(app_name_from(Some("LINE"), "LINE.exe"), "LINE");
        assert_eq!(app_name_from(Some("  LINE for Windows  "), "LINE.exe"), "LINE for Windows");
        assert_eq!(app_name_from(Some("   "), "LINE.exe"), "LINE");
        assert_eq!(app_name_from(None, "Notepad.exe"), "Notepad");
    }

    #[test]
    fn string_info_key_is_lowercase_hex() {
        assert_eq!(string_info_path(0x0411, 0x04B0), "\\StringFileInfo\\041104b0\\FileDescription");
        assert_eq!(string_info_path(0x0409, 0x04e4), "\\StringFileInfo\\040904e4\\FileDescription");
    }

    #[test]
    fn rect_math() {
        assert_eq!(rect_from_ltrb(10, 20, 110, 80), Rect { x: 10, y: 20, w: 100, h: 60 });
        assert_eq!(rect_from_ltrb(-1920, 0, 0, 1080), Rect { x: -1920, y: 0, w: 1920, h: 1080 });
    }

    #[test]
    fn shell_classes_are_skipped() {
        assert!(skip_class("Progman"));
        assert!(skip_class("shell_traywnd"));
        assert!(skip_class("WorkerW"));
        assert!(!skip_class("Chrome_WidgetWin_1"));
        assert!(!skip_class("Notepad"));
    }

    #[test]
    fn tool_window_bit() {
        assert!(is_tool_style(WS_EX_TOOLWINDOW.0 as isize));
        assert!(!is_tool_style(0));
        assert!(!is_tool_style(0x0008)); // WS_EX_TOPMOST
    }
}

/// Where the client area starts inside the window's own picture (`capture::png` draws the whole window, frame included):
/// ClientToScreen(0,0) minus the window rect's top-left. Add nothing; SUBTRACT this from picture pixels to get client pixels.
pub fn client_origin_in_window(h: Hwnd) -> Result<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    if !is_alive(h) {
        return Err(LendError::Gone);
    }
    let mut p = POINT { x: 0, y: 0 };
    if !unsafe { ClientToScreen(hwnd(h), &mut p) }.as_bool() {
        return Err(LendError::Gone);
    }
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd(h), &mut r) }.map_err(os_err)?;
    Ok((p.x - r.left, p.y - r.top))
}
