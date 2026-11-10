//! Lend an App: UI Automation snapshot of a lent window.
//!
//! `snapshot` reads the control view (one FindAllBuildCache call) of one window and returns the elements a bot can
//! act on, each with a short ref ("e1", "e2", ...) and a rect in the window's client
//! coordinates. The walk runs on its own MTA thread so a hung app cannot hang the caller.
//! Chromium-based apps may expose little or nothing; that yields an empty list.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{Interface, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationCacheRequest, IUIAutomationElement,
    IUIAutomationTextPattern, IUIAutomationValuePattern, TreeScope_Subtree,
    UIA_BoundingRectanglePropertyId, UIA_ControlTypePropertyId, UIA_IsEnabledPropertyId,
    UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId, UIA_IsPasswordPropertyId,
    UIA_NamePropertyId, UIA_NativeWindowHandlePropertyId, UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetWindowLongPtrW, SendMessageTimeoutW, GWL_STYLE, OBJID_CLIENT,
    SMTO_ABORTIFHUNG, WM_GETOBJECT,
};

use super::{is_chromium_class, win, Hwnd, LendError, Rect, Result, FEW_ELEMENTS};
use crate::potato::lend_tools::is_password_field;

/// Most characters read from a document through the TextPattern.
const MAX_TEXT_CHARS: i32 = 4000;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Element {
    /// "e1", "e2", ... in walk order.
    pub r#ref: String,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    /// Client coordinates of the window the snapshot was taken from.
    pub rect: Rect,
    pub enabled: bool,
    pub focusable: bool,
    /// A password box (by name, UIA `IsPassword`, or the Win32 `ES_PASSWORD` style). Its value is never read.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub password: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Snapshot {
    pub id: String,
    pub elements: Vec<Element>,
    pub truncated: bool,
}

/// One UIA node as read from the cache, before filtering. Rect is in screen coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawNode {
    pub control_type: i32,
    pub name: String,
    pub value: Option<String>,
    pub screen_rect: Rect,
    pub enabled: bool,
    pub focusable: bool,
    pub offscreen: bool,
    pub password: bool,
}

/// Short role name for a `UIA_*ControlTypeId` value; unknown ids map to "other".
pub(crate) fn role_for_control_type(control_type: i32) -> &'static str {
    match control_type {
        50000 => "button",
        50001 => "calendar",
        50002 => "checkbox",
        50003 => "combobox",
        50004 => "edit",
        50005 => "hyperlink",
        50006 => "image",
        50007 => "listitem",
        50008 => "list",
        50009 => "menu",
        50010 => "menubar",
        50011 => "menuitem",
        50012 => "progressbar",
        50013 => "radio",
        50014 => "scrollbar",
        50015 => "slider",
        50016 => "spinner",
        50017 => "statusbar",
        50018 => "tab",
        50019 => "tabitem",
        50020 => "text",
        50021 => "toolbar",
        50022 => "tooltip",
        50023 => "tree",
        50024 => "treeitem",
        50025 => "custom",
        50026 => "group",
        50027 => "thumb",
        50028 => "datagrid",
        50029 => "dataitem",
        50030 => "document",
        50031 => "splitbutton",
        50032 => "window",
        50033 => "pane",
        50034 => "header",
        50035 => "headeritem",
        50036 => "table",
        50037 => "titlebar",
        50038 => "separator",
        50039 => "semanticzoom",
        50040 => "appbar",
        _ => "other",
    }
}

fn is_interactive(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "edit"
            | "checkbox"
            | "radio"
            | "combobox"
            | "listitem"
            | "menuitem"
            | "tabitem"
            | "hyperlink"
            | "slider"
            | "treeitem"
    )
}

/// Keep a node that has visible size, is on screen, and either has a name or value
/// or is an interactive control.
fn keep(node: &RawNode, role: &str) -> bool {
    if node.offscreen || node.screen_rect.w <= 0 || node.screen_rect.h <= 0 {
        return false;
    }
    let has_name = !node.name.trim().is_empty();
    let has_value = node.value.as_deref().is_some_and(|v| !v.is_empty());
    has_name || has_value || is_interactive(role)
}

/// Turn raw nodes into elements. `offset` is the screen-to-client offset of the window
/// (the result of ScreenToClient on screen point (0, 0)). Refs count only kept elements.
pub(crate) fn build_elements(raw: &[RawNode], offset: (i32, i32)) -> Vec<Element> {
    let mut out = Vec::new();
    for node in raw {
        let role = role_for_control_type(node.control_type);
        if !keep(node, role) {
            continue;
        }
        let r = &node.screen_rect;
        out.push(Element {
            r#ref: format!("e{}", out.len() + 1),
            role: role.to_string(),
            name: node.name.clone(),
            // A password box never shows its value, whatever the walk read.
            value: node.value.clone().filter(|v| !v.is_empty() && !node.password),
            rect: Rect { x: r.x + offset.0, y: r.y + offset.1, w: r.w, h: r.h },
            enabled: node.enabled,
            focusable: node.focusable,
            password: node.password,
        });
    }
    out
}

/// Look up an element by its ref in a snapshot.
pub fn find<'a>(s: &'a Snapshot, r: &str) -> Option<&'a Element> {
    s.elements.iter().find(|e| e.r#ref == r)
}

fn new_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{:x}{:x}", nanos & 0xffff_ffff, seq & 0xffff)
}

/// Shared between the walk thread and the caller. The caller reads whatever was
/// collected when the timeout hits.
#[derive(Default)]
struct Collected {
    nodes: Vec<RawNode>,
    truncated: bool,
}

/// Screen-to-client offset for a window: ScreenToClient of screen point (0, 0).
fn client_offset(hwnd: HWND) -> Result<(i32, i32)> {
    let mut p = POINT { x: 0, y: 0 };
    let ok = unsafe { ScreenToClient(hwnd, &mut p) };
    if !ok.as_bool() {
        return Err(LendError::Gone);
    }
    Ok((p.x, p.y))
}

/// Take a snapshot of the window's control tree. Returns `truncated: true` when the walk
/// hit `max_nodes`, the depth cap, or the timeout. On timeout with nothing collected,
/// returns `Err(Os("timeout"))`.
pub fn snapshot(h: Hwnd, max_nodes: usize, timeout_ms: u64) -> Result<Snapshot> {
    let hwnd = HWND(h as *mut c_void);
    let offset = client_offset(hwnd)?;

    let shared = Arc::new(Mutex::new(Collected::default()));
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let (tx, rx) = mpsc::channel::<std::result::Result<(), String>>();
    let thread_shared = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("lend-a11y".into())
        .spawn(move || {
            let outcome = run_walk(h, max_nodes, deadline, &thread_shared);
            let _ = tx.send(outcome);
        })
        .map_err(|e| LendError::Os(e.to_string()))?;

    let timed_out = match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(())) => false,
        Ok(Err(e)) => return Err(LendError::Os(e)),
        Err(mpsc::RecvTimeoutError::Timeout) => true,
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(LendError::Os("accessibility thread ended".into()))
        }
    };

    let collected = shared.lock().map_err(|_| LendError::Os("lock poisoned".into()))?;
    if timed_out && collected.nodes.is_empty() {
        return Err(LendError::Os("timeout".into()));
    }
    Ok(Snapshot {
        id: new_id(),
        elements: build_elements(&collected.nodes, offset),
        truncated: collected.truncated || timed_out,
    })
}

/// Runs on the walk thread: COM init, UIA setup, then the walk. Errors are text only.
fn run_walk(
    h: Hwnd,
    max_nodes: usize,
    deadline: Instant,
    shared: &Mutex<Collected>,
) -> std::result::Result<(), String> {
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init.is_err() {
        return Err(format!("CoInitializeEx failed: {init:?}"));
    }
    let outcome = walk_with_uia(h, max_nodes, deadline, shared).map_err(|e| format!("{e}"));
    unsafe { CoUninitialize() };
    outcome
}

fn walk_with_uia(
    h: Hwnd,
    max_nodes: usize,
    deadline: Instant,
    shared: &Mutex<Collected>,
) -> windows::core::Result<()> {
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
    let cache: IUIAutomationCacheRequest = unsafe { automation.CreateCacheRequest()? };
    unsafe {
        cache.AddProperty(UIA_NamePropertyId)?;
        cache.AddProperty(UIA_ControlTypePropertyId)?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        cache.AddProperty(UIA_IsEnabledPropertyId)?;
        cache.AddProperty(UIA_IsKeyboardFocusablePropertyId)?;
        cache.AddProperty(UIA_IsOffscreenPropertyId)?;
        cache.AddProperty(UIA_IsPasswordPropertyId)?;
        cache.AddProperty(UIA_NativeWindowHandlePropertyId)?;
        cache.AddPattern(UIA_ValuePatternId)?;
        cache.AddPattern(UIA_TextPatternId)?;
    }
    let root = unsafe {
        automation.ElementFromHandleBuildCache(HWND(h as *mut c_void), &cache)?
    };
    // ONE cross-process call fetches the whole control-view subtree with every property cached, in document
    // order. The old per-node TreeWalker made several round trips per element, which is what made this slow.
    let condition = unsafe { automation.ControlViewCondition()? };
    let found = unsafe { root.FindAllBuildCache(TreeScope_Subtree, &condition, &cache)? };
    let total = unsafe { found.Length()? }.max(0) as usize;
    let mut nodes = Vec::new();
    if let Some(node) = read_raw(&root) {
        nodes.push(node);
    }
    let mut truncated = total + 1 > max_nodes;
    for i in 0..total.min(max_nodes.saturating_sub(1)) {
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let Ok(el) = (unsafe { found.GetElement(i as i32) }) else { continue };
        if let Some(node) = read_raw(&el) {
            nodes.push(node);
        }
    }
    if let Ok(mut c) = shared.lock() {
        c.nodes = nodes;
        c.truncated = truncated;
    }
    Ok(())
}

/// Read one element from its cache. None when the element lacks the basics.
fn read_raw(el: &IUIAutomationElement) -> Option<RawNode> {
    let control_type = unsafe { el.CachedControlType() }.ok()?.0;
    let role = role_for_control_type(control_type);
    let rect = unsafe { el.CachedBoundingRectangle() }.ok()?;
    let name = unsafe { el.CachedName() }.map(|b| b.to_string()).unwrap_or_default();
    let enabled = unsafe { el.CachedIsEnabled() }.map(|b| b.as_bool()).unwrap_or(false);
    let focusable =
        unsafe { el.CachedIsKeyboardFocusable() }.map(|b| b.as_bool()).unwrap_or(false);
    let offscreen = unsafe { el.CachedIsOffscreen() }.map(|b| b.as_bool()).unwrap_or(true);
    let is_password_prop = unsafe { el.CachedIsPassword() }.map(|b| b.as_bool()).unwrap_or(false);
    let native = unsafe { el.CachedNativeWindowHandle() }.map(|h| h.0 as isize).unwrap_or(0);
    // The name rule only applies to edit controls ("Show password" on a button is not a password box).
    let name_for_rule = if role == "edit" { name.as_str() } else { "" };
    let password = is_password_field(name_for_rule, is_password_prop, native_has_es_password(native));
    // A password box's value is never even read.
    let value = if password {
        None
    } else {
        let value = cached_value(el);
        if needs_text_fallback(role, value.as_deref()) {
            text_value(el).or(value)
        } else {
            value
        }
    };
    Some(RawNode {
        control_type,
        name,
        value,
        screen_rect: Rect {
            x: rect.left,
            y: rect.top,
            w: rect.right - rect.left,
            h: rect.bottom - rect.top,
        },
        enabled,
        focusable,
        offscreen,
        password,
    })
}

/// Win32 `ES_PASSWORD` edit style.
const ES_PASSWORD: isize = 0x20;

/// Pure: is this window a standard "Edit" control with the `ES_PASSWORD` bit? (The bit means other things on other classes.)
pub(crate) fn style_has_es_password(class: &str, style: isize) -> bool {
    class.eq_ignore_ascii_case("Edit") && style & ES_PASSWORD != 0
}

/// The element's own window handle (0 when it has none) is an Edit control with `ES_PASSWORD`. Read-only.
fn native_has_es_password(native: isize) -> bool {
    if native == 0 || !win::is_alive(native) {
        return false;
    }
    let style = unsafe { GetWindowLongPtrW(win::hwnd(native), GWL_STYLE) };
    style_has_es_password(&win::class_name(native), style)
}

// ---- Chromium / Electron: ask the window for its accessibility tree ----------------------------------

const NUDGE_TIMEOUT_MS: u32 = 500;
const NUDGE_RETRY_WAIT: Duration = Duration::from_millis(300);
const RENDER_CLASS: &str = "Chrome_RenderWidgetHostHWND";

fn nudged() -> &'static Mutex<std::collections::HashSet<Hwnd>> {
    static SET: std::sync::OnceLock<Mutex<std::collections::HashSet<Hwnd>>> = std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

/// Send `WM_GETOBJECT(OBJID_CLIENT)` to one window, as a screen reader does. Never activates or focuses anything.
fn send_get_object(h: HWND) {
    let mut result = 0usize;
    unsafe {
        let _ = SendMessageTimeoutW(
            h,
            WM_GETOBJECT,
            WPARAM(0),
            LPARAM(OBJID_CLIENT.0 as isize),
            SMTO_ABORTIFHUNG,
            NUDGE_TIMEOUT_MS,
            Some(&mut result),
        );
    }
}

unsafe extern "system" fn find_render_child(child: HWND, lparam: LPARAM) -> BOOL {
    let found = unsafe { &mut *(lparam.0 as *mut Option<Hwnd>) };
    if win::class_name(child.0 as isize) == RENDER_CLASS {
        *found = Some(child.0 as isize);
        return BOOL(0);
    }
    BOOL(1)
}

/// For a Chromium or Electron window, once per window: send `WM_GETOBJECT` to it and to its
/// `Chrome_RenderWidgetHostHWND` child so it builds its accessibility tree. Other windows and repeats do nothing.
/// Returns true when the message was sent.
pub fn nudge_chromium(h: Hwnd) -> bool {
    if !win::is_alive(h) || !is_chromium_class(&win::class_name(h)) || !nudged().lock().unwrap_or_else(|e| e.into_inner()).insert(h) {
        return false;
    }
    send_get_object(win::hwnd(h));
    let mut found: Option<Hwnd> = None;
    unsafe {
        let _ = EnumChildWindows(Some(win::hwnd(h)), Some(find_render_child), LPARAM(&mut found as *mut Option<Hwnd> as isize));
    }
    if let Some(child) = found {
        send_get_object(win::hwnd(child));
    }
    true
}

/// `snapshot`, with the Chromium nudge first. When a Chromium window still shows fewer than `FEW_ELEMENTS`
/// elements, wait a moment and look once more (it builds its tree after the first ask).
pub fn snapshot_nudged(h: Hwnd, max_nodes: usize, timeout_ms: u64) -> Result<Snapshot> {
    nudge_chromium(h);
    let first = snapshot(h, max_nodes, timeout_ms)?;
    if first.elements.len() >= FEW_ELEMENTS || !is_chromium_class(&win::class_name(h)) {
        return Ok(first);
    }
    std::thread::sleep(NUDGE_RETRY_WAIT);
    match snapshot(h, max_nodes, timeout_ms) {
        Ok(second) if second.elements.len() >= first.elements.len() => Ok(second),
        _ => Ok(first),
    }
}

/// Documents and edit fields whose ValuePattern gave nothing may still expose their text
/// through the TextPattern (for example Notepad's "Text editor" document).
pub(crate) fn needs_text_fallback(role: &str, value: Option<&str>) -> bool {
    matches!(role, "document" | "edit") && value.is_none_or(str::is_empty)
}

/// Text of the element's whole document range, capped at `MAX_TEXT_CHARS`.
/// Any failure yields None so the caller keeps the value it had.
fn text_value(el: &IUIAutomationElement) -> Option<String> {
    let pattern = unsafe { el.GetCachedPattern(UIA_TextPatternId) }.ok()?;
    let text_pattern: IUIAutomationTextPattern = pattern.cast().ok()?;
    let range = unsafe { text_pattern.DocumentRange() }.ok()?;
    let text = unsafe { range.GetText(MAX_TEXT_CHARS) }.ok()?.to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn cached_value(el: &IUIAutomationElement) -> Option<String> {
    let pattern = unsafe { el.GetCachedPattern(UIA_ValuePatternId) }.ok()?;
    let value_pattern: IUIAutomationValuePattern = pattern.cast().ok()?;
    let value = unsafe { value_pattern.CachedValue() }.ok()?;
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(ct: i32, name: &str, value: Option<&str>, w: i32, h: i32, offscreen: bool) -> RawNode {
        RawNode {
            control_type: ct,
            name: name.to_string(),
            value: value.map(str::to_string),
            screen_rect: Rect { x: 100, y: 200, w, h },
            enabled: true,
            focusable: false,
            offscreen,
            password: false,
        }
    }

    #[test]
    fn maps_common_control_types() {
        assert_eq!(role_for_control_type(50000), "button");
        assert_eq!(role_for_control_type(50004), "edit");
        assert_eq!(role_for_control_type(50013), "radio");
        assert_eq!(role_for_control_type(50011), "menuitem");
        assert_eq!(role_for_control_type(50020), "text");
        assert_eq!(role_for_control_type(99999), "other");
        assert_eq!(role_for_control_type(-1), "other");
    }

    #[test]
    fn keeps_named_or_valued_or_interactive_only() {
        // Named text is kept.
        assert_eq!(build_elements(&[node(50020, "Hello", None, 10, 10, false)], (0, 0)).len(), 1);
        // Unnamed pane is skipped.
        assert!(build_elements(&[node(50033, "", None, 10, 10, false)], (0, 0)).is_empty());
        // Unnamed button is kept (interactive).
        assert_eq!(build_elements(&[node(50000, "", None, 10, 10, false)], (0, 0)).len(), 1);
        // Value-only pane is kept.
        assert_eq!(build_elements(&[node(50033, "", Some("typed"), 10, 10, false)], (0, 0)).len(), 1);
        // Whitespace-only name does not count.
        assert!(build_elements(&[node(50033, "   ", None, 10, 10, false)], (0, 0)).is_empty());
        // Zero-size and offscreen are skipped even when named or interactive.
        assert!(build_elements(&[node(50000, "OK", None, 0, 10, false)], (0, 0)).is_empty());
        assert!(build_elements(&[node(50000, "OK", None, 10, 0, false)], (0, 0)).is_empty());
        assert!(build_elements(&[node(50000, "OK", None, 10, 10, true)], (0, 0)).is_empty());
    }

    #[test]
    fn refs_are_sequential_over_kept_elements_only() {
        let raw = vec![
            node(50033, "", None, 10, 10, false),   // dropped
            node(50000, "Send", None, 10, 10, false), // e1
            node(50000, "", None, 0, 0, false),     // dropped (zero size)
            node(50020, "Hi", None, 10, 10, false), // e2
        ];
        let els = build_elements(&raw, (0, 0));
        let refs: Vec<&str> = els.iter().map(|e| e.r#ref.as_str()).collect();
        assert_eq!(refs, vec!["e1", "e2"]);
        assert_eq!(els[0].role, "button");
        assert_eq!(els[1].name, "Hi");
    }

    #[test]
    fn applies_client_offset_and_keeps_size() {
        let els = build_elements(&[node(50000, "Go", None, 40, 20, false)], (-8, -31));
        assert_eq!(els[0].rect, Rect { x: 92, y: 169, w: 40, h: 20 });
    }

    #[test]
    fn empty_value_becomes_none() {
        let els = build_elements(&[node(50000, "Go", Some(""), 10, 10, false)], (0, 0));
        assert_eq!(els[0].value, None);
    }

    #[test]
    fn text_fallback_only_for_empty_document_or_edit() {
        assert!(needs_text_fallback("document", None));
        assert!(needs_text_fallback("edit", Some("")));
        assert!(!needs_text_fallback("document", Some("typed")));
        assert!(!needs_text_fallback("button", None));
        assert!(!needs_text_fallback("text", Some("")));
    }

    #[test]
    fn password_node_never_shows_its_value() {
        let mut n = node(50004, "PIN", Some("1234"), 10, 10, false);
        n.password = true;
        let els = build_elements(&[n], (0, 0));
        assert!(els[0].password);
        assert_eq!(els[0].value, None);
        let json = serde_json::to_value(&els[0]).unwrap();
        assert_eq!(json["password"], true);
        let plain = build_elements(&[node(50004, "Name", Some("x"), 10, 10, false)], (0, 0));
        assert!(!plain[0].password);
        assert!(serde_json::to_value(&plain[0]).unwrap().get("password").is_none());
    }

    #[test]
    fn es_password_needs_the_edit_class() {
        assert!(style_has_es_password("Edit", 0x5001_0020));
        assert!(style_has_es_password("edit", 0x20));
        assert!(!style_has_es_password("Edit", 0x5001_0000));
        assert!(!style_has_es_password("Button", 0x20));
    }

    #[test]
    fn find_returns_matching_ref_or_none() {
        let els = build_elements(
            &[node(50000, "A", None, 10, 10, false), node(50000, "B", None, 10, 10, false)],
            (0, 0),
        );
        let snap = Snapshot { id: "x".into(), elements: els, truncated: false };
        assert_eq!(find(&snap, "e2").map(|e| e.name.as_str()), Some("B"));
        assert!(find(&snap, "e3").is_none());
    }
}
