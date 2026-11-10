//! Lend an App: border overlay for a lent window (slice S3).
//!
//! `Border::start` spawns one thread that owns two layered overlay windows:
//! a click-through FRAME around the target and a clickable TAB above it. The
//! thread also owns the WinEvent hooks that follow the target and the message
//! loop. Nothing here activates or focuses a window: overlays are created
//! hidden, shown with SW_SHOWNOACTIVATE, moved with SWP_NOACTIVATE, and answer
//! WM_MOUSEACTIVATE with MA_NOACTIVATE.
//!
//! Events: `Border::events()` returns the receiver. A click on the tab button
//! sends `TabClicked(state)` with the state shown at click time; the border
//! never changes its own state, the session decides. `UserFocused` fires when
//! the target's process becomes foreground, `Foregrounded` when the target window itself does, `Minimized` when
//! it is minimized (the watch runs in every state, Hidden included). `TargetGone` fires when the target
//! is destroyed; the thread then exits.
//!
//! Commands (`set_state`, `set_label`, `stop`) go over an mpsc channel and wake
//! the thread with a posted message. `stop` also runs on drop.


use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    BLENDFUNCTION, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, GetMonitorInfoW, HBITMAP, HDC,
    HFONT, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, GetClassNameW, GetForegroundWindow, GetMessageW,
    GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindow, MA_NOACTIVATE,
    MSG, OBJID_WINDOW, PostQuitMessage, PostThreadMessageW, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
    KillTimer, SetTimer, TranslateMessage, ULW_ALPHA, WM_TIMER, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WINDOW_EX_STYLE,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::{Hwnd, LendError, Rect, Result};

/// What the tab and frame show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The bot is using the app. Light frame, "Take control" button.
    Bot,
    /// The person has control. Dashed frame, "Hand back" button.
    You,
    /// The bot needs the person. Orange frame, "I'm on it" button.
    NeedsYou,
    /// Draw nothing.
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderEvent {
    /// The tab button was clicked; carries the state shown at click time.
    TabClicked(State),
    /// The target's process became the foreground window.
    UserFocused,
    /// The target itself (this exact window) became the foreground window.
    Foregrounded,
    /// The target was minimized.
    Minimized,
    /// The target window was destroyed. The border has stopped.
    TargetGone,
}

enum Cmd {
    SetState(State),
    #[allow(dead_code)]
    SetLabel(String),
    Stop,
}

/// Frame margin around the target, in px. The stroke sits on the target edge.
const M: i32 = 4;
const TAB_W: i32 = 300;
const TAB_H: i32 = 26;
const BTN_W: i32 = 84;
const BTN_H: i32 = 20;
const BTN_X: i32 = TAB_W - 4 - BTN_W;
const BTN_Y: i32 = (TAB_H - BTN_H) / 2;

const WM_APP_WAKE: u32 = WM_APP + 1;

/// A running border overlay. Dropping it stops the thread and waits for it.
pub struct Border {
    tid: u32,
    cmds: Sender<Cmd>,
    events: Receiver<BorderEvent>,
    join: Mutex<Option<JoinHandle<()>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    frame_hwnd: Arc<AtomicIsize>,
}

impl Border {
    /// Start an overlay around `target`. `label` is the app name shown in the
    /// tab ("<label> · used by your bot"). The overlays start hidden-to-focus
    /// safe: no activation happens here, and the initial state is `Bot`.
    pub fn start(target: Hwnd, label: String) -> Result<Border> {
        let t = hwnd(target);
        if !unsafe { IsWindow(Some(t)) }.as_bool() {
            return Err(LendError::Gone);
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(t, Some(&mut pid)) };
        if pid == 0 {
            return Err(LendError::Os(format!("no pid for window {target:#x}")));
        }
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32>>(1);
        let frame_hwnd = Arc::new(AtomicIsize::new(0));
        let frame_out = frame_hwnd.clone();
        let handle = std::thread::Builder::new()
            .name("lend-border".into())
            .spawn(move || {
                if let Err(e) = border_thread(target, pid, label, cmd_rx, ev_tx, &ready_tx, &frame_out) {
                    let _ = ready_tx.try_send(Err(e));
                }
            })
            .map_err(|e| LendError::Os(format!("spawn border thread: {e}")))?;
        match ready_rx.recv() {
            Ok(Ok(tid)) => Ok(Border {
                tid,
                cmds: cmd_tx,
                events: ev_rx,
                join: Mutex::new(Some(handle)),
                frame_hwnd,
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err(LendError::Os("border thread exited during start".into())),
        }
    }

    pub fn set_state(&self, state: State) {
        self.send(Cmd::SetState(state));
    }

    #[allow(dead_code)] // the label is the app name set at start
    pub fn set_label(&self, label: String) {
        self.send(Cmd::SetLabel(label));
    }

    /// Events from the overlay thread. Read them with `try_recv`/`recv`.
    /// Test only: the frame overlay window handle.
    #[cfg(test)]
    pub(crate) fn frame_hwnd(&self) -> Hwnd {
        self.frame_hwnd.load(Ordering::SeqCst)
    }

    pub fn events(&self) -> &Receiver<BorderEvent> {
        &self.events
    }

    /// Ask the thread to stop. Safe to call more than once.
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }

    fn send(&self, cmd: Cmd) {
        if self.cmds.send(cmd).is_ok() {
            unsafe {
                let _ = PostThreadMessageW(self.tid, WM_APP_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl Drop for Border {
    fn drop(&mut self) {
        self.stop();
        if let Some(h) = self.join.get_mut().ok().and_then(|j| j.take()) {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Pure geometry, text and rules (unit tested).
// ---------------------------------------------------------------------------

/// Frame window rect: the target grown by `m` on every side.
pub(crate) fn frame_rect(target: Rect, m: i32) -> Rect {
    Rect {
        x: target.x - m,
        y: target.y - m,
        w: target.w + 2 * m,
        h: target.h + 2 * m,
    }
}

/// Tab top-left: centred above the target. If there is no room above on the
/// monitor, it moves inside the target's top edge. Kept inside the monitor's
/// width.
pub(crate) fn tab_pos(target: Rect, mon: Rect) -> (i32, i32) {
    let mut tx = target.x + target.w / 2 - TAB_W / 2;
    let mut ty = target.y - TAB_H - 4;
    if ty < mon.y {
        ty = target.y + 6;
    }
    let lo = mon.x + 4;
    let hi = (mon.x + mon.w - TAB_W - 4).max(lo);
    tx = tx.clamp(lo, hi);
    (tx, ty)
}

/// Dash pattern for the dashed frame: 6 px on, 4 px off, along the perimeter.
pub(crate) fn dash_on(t: i32) -> bool {
    t.rem_euclid(10) < 6
}

/// Perimeter position for the dash pattern, clockwise from the top-left.
fn perimeter_t(x: i32, y: i32, w: i32, h: i32, d: i32) -> i32 {
    if y == d {
        x
    } else if w - 1 - x == d {
        w + y
    } else if h - 1 - y == d {
        w + h + (w - 1 - x)
    } else {
        2 * w + h + (h - 1 - y)
    }
}

/// Tab label and button text for a state. Hidden draws nothing.
pub(crate) fn tab_text(state: State, label: &str) -> (String, &'static str) {
    match state {
        State::Bot => (format!("{label} · used by your bot"), "Take control"),
        State::You => ("You have control".to_string(), "Hand back"),
        State::NeedsYou => ("Your bot needs you".to_string(), "I'm on it"),
        State::Hidden => (String::new(), ""),
    }
}

/// Overlays show unless minimised, covered by another full-screen window, or
/// hidden by state.
pub(crate) fn should_show(minimized: bool, fs_covered: bool, state: State) -> bool {
    !minimized && !fs_covered && state != State::Hidden
}

/// Premultiplied ARGB pixel for colour (r,g,b) at alpha a.
fn argb(r: u32, g: u32, b: u32, a: u32) -> u32 {
    let p = |c: u32| c * a / 255;
    (a << 24) | (p(r) << 16) | (p(g) << 8) | p(b)
}

/// Pixel for the frame window. `d` is the distance to the nearest window edge.
/// The target edge sits at d == M, so the 2 px light stroke covers d in
/// {M-1, M}. For Bot, a 1 px dark outline sits at d == M-2, outside the stroke,
/// so the frame stays visible on dark backgrounds.
fn frame_pixel(state: State, x: i32, y: i32, d: i32, w: i32, h: i32) -> u32 {
    let on_stroke = d == M - 1 || d == M;
    match state {
        State::Bot => {
            if on_stroke {
                argb(242, 242, 240, 255)
            } else if d == M - 2 {
                argb(20, 20, 20, 200)
            } else if d < M - 2 {
                argb(242, 242, 240, 40)
            } else {
                0
            }
        }
        State::You => {
            let t = perimeter_t(x, y, w, h, d);
            if on_stroke && dash_on(t) {
                argb(154, 154, 150, 255)
            } else {
                0
            }
        }
        State::NeedsYou => {
            if on_stroke {
                argb(224, 178, 82, 255)
            } else {
                0
            }
        }
        State::Hidden => 0,
    }
}

// ---------------------------------------------------------------------------
// Win32 helpers. Everything below runs on the overlay thread only.
// ---------------------------------------------------------------------------

thread_local! {
    /// Shared state for the overlay thread. Hooks and the window procedure
    /// run on this thread, so they find it here.
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn ctx() -> Option<Rc<Ctx>> {
    CTX.with(|c| c.borrow().clone())
}

struct Ctx {
    target: isize,
    target_pid: u32,
    frame: isize,
    tab: isize,
    events: Sender<BorderEvent>,
    state: Cell<State>,
    label: RefCell<String>,
    minimized: Cell<bool>,
    /// Target rect at the last reposition; the timer compares against it.
    last_rect: Cell<Option<Rect>>,
    fs_covered: Cell<bool>,
    visible: Cell<bool>,
    /// Frame DIB, rebuilt only when the size changes.
    frame_dib: RefCell<Option<Dib>>,
    frame_painted: Cell<Option<State>>,
    tab_dib: RefCell<Option<Dib>>,
    tab_painted: RefCell<Option<(State, String)>>,
}

fn raw(h: HWND) -> isize {
    h.0 as isize
}

fn hwnd(v: isize) -> HWND {
    HWND(v as *mut c_void)
}

fn os_err(what: &str, e: impl std::fmt::Display) -> LendError {
    LendError::Os(format!("{what}: {e}"))
}

fn rect_of(r: RECT) -> Rect {
    Rect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    }
}

/// Visible frame rect (DWM extended bounds, no invisible resize borders).
fn visible_rect(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(rect_of(r))
}

fn monitor_rect(h: HWND) -> Rect {
    unsafe {
        let m = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(m, &mut mi);
        rect_of(mi.rcMonitor)
    }
}

/// A 32-bit top-down DIB selected into a memory DC.
struct Dib {
    hdc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Dib {
    fn new(w: i32, h: i32) -> Result<Dib> {
        unsafe {
            let hdc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = null_mut();
            let bmp = match CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(b) => b,
                Err(e) => {
                    let _ = DeleteDC(hdc);
                    return Err(os_err("CreateDIBSection", e));
                }
            };
            if bits.is_null() {
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                let _ = DeleteDC(hdc);
                return Err(LendError::Os("CreateDIBSection returned no bits".into()));
            }
            let old = SelectObject(hdc, HGDIOBJ(bmp.0));
            Ok(Dib {
                hdc,
                bmp,
                old,
                bits: bits as *mut u32,
                w,
                h,
            })
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.hdc);
        }
    }
}

/// Push a DIB to a layered window at (x,y). Topmost, never activates.
fn present(h: HWND, dib: &Dib, x: i32, y: i32) -> Result<()> {
    let size = SIZE { cx: dib.w, cy: dib.h };
    let src = POINT { x: 0, y: 0 };
    let dst = POINT { x, y };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        SetWindowPos(h, Some(HWND_TOPMOST), x, y, dib.w, dib.h, SWP_NOACTIVATE)
            .map_err(|e| os_err("SetWindowPos", e))?;
        UpdateLayeredWindow(
            h,
            None,
            Some(&dst),
            Some(&size),
            Some(dib.hdc),
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
        .map_err(|e| os_err("UpdateLayeredWindow", e))?;
    }
    Ok(())
}

/// Repaint the frame only when its size or state changed. Moves reuse the DIB.
fn paint_frame(c: &Ctx, fr: Rect) -> Result<()> {
    let state = c.state.get();
    let mut slot = c.frame_dib.borrow_mut();
    let fits = matches!(slot.as_ref(), Some(d) if d.w == fr.w && d.h == fr.h);
    if !fits {
        *slot = Some(Dib::new(fr.w, fr.h)?);
        c.frame_painted.set(None);
    }
    let Some(dib) = slot.as_mut() else {
        return Ok(());
    };
    if c.frame_painted.get() != Some(state) {
        let (w, h) = (dib.w, dib.h);
        let px = dib.pixels();
        for yy in 0..h {
            for xx in 0..w {
                let d = xx.min(yy).min(w - 1 - xx).min(h - 1 - yy);
                px[(yy * w + xx) as usize] = frame_pixel(state, xx, yy, d, w, h);
            }
        }
        c.frame_painted.set(Some(state));
    }
    present(hwnd(c.frame), dib, fr.x, fr.y)
}

/// Fill a rounded rect inside `px` (stride `stride`). Optional 1 px border.
#[allow(clippy::too_many_arguments)]
fn fill_round(
    px: &mut [u32],
    stride: i32,
    rx: i32,
    ry: i32,
    rw: i32,
    rh: i32,
    radius: f32,
    fill: u32,
    border: Option<u32>,
) {
    for y in ry..ry + rh {
        for x in rx..rx + rw {
            let (px_c, py_c) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = px_c.clamp(rx as f32 + radius, (rx + rw) as f32 - radius);
            let cy = py_c.clamp(ry as f32 + radius, (ry + rh) as f32 - radius);
            let dist = ((px_c - cx).powi(2) + (py_c - cy).powi(2)).sqrt();
            if dist > radius {
                continue;
            }
            let c = match border {
                Some(b) if dist > radius - 1.0 => b,
                _ => fill,
            };
            px[(y * stride + x) as usize] = c;
        }
    }
}

fn make_font() -> HFONT {
    unsafe {
        CreateFontW(
            -12,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

/// GDI text into the tab DIB. Callers fix alpha afterwards.
fn draw_label(hdc: HDC, label: &str, button: &str) {
    let font = make_font();
    unsafe {
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        SetBkMode(hdc, TRANSPARENT);

        SetTextColor(hdc, COLORREF(0x00FF_FFFF));
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut tr = RECT {
            left: 12,
            top: 0,
            right: BTN_X - 8,
            bottom: TAB_H,
        };
        DrawTextW(
            hdc,
            &mut text,
            &mut tr,
            DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0 | DT_NOPREFIX.0 | DT_END_ELLIPSIS.0),
        );

        SetTextColor(hdc, COLORREF(0x0015_1515));
        let mut btn: Vec<u16> = button.encode_utf16().collect();
        let mut br = RECT {
            left: BTN_X,
            top: BTN_Y,
            right: BTN_X + BTN_W,
            bottom: BTN_Y + BTN_H,
        };
        DrawTextW(
            hdc,
            &mut btn,
            &mut br,
            DRAW_TEXT_FORMAT(DT_SINGLELINE.0 | DT_VCENTER.0 | DT_NOPREFIX.0 | DT_CENTER.0),
        );

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

fn render_tab(dib: &mut Dib, state: State, label: &str) {
    let (text, button) = tab_text(state, label);
    {
        let px = dib.pixels();
        px.fill(0);
        fill_round(
            px,
            TAB_W,
            0,
            0,
            TAB_W,
            TAB_H,
            TAB_H as f32 / 2.0,
            argb(21, 21, 21, 235),
            Some(argb(68, 68, 68, 255)),
        );
        fill_round(
            px,
            TAB_W,
            BTN_X,
            BTN_Y,
            BTN_W,
            BTN_H,
            BTN_H as f32 / 2.0,
            argb(238, 238, 238, 255),
            None,
        );
    }
    // Remember the background, let GDI draw text, then make changed pixels opaque.
    let before: Vec<u32> = dib.pixels().to_vec();
    draw_label(dib.hdc, &text, button);
    let px = dib.pixels();
    for (i, p) in px.iter_mut().enumerate() {
        if *p & 0x00FF_FFFF != before[i] & 0x00FF_FFFF {
            *p = 0xFF00_0000 | (*p & 0x00FF_FFFF);
        } else {
            *p = before[i];
        }
    }
}

/// Repaint the tab only when its state or label changed.
fn paint_tab(c: &Ctx, x: i32, y: i32) -> Result<()> {
    let state = c.state.get();
    let label = c.label.borrow().clone();
    let mut slot = c.tab_dib.borrow_mut();
    if slot.is_none() {
        *slot = Some(Dib::new(TAB_W, TAB_H)?);
        *c.tab_painted.borrow_mut() = None;
    }
    let Some(dib) = slot.as_mut() else {
        return Ok(());
    };
    let key = (state, label.clone());
    if c.tab_painted.borrow().as_ref() != Some(&key) {
        render_tab(dib, state, &label);
        *c.tab_painted.borrow_mut() = Some(key);
    }
    present(hwnd(c.tab), dib, x, y)
}

/// Lay out both overlays around the target's current visible rect.
fn reposition(c: &Ctx) {
    let state = c.state.get();
    if state != State::Hidden && !c.minimized.get() {
        let t = hwnd(c.target);
        if let Some(r) = visible_rect(t) {
            c.last_rect.set(Some(r));
            let _ = paint_frame(c, frame_rect(r, M));
            let (tx, ty) = tab_pos(r, monitor_rect(t));
            let _ = paint_tab(c, tx, ty);
        }
    }
    update_visibility(c);
}

/// Safety net, also run from the hook: follow minimise/restore, a moved or
/// resized target, and a destroyed target. Returns false when the target is gone.
fn sync_target(c: &Ctx) -> bool {
    let t = hwnd(c.target);
    if !unsafe { IsWindow(Some(t)) }.as_bool() {
        let _ = c.events.send(BorderEvent::TargetGone);
        unsafe { PostQuitMessage(0) };
        return false;
    }
    let iconic = unsafe { IsIconic(t) }.as_bool();
    if iconic != c.minimized.get() {
        c.minimized.set(iconic);
        if iconic {
            let _ = c.events.send(BorderEvent::Minimized);
        }
        reposition(c);
        return true;
    }
    if !iconic && visible_rect(t) != c.last_rect.get() {
        reposition(c);
    }
    true
}

/// Show or hide both overlays together. Shows only without activating.
fn update_visibility(c: &Ctx) {
    let show = should_show(c.minimized.get(), c.fs_covered.get(), c.state.get());
    if c.visible.get() == show {
        return;
    }
    c.visible.set(show);
    let cmd = if show { SW_SHOWNOACTIVATE } else { SW_HIDE };
    unsafe {
        let _ = ShowWindow(hwnd(c.frame), cmd);
        let _ = ShowWindow(hwnd(c.tab), cmd);
    }
}

/// Hide while a non-desktop window other than the target covers the target's monitor.
fn refresh_fullscreen(c: &Ctx) {
    let fg = unsafe { GetForegroundWindow() };
    let covered = if fg.0.is_null()
        || raw(fg) == c.target
        || raw(fg) == c.frame
        || raw(fg) == c.tab
    {
        false
    } else {
        let mut class = [0u16; 64];
        let n = unsafe { GetClassNameW(fg, &mut class) }.max(0) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if class == "Progman" || class == "WorkerW" {
            false
        } else {
            let mut rc = RECT::default();
            let got = unsafe { GetWindowRect(fg, &mut rc) }.is_ok();
            let m = monitor_rect(hwnd(c.target));
            got && rc.left <= m.x
                && rc.top <= m.y
                && rc.right >= m.x + m.w
                && rc.bottom >= m.y + m.h
        }
    };
    c.fs_covered.set(covered);
    update_visibility(c);
}

fn in_button(x: i32, y: i32) -> bool {
    x >= BTN_X && x < BTN_X + BTN_W && y >= BTN_Y && y < BTN_Y + BTN_H
}

fn point_from_lparam(lp: LPARAM) -> (i32, i32) {
    let v = lp.0 as i64;
    let x = (v & 0xFFFF) as i16 as i32;
    let y = ((v >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONUP => {
                if let Some(c) = ctx() {
                    if raw(h) == c.tab {
                        let (x, y) = point_from_lparam(lp);
                        if in_button(x, y) {
                            let _ = c.events.send(BorderEvent::TabClicked(c.state.get()));
                        }
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(h, msg, wp, lp)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

/// Target-window events, filtered to the target's pid by the hook.
unsafe extern "system" fn tracking_hook(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd_: HWND,
    id_obj: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    unsafe {
        let Some(c) = ctx() else {
            return;
        };
        // Location changes are accepted for any object id on the target: Chromium
        // apps report moves with other ids. Reposition is cheap and the timer
        // dedups. Everything else needs the window object itself.
        if raw(hwnd_) != c.target {
            return;
        }
        if event != EVENT_OBJECT_LOCATIONCHANGE && id_obj != OBJID_WINDOW.0 {
            return;
        }
        match event {
            EVENT_OBJECT_LOCATIONCHANGE => {
                sync_target(&c);
            }
            EVENT_SYSTEM_MINIMIZESTART => {
                c.minimized.set(true);
                let _ = c.events.send(BorderEvent::Minimized);
                update_visibility(&c);
            }
            EVENT_SYSTEM_MINIMIZEEND => {
                c.minimized.set(false);
                reposition(&c);
            }
            EVENT_OBJECT_DESTROY => {
                let _ = c.events.send(BorderEvent::TargetGone);
                PostQuitMessage(0);
            }
            _ => {}
        }
    }
}

/// Global foreground changes: reports the target's process becoming foreground.
unsafe extern "system" fn foreground_hook(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd_: HWND,
    _id_obj: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    unsafe {
        if event != EVENT_SYSTEM_FOREGROUND {
            return;
        }
        let Some(c) = ctx() else {
            return;
        };
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd_, Some(&mut pid));
        if pid != 0 && pid == c.target_pid {
            let _ = c.events.send(BorderEvent::UserFocused);
        }
        if raw(hwnd_) == c.target {
            let _ = c.events.send(BorderEvent::Foregrounded);
        }
        refresh_fullscreen(&c);
    }
}

fn register_class(name: PCWSTR, hinst: HINSTANCE) {
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        lpszClassName: name,
        ..Default::default()
    };
    // Fails harmlessly when another border on this process already registered it.
    let _ = unsafe { RegisterClassExW(&wc) };
}

fn create_overlay(class: PCWSTR, ex: WINDOW_EX_STYLE, hinst: HINSTANCE) -> Result<HWND> {
    // WS_POPUP without WS_VISIBLE: created hidden, shown later without activation.
    unsafe {
        CreateWindowExW(
            ex,
            class,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst),
            None,
        )
    }
    .map_err(|e| os_err("CreateWindowExW", e))
}

/// Owns the windows and hooks. Its Drop unhooks, destroys the windows, and
/// clears the thread-local context, whichever way the thread exits.
struct Teardown {
    frame: isize,
    tab: isize,
    hooks: Vec<HWINEVENTHOOK>,
}

impl Drop for Teardown {
    fn drop(&mut self) {
        unsafe {
            for h in self.hooks.drain(..) {
                let _ = UnhookWinEvent(h);
            }
            let _ = DestroyWindow(hwnd(self.frame));
            let _ = DestroyWindow(hwnd(self.tab));
        }
        CTX.with(|c| *c.borrow_mut() = None);
    }
}

fn border_thread(
    target: Hwnd,
    pid: u32,
    label: String,
    cmds: Receiver<Cmd>,
    events: Sender<BorderEvent>,
    ready: &SyncSender<Result<u32>>,
    frame_out: &AtomicIsize,
) -> Result<()> {
    let t = hwnd(target);
    let hinst: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| os_err("GetModuleHandleW", e))?
        .into();
    register_class(w!("LendBorderFrame"), hinst);
    register_class(w!("LendBorderTab"), hinst);

    let frame = create_overlay(
        w!("LendBorderFrame"),
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
        hinst,
    )?;
    let tab = create_overlay(
        w!("LendBorderTab"),
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        hinst,
    )?;
    frame_out.store(raw(frame), Ordering::SeqCst);
    let mut td = Teardown {
        frame: raw(frame),
        tab: raw(tab),
        hooks: Vec::new(),
    };
    for h in [frame, tab] {
        // Capture exclusion is best effort.
        let _ = unsafe { SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE) };
    }

    let c = Rc::new(Ctx {
        target,
        target_pid: pid,
        frame: raw(frame),
        tab: raw(tab),
        events,
        state: Cell::new(State::Bot),
        label: RefCell::new(label),
        minimized: Cell::new(unsafe { IsIconic(t) }.as_bool()),
        last_rect: Cell::new(None),
        fs_covered: Cell::new(false),
        visible: Cell::new(false),
        frame_dib: RefCell::new(None),
        frame_painted: Cell::new(None),
        tab_dib: RefCell::new(None),
        tab_painted: RefCell::new(None),
    });
    CTX.with(|x| *x.borrow_mut() = Some(c.clone()));

    let tid = unsafe { GetCurrentThreadId() };
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    // One range covers minimize start/end, destroy and location change; filtered by hwnd/id.
    let hook_target = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(tracking_hook),
            pid,
            0,
            flags,
        )
    };
    if hook_target.is_invalid() {
        return Err(LendError::Os("SetWinEventHook (target) failed".into()));
    }
    td.hooks.push(hook_target);
    let hook_fg = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_hook),
            0,
            0,
            flags,
        )
    };
    if hook_fg.is_invalid() {
        return Err(LendError::Os("SetWinEventHook (foreground) failed".into()));
    }
    td.hooks.push(hook_fg);

    refresh_fullscreen(&c);
    reposition(&c);
    // Thread timer (no window): ~100 ms poll as a safety net for missed hook events.
    let timer = unsafe { SetTimer(None, 0, 100, None) };
    let _ = ready.try_send(Ok(tid));

    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 <= 0 {
            break;
        }
        if msg.hwnd.0.is_null() && msg.message == WM_TIMER {
            if sync_target(&c) {
                refresh_fullscreen(&c);
            }
            continue;
        }
        if msg.hwnd.0.is_null() && msg.message == WM_APP_WAKE {
            while let Ok(cmd) = cmds.try_recv() {
                match cmd {
                    Cmd::SetState(s) => {
                        c.state.set(s);
                        reposition(&c);
                    }
                    Cmd::SetLabel(l) => {
                        *c.label.borrow_mut() = l;
                        reposition(&c);
                    }
                    Cmd::Stop => unsafe { PostQuitMessage(0) },
                }
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if timer != 0 {
        let _ = unsafe { KillTimer(None, timer) };
    }
    drop(td);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn frame_rect_surrounds_target() {
        assert_eq!(
            frame_rect(rect(100, 50, 800, 600), M),
            rect(96, 46, 808, 608)
        );
    }

    #[test]
    fn tab_sits_above_target_when_there_is_room() {
        let mon = rect(0, 0, 1920, 1080);
        // Centred: 100 + 400 - 150 = 350. Above: 50 - 26 - 4 = 20.
        assert_eq!(tab_pos(rect(100, 50, 800, 600), mon), (350, 20));
    }

    #[test]
    fn tab_moves_inside_target_when_no_room_above() {
        let mon = rect(0, 0, 1920, 1080);
        // Above would be 10 - 30 = -20, which is off the monitor: inside at y + 6.
        assert_eq!(tab_pos(rect(100, 10, 800, 600), mon), (350, 16));
    }

    #[test]
    fn tab_is_clamped_to_monitor_width() {
        let mon = rect(0, 0, 1920, 1080);
        assert_eq!(tab_pos(rect(0, 100, 100, 100), mon), (4, 70));
    }

    #[test]
    fn dash_pattern_is_six_on_four_off() {
        for t in 0..6 {
            assert!(dash_on(t), "{t} should be on");
        }
        for t in 6..10 {
            assert!(!dash_on(t), "{t} should be off");
        }
        assert!(dash_on(10));
        assert!(!dash_on(-1));
    }

    #[test]
    fn tab_text_per_state() {
        assert_eq!(
            tab_text(State::Bot, "LINE"),
            ("LINE · used by your bot".to_string(), "Take control")
        );
        assert_eq!(
            tab_text(State::You, "LINE"),
            ("You have control".to_string(), "Hand back")
        );
        assert_eq!(
            tab_text(State::NeedsYou, "LINE"),
            ("Your bot needs you".to_string(), "I'm on it")
        );
        assert_eq!(tab_text(State::Hidden, "LINE").0, "");
    }

    #[test]
    fn bot_frame_has_dark_outline_outside_light_stroke() {
        let (w, h) = (100, 100);
        // d = M-1 and M are the light stroke; d = M-2 is the dark outline.
        assert_eq!(frame_pixel(State::Bot, 0, 0, M - 1, w, h), argb(242, 242, 240, 255));
        assert_eq!(frame_pixel(State::Bot, 0, 0, M, w, h), argb(242, 242, 240, 255));
        assert_eq!(frame_pixel(State::Bot, 0, 0, M - 2, w, h), argb(20, 20, 20, 200));
        assert_eq!(frame_pixel(State::Bot, 0, 0, M + 1, w, h), 0);
    }

    #[test]
    fn hidden_draws_nothing() {
        assert_eq!(frame_pixel(State::Hidden, 0, 0, M, 100, 100), 0);
    }

    #[test]
    fn visibility_rules() {
        assert!(should_show(false, false, State::Bot));
        assert!(should_show(false, false, State::NeedsYou));
        assert!(!should_show(true, false, State::Bot));
        assert!(!should_show(false, true, State::You));
        assert!(!should_show(false, false, State::Hidden));
    }
}
