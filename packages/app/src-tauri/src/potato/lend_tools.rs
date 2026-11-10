//! `app_observe` and `app_act`: the bot uses an app the person lent from their own Windows PC.
//!
//! They run here, in the app, through `lend::session::hub()`; they do not use the sandbox VM, so they work while it is stopped.
//! Checks, in order: Potato Bot on, the owner's turn, the Lend an App Labs switch, the arguments, the app's lease
//! (`LEASE_HELD_BY_PERSON` when the person has it), the day's cap. Input is only ever posted to the app's window.
//!
//! Coordinates. `app_observe` pictures are the whole window, frame included, as a JPEG whose long side is at most 1280 px (shrunk
//! when the window is bigger; `image_size` is the picture's size). `x`,`y` in `app_act` are pixels of that picture: they are scaled
//! back to window pixels with the size recorded at the last observe, then the input layer wants client pixels (inside the frame),
//! so the window's client origin is subtracted.
//! A `ref` already is in client pixels (the centre of the element's rectangle in the latest snapshot).
//!
//! The bot never types into a password box: a click on a password box (name says password, UIA `IsPassword`, or a Win32
//! `ES_PASSWORD` edit) marks the app, and `type` is refused until a different click or key. Its value is never listed.

use std::collections::HashMap;
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use super::lend::session::{hub, LentView, Phase};
use super::lend::LendError;
use super::sandbox_calls::{CallError, Calls, Context, Gates, Request, Tool, MAX_IMAGE_BYTES};

/// Most elements an `app_observe` answer lists.
pub const MAX_ELEMENTS: usize = 150;
/// Most characters of one element name or value in an answer.
const MAX_TEXT_CHARS: usize = 120;
/// Most characters the bot may type in one call.
pub const MAX_TYPE_CHARS: usize = 4000;
const NOTCH: i32 = 120;
const MAX_NOTCHES: f64 = 20.0;
const MAX_COORD: f64 = 16_384.0;

/// Where an action lands: an element of the latest snapshot, or a pixel of the last picture.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Ref(String),
    Point(i32, i32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppAction {
    Click(Target),
    DoubleClick(Target),
    /// `target`: click that element first (so the text goes there).
    Type { text: String, target: Option<String> },
    Key(u16),
    /// `notches`: positive scrolls up.
    Scroll { target: Target, notches: i32 },
}

/// One control as the answer lists it (and as the password check sees it).
#[derive(Clone, Debug, PartialEq)]
pub struct ObsElem {
    pub r#ref: String,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    /// Client pixels: x, y, w, h.
    pub rect: (i32, i32, i32, i32),
    /// The snapshot found a password box (UIA `IsPassword` or `ES_PASSWORD`; the name rule is applied by `is_password`).
    pub password: bool,
}

impl ObsElem {
    /// A password box by any of the three signs.
    pub fn is_password(&self) -> bool {
        self.password || (self.role == "edit" && is_password_field(&self.name, false, false))
    }
}

// ---- arguments -------------------------------------------------------------------------------------

fn bad(message: impl Into<String>) -> CallError {
    CallError::new("BAD_ARGS", message)
}

fn text_arg<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// `app_observe {app?, image?}`.
pub fn parse_observe(obj: &Map<String, Value>) -> Result<Request, CallError> {
    let image = match obj.get("image") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad("image must be true or false.")),
    };
    Ok(Request::AppObserve { app: text_arg(obj, "app").map(str::to_string), image })
}

/// The key names `app_act` knows, as virtual-key codes.
pub fn key_code(name: &str) -> Option<u16> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        _ => return None,
    })
}

fn coord(obj: &Map<String, Value>, key: &str) -> Result<Option<i32>, CallError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && (-MAX_COORD..=MAX_COORD).contains(&n) => Ok(Some(n.round() as i32)),
            _ => Err(bad(format!("{key} must be a number of pixels from the last app_observe picture."))),
        },
    }
}

fn target(obj: &Map<String, Value>, action: &str) -> Result<Target, CallError> {
    if let Some(r) = text_arg(obj, "ref") {
        return Ok(Target::Ref(r.to_string()));
    }
    match (coord(obj, "x")?, coord(obj, "y")?) {
        (Some(x), Some(y)) => Ok(Target::Point(x, y)),
        _ => Err(bad(format!("{action} needs a ref from app_observe, or both x and y."))),
    }
}

/// `app_act {app, action, ref?, x?, y?, text?, key?, delta?}`.
pub fn parse_act(obj: &Map<String, Value>) -> Result<Request, CallError> {
    let app = text_arg(obj, "app").ok_or_else(|| bad("app_act needs app: the id or name of a lent app."))?.to_string();
    let kind = text_arg(obj, "action").ok_or_else(|| bad("app_act needs an action: click, double_click, type, key or scroll."))?;
    let action = match kind {
        "click" => AppAction::Click(target(obj, "click")?),
        "double_click" => AppAction::DoubleClick(target(obj, "double_click")?),
        "type" => {
            let text = obj.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                return Err(bad("type needs text."));
            }
            if text.chars().count() > MAX_TYPE_CHARS {
                return Err(bad("That is too much text for one call. Type it in smaller parts."));
            }
            AppAction::Type { text: text.to_string(), target: text_arg(obj, "ref").map(str::to_string) }
        }
        "key" => {
            let name = text_arg(obj, "key").ok_or_else(|| bad("key needs a key name, like enter or tab."))?;
            let vk = key_code(name).ok_or_else(|| {
                bad(format!("Unknown key \"{name}\". Use enter, tab, escape, backspace, delete, up, down, left, right, pageup, pagedown, home or end."))
            })?;
            AppAction::Key(vk)
        }
        "scroll" => {
            let notches = match obj.get("delta") {
                None | Some(Value::Null) => -3.0,
                Some(v) => v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad("delta must be a number of wheel notches."))?,
            };
            AppAction::Scroll { target: target(obj, "scroll")?, notches: notches.clamp(-MAX_NOTCHES, MAX_NOTCHES).round() as i32 }
        }
        other => return Err(bad(format!("Unknown action \"{other}\". Use click, double_click, type, key or scroll."))),
    };
    Ok(Request::AppAct { app, action })
}

// ---- pure helpers ----------------------------------------------------------------------------------

/// Which lent app a name means: its id, its exact name, or a name that contains the words (only when that is one app).
pub fn resolve<'a>(apps: &'a [LentView], query: &str) -> Result<&'a LentView, CallError> {
    let q = query.trim().to_lowercase();
    if let Some(v) = apps.iter().find(|v| v.id.to_lowercase() == q) {
        return Ok(v);
    }
    let exact: Vec<&LentView> = apps.iter().filter(|v| v.app_name.to_lowercase() == q).collect();
    let pool: Vec<&LentView> = if exact.is_empty() {
        apps.iter().filter(|v| v.app_name.to_lowercase().contains(&q) || v.title.to_lowercase().contains(&q)).collect()
    } else {
        exact
    };
    match pool.as_slice() {
        [one] => Ok(one),
        [] => Err(CallError::new("NO_SUCH_APP", "NO_SUCH_APP: no lent app has that id or name. Call app_observe with no app to list them, or ask the person to lend the app first.")),
        many => {
            let ids: Vec<String> = many.iter().map(|v| format!("{} ({})", v.id, v.app_name)).collect();
            Err(bad(format!("More than one lent app matches. Use its id: {}.", ids.join(", "))))
        }
    }
}

fn clip(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_TEXT_CHARS).collect()
}

fn phase_word(view: &LentView) -> Value {
    serde_json::to_value(view.phase).unwrap_or(Value::Null)
}

fn mode_word(view: &LentView) -> Value {
    serde_json::to_value(view.mode).unwrap_or(Value::Null)
}

/// `app_observe` with no app: the list.
pub fn list_json(apps: &[LentView]) -> Value {
    if apps.is_empty() {
        return json!({ "apps": [], "message": "No apps are lent. Ask the person to lend one from the Computer tab." });
    }
    let rows: Vec<Value> = apps.iter().map(|v| json!({ "id": v.id, "name": v.app_name, "phase": phase_word(v), "mode": mode_word(v) })).collect();
    json!({ "apps": rows })
}

/// Told to the bot when the app's picture looks frozen.
pub const PICTURE_PAUSED_NOTE: &str = "The picture of this app is paused (it looks minimized or blank), so the image may be old. The controls list is still current.";

/// `app_observe` with an app: the controls (without the image, which the caller adds).
pub fn observe_json(view: &LentView, elems: &[ObsElem], truncated: bool, image_size: Option<(u32, u32)>) -> Value {
    let mut out = Map::new();
    out.insert("app".into(), json!({ "id": view.id, "name": view.app_name }));
    out.insert("phase".into(), phase_word(view));
    if matches!(view.phase, Phase::You | Phase::NeedsYou) {
        out.insert("note".into(), json!("The person has control of this app right now (phase you). You can read it but not act; wait for them to hand it back."));
    }
    if !view.picture_live {
        out.insert("picture_note".into(), json!(PICTURE_PAUSED_NOTE));
    }
    let rows: Vec<Value> = elems
        .iter()
        .take(MAX_ELEMENTS)
        .map(|e| {
            let mut row = Map::new();
            row.insert("ref".into(), json!(e.r#ref));
            row.insert("role".into(), json!(e.role));
            row.insert("name".into(), json!(clip(&e.name)));
            if let Some(v) = e.value.as_deref().filter(|v| !v.is_empty() && !e.is_password()) {
                row.insert("value".into(), json!(clip(v)));
            }
            Value::Object(row)
        })
        .collect();
    out.insert("truncated".into(), json!(truncated || elems.len() > MAX_ELEMENTS));
    out.insert("elements".into(), Value::Array(rows));
    if let Some((w, h)) = image_size {
        out.insert("image_size".into(), json!({ "w": w, "h": h }));
    }
    Value::Object(out)
}

/// A picture pixel as a client pixel: subtract where the client area starts inside the window picture.
pub fn picture_to_client(x: i32, y: i32, client_origin: (i32, i32)) -> (i32, i32) {
    (x - client_origin.0, y - client_origin.1)
}

/// The centre of an element's rectangle (client pixels).
pub fn centre(e: &ObsElem) -> (i32, i32) {
    (e.rect.0 + e.rect.2 / 2, e.rect.1 + e.rect.3 / 2)
}

/// Is this field a password box? Its name says so (English, Chinese, Japanese or Korean), or UIA `IsPassword` is true,
/// or it is a Win32 edit with `ES_PASSWORD`. The caller applies the name rule to edit controls only.
pub fn is_password_field(name: &str, is_password_prop: bool, has_es_password: bool) -> bool {
    if is_password_prop || has_es_password {
        return true;
    }
    let name = name.to_lowercase();
    ["password", "passcode", "密碼", "密码", "パスワード", "비밀번호"].iter().any(|w| name.contains(w))
}

/// Told to the bot when a Chromium or Electron app gave (almost) no controls.
pub const NO_LABELS_NOTE: &str = "No labels for this app; use x/y on the picture.";

/// Pure: the "no labels" note, for a snapshot of `count` elements of a window of this class.
pub fn no_labels_note(count: usize, class: &str) -> Option<&'static str> {
    (count < crate::potato::lend::FEW_ELEMENTS && crate::potato::lend::is_chromium_class(class)).then_some(NO_LABELS_NOTE)
}

/// Is the smallest edit control holding the client point a password box?
pub fn password_at(elems: &[ObsElem], x: i32, y: i32) -> bool {
    elems
        .iter()
        .filter(|e| e.role == "edit" && x >= e.rect.0 && x < e.rect.0 + e.rect.2 && y >= e.rect.1 && y < e.rect.1 + e.rect.3)
        .min_by_key(|e| e.rect.2 as i64 * e.rect.3 as i64)
        .is_some_and(ObsElem::is_password)
}

/// Windows wheel units for notches (positive = up).
pub fn wheel_delta(notches: i32) -> i32 {
    notches.saturating_mul(NOTCH)
}

fn map_error(e: LendError) -> CallError {
    match e {
        LendError::PersonHasControl => CallError {
            code: "LEASE_HELD_BY_PERSON",
            message: "The person is using that app right now. Tell them you are waiting, and try again after they hand it back.".to_string(),
            needs_you: true,
        },
        LendError::NoSuchApp => CallError::new("NO_SUCH_APP", "NO_SUCH_APP: that app is not lent (any more). Ask the person to lend it first."),
        LendError::Gone => CallError::new("APP_GONE", "APP_GONE: that app was closed, so it is no longer lent."),
        LendError::Unsupported => CallError::new("APP_UNSUPPORTED", "Lending apps works on Windows only."),
        LendError::Os(why) => CallError::new("APP_ACTION_FAILED", format!("That did not work: {why}")),
    }
}

// ---- the real apps ---------------------------------------------------------------------------------

#[cfg(windows)]
mod os {
    use super::*;
    use crate::potato::lend::session::{Elem, Snap};

    pub fn elems_of(snap: &Snap) -> (Vec<ObsElem>, bool) {
        (snap.elements.iter().map(obs_of).collect(), snap.truncated)
    }
    pub fn obs_of(e: &Elem) -> ObsElem {
        ObsElem { r#ref: e.r#ref.clone(), role: e.role.clone(), name: e.name.clone(), value: e.value.clone(), rect: (e.rect.x, e.rect.y, e.rect.w, e.rect.h), password: e.password }
    }
    pub fn class_name(hwnd: isize) -> String {
        crate::potato::lend::win::class_name(hwnd)
    }
    pub fn client_origin(hwnd: isize) -> Result<(i32, i32), LendError> {
        crate::potato::lend::win::client_origin_in_window(hwnd)
    }
    pub fn window_size(hwnd: isize) -> Result<(u32, u32), LendError> {
        let r = crate::potato::lend::win::info(hwnd)?.rect;
        Ok((r.w.max(0) as u32, r.h.max(0) as u32))
    }
}

#[cfg(not(windows))]
mod os {
    use super::*;
    use crate::potato::lend::session::{Elem, Snap};

    pub fn elems_of(_: &Snap) -> (Vec<ObsElem>, bool) {
        (Vec::new(), false)
    }
    pub fn obs_of(_: &Elem) -> ObsElem {
        ObsElem { r#ref: String::new(), role: String::new(), name: String::new(), value: None, rect: (0, 0, 0, 0), password: false }
    }
    pub fn class_name(_: isize) -> String {
        String::new()
    }
    pub fn client_origin(_: isize) -> Result<(i32, i32), LendError> {
        Err(LendError::Unsupported)
    }
    pub fn window_size(_: isize) -> Result<(u32, u32), LendError> {
        Err(LendError::Unsupported)
    }
}

/// Per app: (picture w, picture h, window w, window h) of the last `app_observe` picture, to turn picture pixels into window pixels.
fn picture_scales() -> &'static Mutex<HashMap<String, (u32, u32, u32, u32)>> {
    static SCALES: OnceLock<Mutex<HashMap<String, (u32, u32, u32, u32)>>> = OnceLock::new();
    SCALES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pure: a picture pixel as a window pixel, given (picture w, h, window w, h). No scale known or same size: unchanged.
pub fn picture_to_window(x: i32, y: i32, scale: Option<(u32, u32, u32, u32)>) -> (i32, i32) {
    match scale {
        Some((pw, ph, ww, wh)) if pw > 0 && ph > 0 && (pw, ph) != (ww, wh) => (
            (x as i64 * ww as i64 / pw as i64) as i32,
            (y as i64 * wh as i64 / ph as i64) as i32,
        ),
        _ => (x, y),
    }
}

/// The type of a picture, from its first bytes.
fn picture_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        "image/png"
    }
}

/// Apps (by id) where the last click was on a password box.
fn password_marks() -> &'static Mutex<HashMap<String, bool>> {
    static MARKS: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    MARKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn observe(app: Option<&str>, image: bool, want_image: bool) -> Result<Value, CallError> {
    let hub = hub();
    let apps = hub.list();
    let Some(query) = app else {
        return Ok(list_json(&apps));
    };
    let view = resolve(&apps, query)?.clone();
    let snap = hub.snapshot(&view.id).map_err(map_error)?;
    let view = hub.view(&view.id).map_err(map_error)?; // the phase as it is now
    let (elems, truncated) = os::elems_of(&snap);
    let png = if image && want_image { Some(hub.capture(&view.id).map_err(map_error)?) } else { None };
    let size = png.as_deref().and_then(crate::potato::sandbox::control::picture_size);
    if let (Some((pw, ph)), Ok(win)) = (size, os::window_size(view.hwnd)) {
        picture_scales().lock().insert(view.id.clone(), (pw, ph, win.0, win.1));
    }
    let mut out = observe_json(&view, &elems, truncated, size);
    if let Some(map) = out.as_object_mut() {
        if let Some(note) = no_labels_note(elems.len(), &os::class_name(view.hwnd)) {
            map.insert("labels_note".into(), json!(note));
        }
        match png {
            Some(png) if png.len() <= MAX_IMAGE_BYTES => {
                use base64::Engine;
                map.insert("image".into(), json!({ "mime": picture_mime(&png), "data": base64::engine::general_purpose::STANDARD.encode(&png) }));
            }
            Some(_) => {
                map.insert("image_note".into(), json!("The picture was too big to send, so you get the controls only."));
            }
            None if image => {
                map.insert("image_note".into(), json!("This model cannot see pictures, so you get the controls only."));
            }
            None => {}
        }
    }
    Ok(out)
}

/// The client point an action means, from a ref or a picture pixel.
fn point_of(id: &str, hwnd: isize, target: &Target) -> Result<(i32, i32), CallError> {
    let hub = hub();
    match target {
        Target::Point(x, y) => {
            let scale = picture_scales().lock().get(id).copied();
            let (wx, wy) = picture_to_window(*x, *y, scale);
            Ok(picture_to_client(wx, wy, os::client_origin(hwnd).map_err(map_error)?))
        }
        Target::Ref(r) => {
            let mut found = hub.element(id, r).map_err(map_error)?;
            if found.is_none() {
                hub.snapshot(id).map_err(map_error)?; // no snapshot yet: take one
                found = hub.element(id, r).map_err(map_error)?;
            }
            let e = found.ok_or_else(|| CallError::new("NO_SUCH_REF", format!("There is no element {r} in the latest look. Call app_observe again for fresh refs.")))?;
            Ok(centre(&os::obs_of(&e)))
        }
    }
}

fn marks_password(id: &str, target: &Target, point: (i32, i32)) -> bool {
    let hub = hub();
    match target {
        Target::Ref(r) => hub.element(id, r).ok().flatten().is_some_and(|e| os::obs_of(&e).is_password()),
        Target::Point(..) => hub.snapshot_cached(id).is_some_and(|snap| password_at(&os::elems_of(&snap).0, point.0, point.1)),
    }
}

fn act(app: &str, action: &AppAction) -> Result<Value, CallError> {
    let hub = hub();
    let apps = hub.list();
    let view = resolve(&apps, app)?.clone();
    let id = view.id.as_str();
    // The lease first: nothing else is read or touched while the person has the app.
    hub.bot_begin(id).map_err(map_error)?;
    let done = match action {
        AppAction::Click(t) | AppAction::DoubleClick(t) => {
            let (x, y) = point_of(id, view.hwnd, t)?;
            let pw = marks_password(id, t, (x, y));
            let single = matches!(action, AppAction::Click(_));
            if single {
                hub.act_click(id, x, y).map_err(map_error)?;
            } else {
                hub.act_double_click(id, x, y).map_err(map_error)?;
            }
            password_marks().lock().insert(id.to_string(), pw);
            if single { "click" } else { "double_click" }
        }
        AppAction::Type { text, target } => {
            if let Some(r) = target {
                let t = Target::Ref(r.clone());
                let (x, y) = point_of(id, view.hwnd, &t)?;
                let pw = marks_password(id, &t, (x, y));
                password_marks().lock().insert(id.to_string(), pw);
                if !pw {
                    hub.act_click(id, x, y).map_err(map_error)?;
                }
            }
            if password_marks().lock().get(id).copied().unwrap_or(false) {
                // Raise the "needs you" card so the person types it themselves.
                let _ = hub.needs_you(id, "A password is needed. Please type it yourself.");
                return Err(CallError::new("PASSWORD_FIELD", "Don't type passwords; ask the person."));
            }
            hub.act_type(id, text).map_err(map_error)?;
            "type"
        }
        AppAction::Key(vk) => {
            hub.act_key(id, *vk).map_err(map_error)?;
            password_marks().lock().insert(id.to_string(), false);
            "key"
        }
        AppAction::Scroll { target, notches } => {
            let (x, y) = point_of(id, view.hwnd, target)?;
            hub.act_scroll(id, x, y, wheel_delta(*notches)).map_err(map_error)?;
            "scroll"
        }
    };
    Ok(json!({ "ok": true, "done": done, "app": { "id": view.id, "name": view.app_name }, "note": "Call app_observe to see what changed." }))
}

impl Calls {
    /// Runs `app_observe` / `app_act`. Not the sandbox: no VM, no screen lease, no `Sandbox` switch.
    pub(super) fn run_app_call(&self, gates: Gates, ctx: &Context, tool: Tool, args: &Value) -> Result<Value, CallError> {
        if !gates.bot_on {
            return Err(CallError::new("POTATO_OFF", "Potato Bot is switched off."));
        }
        if !super::rules::tool_access(ctx.trust).sandbox {
            return Err(CallError::new("TOOL_NOT_ALLOWED", "This chat may not use the owner's apps."));
        }
        if !super::live::lend_labs_on() {
            return Err(CallError::new("LEND_OFF", "Lending apps is switched off. Turn on Lend an App in Settings > General > Labs."));
        }
        let request = super::sandbox_calls::validate(tool, args, None, &Default::default())?;
        let now = self.host.now_ms();
        let mut answer = match request {
            Request::AppObserve { app, image } => {
                self.spend(ctx, now, 1)?;
                observe(app.as_deref(), image, ctx.want_image)?
            }
            Request::AppAct { app, action } => {
                // Refuse before the day's cap is spent when the person has the app or it is not lent.
                let apps = hub().list();
                let view = resolve(&apps, &app)?;
                if matches!(view.phase, Phase::You | Phase::NeedsYou) {
                    return Err(map_error(LendError::PersonHasControl));
                }
                self.spend(ctx, now, 1)?;
                act(&app, &action)?
            }
            _ => return Err(CallError::new("UNKNOWN_TOOL", "There is no such app tool.")),
        };
        super::sandbox_calls::redact(&mut answer);
        Ok(answer)
    }
}

#[cfg(test)]
#[path = "lend_tools_tests.rs"]
mod tests;
