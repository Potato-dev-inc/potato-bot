//! Lend an App v2: mirror a lent Windows app into the sandbox screen.
//!
//! Every lent app gets two threads. While the sandbox runs they keep a viewer window open on the bot's desktop
//! (`POST /v1/lend/open`).
//! - The FRAME thread (`Worker`) sends a JPEG of the lent window (`/v1/lend/frame`, long side at most 1280 px, quality 60):
//!   every 100 ms while the picture is changing, every 500 ms when it is still, skipped while the pixels are unchanged
//!   (keep-alive every 5 s). Right after input is applied it sends a frame at once, and again 120 ms later.
//! - The INPUT thread (`Pump`) long-polls `/v1/lend/events` (the guest holds the request until an event arrives, at
//!   most 1.5 s) and applies what comes back, then asks again at once.
//!
//! Who the input belongs to: when the PERSON has taken control of the sandbox screen, what they click and type in the
//! viewer is theirs. It is posted without the lend lease (it does not turn the app into "bot is using it" and is never
//! dropped as "the person has it"). Otherwise it is the bot's: the lease is checked and the event is dropped while the
//! person has the app.
//! Only the HOST starts connections; the guest never reaches us.
//!
//! `Worker` and `Pump` are the whole behaviour behind two small traits (`Link`: the control service, `Win`: the lent
//! window), so `mirror_tests.rs` runs them with fakes and a clock it moves itself. No WSL, no real window.
//!
//! Coordinates: a frame covers the whole window, so event pixels = window picture pixels even when the JPEG is smaller
//! (the viewer scales); the input layer wants client pixels, so the window's client origin is subtracted (same as `lend_tools`).
//! Right clicks are dropped (the session has no right-click helper).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::session::{Input, Picture};
use super::{Hwnd, LendError};

/// A frame is looked at this often while the picture is changing.
pub const FRAME_EVERY_MS: i64 = 100;
/// ... and this often when it has been still for `ACTIVE_FOR_MS`.
pub const IDLE_FRAME_EVERY_MS: i64 = 500;
/// The picture counts as changing for this long after its last change.
pub const ACTIVE_FOR_MS: i64 = 2000;
/// A second frame this long after input was applied.
pub const AFTER_INPUT_MS: i64 = 120;
/// What the viewer did is fetched this often when `Worker::step` drives everything (tests; no long poll).
#[cfg_attr(not(test), allow(dead_code))]
pub const EVENTS_EVERY_MS: i64 = 100;
/// The guest holds `/v1/lend/events` this long waiting for an event.
pub const EVENTS_WAIT_MS: u64 = 1500;
/// The same picture is sent again after this long, so the viewer knows we are alive.
pub const KEEP_ALIVE_MS: i64 = 5000;
/// While the viewer is not open, opening is tried this often.
pub const REOPEN_EVERY_MS: i64 = 4000;
/// Frames are shrunk so their long side is at most this.
pub const FRAME_MAX_SIDE: u32 = 1280;
const JPEG_QUALITY: u8 = 60;
const TICK: Duration = Duration::from_millis(50);
const MAX_EVENTS: usize = 200;

// ---- the two sides ----------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    /// The sandbox is not up (or cannot be reached): try again later.
    NotRunning,
    Failed(String),
}

/// The sandbox's control service, as the mirror uses it.
pub trait Link: Send + Sync {
    /// Is the sandbox up right now? Cheap (never starts it).
    fn ready(&self) -> bool;
    /// POST a JSON body to a `/v1/lend/*` route.
    fn post(&self, path: &str, body: &Value) -> Result<Value, LinkError>;
}

/// One picture of the lent window. `bgra`: the bytes are B,G,R,A (as Windows gives them) instead of R,G,B,A.
pub struct Shot {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub bgra: bool,
}

/// Who an input belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// The bot: the lease is checked; `PersonHasControl` means "drop it".
    Bot,
    /// The person at the sandbox screen: no lease check, no phase change.
    Person,
}

/// The lent window.
pub trait Win: Send + Sync {
    fn capture(&self) -> Result<Shot, LendError>;
    /// Is the window minimized right now? (A minimized window cannot be captured.)
    fn is_iconic(&self) -> bool {
        false
    }
    /// Tell the session what the latest picture looks like, so it can fix or report a paused one.
    fn report_picture(&self, _picture: Picture) {}
    fn client_origin(&self) -> Result<(i32, i32), LendError>;
    /// Has the person taken control of the sandbox screen (so viewer input is theirs)?
    fn person_has_screen(&self) -> bool;
    fn click(&self, who: Who, x: i32, y: i32) -> Result<(), LendError>;
    fn double_click(&self, who: Who, x: i32, y: i32) -> Result<(), LendError>;
    /// The second half of a double click only (the first click was sent on its own).
    fn double_click_second(&self, who: Who, x: i32, y: i32) -> Result<(), LendError>;
    fn type_text(&self, who: Who, text: &str) -> Result<(), LendError>;
    fn key(&self, who: Who, vk: u16) -> Result<(), LendError>;
    fn scroll(&self, who: Who, x: i32, y: i32, delta: i32) -> Result<(), LendError>;
}

// ---- events -----------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum Event {
    Click { x: i32, y: i32, right: bool, count: u32 },
    /// The second click of a double click: the first one already went out as a `Click`.
    DoubleClick { x: i32, y: i32 },
    Text(String),
    Key(String),
    Scroll { x: i32, y: i32, dy: f64 },
}

fn num(v: &Value, key: &str) -> Option<i32> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= 16_384.0).map(|n| n.round() as i32)
}

/// The events of a `/v1/lend/events` reply; anything malformed is skipped.
pub fn parse_events(reply: &Value) -> Vec<Event> {
    let Some(list) = reply.get("events").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .take(MAX_EVENTS)
        .filter_map(|e| match e.get("t").and_then(Value::as_str)? {
            "click" => Some(Event::Click {
                x: num(e, "x")?,
                y: num(e, "y")?,
                right: e.get("button").and_then(Value::as_str) == Some("right"),
                count: e.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 2) as u32,
            }),
            "dblclick" => Some(Event::DoubleClick { x: num(e, "x")?, y: num(e, "y")? }),
            "text" => e.get("s").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| Event::Text(s.to_string())),
            "key" => e.get("k").and_then(Value::as_str).map(|k| Event::Key(k.to_string())),
            "scroll" => Some(Event::Scroll { x: num(e, "x")?, y: num(e, "y")?, dy: e.get("dy").and_then(Value::as_f64).filter(|d| d.is_finite())? }),
            _ => None,
        })
        .collect()
}

/// Wheel notches for the guest's `dy` (already in notches, positive = down): Windows wants positive = up.
pub fn notches_for(dy: f64) -> i32 {
    if dy == 0.0 {
        return 0;
    }
    let n = dy.round().clamp(-20.0, 20.0) as i32;
    -(if n == 0 { dy.signum() as i32 } else { n })
}

// ---- shared state, the worker, the pump -------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Go,
    /// The window is gone: stop.
    Gone,
}

/// What the frame side and the input side share.
struct Shared {
    id: String,
    link: Arc<dyn Link>,
    win: Arc<dyn Win>,
    opened: AtomicBool,
    /// Counts the polls that applied input; the frame side sends a frame at once when it changes.
    applied: AtomicU64,
    /// The frame thread, woken when input was applied.
    waker: Mutex<Option<std::thread::Thread>>,
}

impl Shared {
    fn wake(&self) {
        if let Some(t) = self.waker.lock().as_ref() {
            t.unpark();
        }
    }
}

/// The input side: asks for the viewer's events and applies them.
pub struct Pump {
    shared: Arc<Shared>,
}

impl Pump {
    /// One request for events. With `wait` the guest holds it until an event arrives (at most `EVENTS_WAIT_MS`).
    /// `Err(())`: the sandbox did not answer, so the viewer is gone.
    pub fn poll(&self, wait: bool) -> Result<Flow, ()> {
        let s = &self.shared;
        let body = if wait { json!({ "id": s.id, "wait_ms": EVENTS_WAIT_MS }) } else { json!({ "id": s.id }) };
        let reply = match s.link.post("/v1/lend/events", &body) {
            Ok(r) => r,
            Err(_) => {
                s.opened.store(false, Ordering::SeqCst);
                return Err(());
            }
        };
        let mut any = false;
        for event in parse_events(&reply) {
            // PersonHasControl and every other failure: the event is dropped. Only a vanished window ends the loop.
            match self.apply(event) {
                Err(LendError::Gone) => return Ok(Flow::Gone),
                Ok(()) => any = true,
                Err(_) => {}
            }
        }
        if any {
            s.applied.fetch_add(1, Ordering::SeqCst);
            s.wake();
        }
        Ok(Flow::Go)
    }

    fn apply(&self, event: Event) -> Result<(), LendError> {
        use crate::potato::lend_tools as tools;
        let win = &self.shared.win;
        let who = if win.person_has_screen() { Who::Person } else { Who::Bot };
        let to_client = |x: i32, y: i32| -> Result<(i32, i32), LendError> { Ok(tools::picture_to_client(x, y, win.client_origin()?)) };
        match event {
            Event::Click { right: true, .. } => Ok(()),
            Event::Click { x, y, count, .. } => {
                let (cx, cy) = to_client(x, y)?;
                if count >= 2 {
                    win.double_click(who, cx, cy)
                } else {
                    win.click(who, cx, cy)
                }
            }
            Event::DoubleClick { x, y } => {
                let (cx, cy) = to_client(x, y)?;
                win.double_click_second(who, cx, cy)
            }
            Event::Text(s) => {
                let s: String = s.chars().take(tools::MAX_TYPE_CHARS).collect();
                win.type_text(who, &s)
            }
            Event::Key(k) => match tools::key_code(&k) {
                Some(vk) => win.key(who, vk),
                None => Ok(()),
            },
            Event::Scroll { x, y, dy } => {
                let notches = notches_for(dy);
                if notches == 0 {
                    return Ok(());
                }
                let (cx, cy) = to_client(x, y)?;
                win.scroll(who, cx, cy, tools::wheel_delta(notches))
            }
        }
    }
}

pub struct Worker {
    shared: Arc<Shared>,
    title: String,
    was_open: bool,
    last_open_try: Option<i64>,
    last_events: Option<i64>,
    last_capture: Option<i64>,
    size: (u32, u32),
    last_hash: Option<u64>,
    last_sent: i64,
    last_change: i64,
    seq: u64,
    seen_applied: u64,
    burst_at: Option<i64>,
    /// The next frame is sent even if the fingerprint did not change (right after input).
    force_next: bool,
}

/// A cheap fingerprint of a picture: its size and every 7th pixel of every 4th row (not every byte; 8 MB of hashing in a debug
/// build costs real time). A change smaller than that can slip through, so a frame is always forced right after input and the
/// keep-alive resends the picture every 5 s.
fn hash_of(shot: &Shot) -> u64 {
    let mut h = DefaultHasher::new();
    (shot.w, shot.h).hash(&mut h);
    let row_bytes = shot.w as usize * 4;
    if row_bytes > 0 {
        for row in shot.rgba.chunks_exact(row_bytes).step_by(4) {
            for px in row.chunks_exact(4).step_by(7) {
                px.hash(&mut h);
            }
        }
    }
    h.finish()
}

/// How many pixels per side `is_blank` looks at.
const BLANK_GRID: usize = 32;
/// A channel at or below this counts as black.
const NEAR_BLACK: u8 = 8;

/// Pure: does the picture look like a window that stopped drawing? True when every sampled pixel is the same colour, or all
/// are near black. Samples a grid of at most `BLANK_GRID` x `BLANK_GRID` pixels; alpha is ignored, and the channel order
/// does not matter (same colour and black are the same in RGBA and BGRA). A zero-size or short buffer is not blank.
pub fn is_blank(shot: &Shot) -> bool {
    let (w, h) = (shot.w as usize, shot.h as usize);
    if w == 0 || h == 0 || shot.rgba.len() < w * h * 4 {
        return false;
    }
    let first = &shot.rgba[0..3];
    let (mut same, mut dark) = (true, true);
    for gy in 0..BLANK_GRID.min(h) {
        let y = gy * h / BLANK_GRID.min(h);
        for gx in 0..BLANK_GRID.min(w) {
            let x = gx * w / BLANK_GRID.min(w);
            let i = (y * w + x) * 4;
            let px = &shot.rgba[i..i + 3];
            same &= px == first;
            dark &= px.iter().all(|c| *c <= NEAR_BLACK);
            if !same && !dark {
                return false;
            }
        }
    }
    true
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Pure: a picture to a JPEG (quality 60), shrunk so the long side is at most `FRAME_MAX_SIDE`. Returns (jpeg, width, height) of the JPEG.
pub fn encode_jpeg(shot: &Shot) -> Result<(Vec<u8>, u32, u32), String> {
    if shot.rgba.len() != shot.w as usize * shot.h as usize * 4 {
        return Err("pixel buffer has the wrong size".to_string());
    }
    let (w, h, rgb) = if shot.bgra {
        super::capture::bgra_to_rgb_scaled(&shot.rgba, shot.w, shot.h, FRAME_MAX_SIDE)
    } else {
        super::capture::rgba_to_rgb_scaled(&shot.rgba, shot.w, shot.h, FRAME_MAX_SIDE)
    };
    let jpeg = super::capture::encode_jpeg_rgb(w, h, &rgb, JPEG_QUALITY).map_err(|e| format!("{e:?}"))?;
    Ok((jpeg, w, h))
}

impl Worker {
    pub fn new(id: &str, title: &str, link: Arc<dyn Link>, win: Arc<dyn Win>) -> Self {
        let shared = Arc::new(Shared {
            id: id.to_string(),
            link,
            win,
            opened: AtomicBool::new(false),
            applied: AtomicU64::new(0),
            waker: Mutex::new(None),
        });
        Worker {
            shared,
            title: title.to_string(),
            was_open: false,
            last_open_try: None,
            last_events: None,
            last_capture: None,
            size: (0, 0),
            last_hash: None,
            last_sent: 0,
            last_change: i64::MIN / 2,
            seq: 0,
            seen_applied: 0,
            burst_at: None,
            force_next: false,
        }
    }

    /// The input side, for the thread that long-polls.
    pub fn pump(&self) -> Pump {
        Pump { shared: self.shared.clone() }
    }

    pub fn is_open(&self) -> bool {
        self.shared.opened.load(Ordering::SeqCst)
    }

    fn due(last: &mut Option<i64>, now: i64, every: i64) -> bool {
        match *last {
            Some(t) if now - t < every => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }

    fn open_body(&self, w: u32, h: u32) -> Value {
        json!({ "id": self.shared.id, "title": self.title, "w": w, "h": h })
    }

    /// Open the viewer when the sandbox is up.
    fn try_open(&mut self) -> Flow {
        if !self.shared.link.ready() {
            return Flow::Go;
        }
        let shot = match self.shared.win.capture() {
            Ok(s) => s,
            Err(LendError::Gone) => return Flow::Gone,
            Err(_) => return Flow::Go,
        };
        if self.shared.link.post("/v1/lend/open", &self.open_body(shot.w, shot.h)).is_ok() {
            self.shared.opened.store(true, Ordering::SeqCst);
            self.was_open = true;
            self.size = (shot.w, shot.h);
            self.last_hash = None; // the first frame goes out at once
            self.last_capture = None;
            self.last_events = None;
        }
        Flow::Go
    }

    /// The frame side only: open the viewer, send frames. Call about every 50 ms (or when woken).
    pub fn step_frames(&mut self, now: i64) -> Flow {
        if !self.is_open() {
            if self.was_open {
                // The sandbox went away: open again when it is back.
                self.was_open = false;
                self.last_open_try = Some(now);
            }
            if Self::due(&mut self.last_open_try, now, REOPEN_EVERY_MS) {
                return self.try_open();
            }
            return Flow::Go;
        }
        // Input was applied: look at once, and once more shortly after (the app needs a moment to redraw).
        let applied = self.shared.applied.load(Ordering::SeqCst);
        if applied != self.seen_applied {
            self.seen_applied = applied;
            self.burst_at = Some(now + AFTER_INPUT_MS);
            self.last_capture = None;
            self.force_next = true;
        } else if self.burst_at.is_some_and(|t| now >= t) {
            self.burst_at = None;
            self.last_capture = None;
            self.force_next = true;
        }
        let every = if now - self.last_change < ACTIVE_FOR_MS || self.burst_at.is_some() { FRAME_EVERY_MS } else { IDLE_FRAME_EVERY_MS };
        if Self::due(&mut self.last_capture, now, every) {
            return self.frame(now);
        }
        Flow::Go
    }

    /// Frames and, without a long poll, events too (what the tests use).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn step(&mut self, now: i64) -> Flow {
        if self.step_frames(now) == Flow::Gone {
            return Flow::Gone;
        }
        if self.is_open() && Self::due(&mut self.last_events, now, EVENTS_EVERY_MS) {
            if let Ok(Flow::Gone) = self.pump().poll(false) {
                return Flow::Gone;
            }
        }
        Flow::Go
    }

    fn frame(&mut self, now: i64) -> Flow {
        let s = self.shared.clone();
        let shot = match s.win.capture() {
            Ok(shot) => shot,
            Err(LendError::Gone) => return Flow::Gone,
            Err(_) => {
                // Minimized or busy: keep the last frame, and tell the session when the window is minimized.
                if s.win.is_iconic() {
                    s.win.report_picture(Picture::Minimized);
                }
                return Flow::Go;
            }
        };
        s.win.report_picture(if is_blank(&shot) { Picture::Blank } else { Picture::Live });
        if (shot.w, shot.h) != self.size {
            if s.link.post("/v1/lend/open", &self.open_body(shot.w, shot.h)).is_err() {
                s.opened.store(false, Ordering::SeqCst);
                return Flow::Go;
            }
            self.size = (shot.w, shot.h);
            self.last_hash = None;
        }
        let hash = hash_of(&shot);
        let changed = self.last_hash != Some(hash);
        let force = std::mem::take(&mut self.force_next);
        if !changed && !force && now - self.last_sent < KEEP_ALIVE_MS {
            return Flow::Go;
        }
        let captured_ms = unix_ms();
        let Ok((jpeg, ..)) = encode_jpeg(&shot) else {
            return Flow::Go;
        };
        self.seq += 1;
        // w and h are the WINDOW's size (what event pixels mean); the JPEG itself may be smaller.
        let body = json!({
            "id": s.id, "seq": self.seq, "w": shot.w, "h": shot.h, "ts": captured_ms,
            "jpeg_b64": base64::engine::general_purpose::STANDARD.encode(&jpeg),
        });
        let mut sent = s.link.post("/v1/lend/frame", &body);
        if matches!(sent, Err(LinkError::Failed(_))) {
            // The guest forgot the app (service restart, or closed): "app is not open". Open again, send once more.
            if s.link.post("/v1/lend/open", &self.open_body(shot.w, shot.h)).is_ok() {
                sent = s.link.post("/v1/lend/frame", &body);
            }
        }
        match sent {
            Ok(_) => {
                self.last_hash = Some(hash);
                self.last_sent = now;
                if changed {
                    self.last_change = now;
                }
            }
            Err(_) => s.opened.store(false, Ordering::SeqCst),
        }
        Flow::Go
    }

    /// Close the viewer (if it was opened).
    pub fn close(&mut self) {
        if self.is_open() {
            let _ = self.shared.link.post("/v1/lend/close", &json!({ "id": self.shared.id }));
            self.shared.opened.store(false, Ordering::SeqCst);
        }
    }
}

// ---- the threads ------------------------------------------------------------------------------------

/// A running mirror. Dropping it (or `stop`) ends its threads, which then close the viewer.
pub struct Mirror {
    stop: Arc<AtomicBool>,
}

impl Mirror {
    pub fn start(mut worker: Worker) -> Mirror {
        let stop = Arc::new(AtomicBool::new(false));
        let pump = worker.pump();
        // The input thread: long poll, apply, ask again at once. Ends with the window.
        let flag = stop.clone();
        let input = std::thread::Builder::new().name("potato-lend-input".into()).spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                if !pump.shared.opened.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                let started = Instant::now();
                match pump.poll(true) {
                    Ok(Flow::Gone) => {
                        flag.store(true, Ordering::SeqCst);
                        pump.shared.wake();
                        break;
                    }
                    Ok(Flow::Go) => {
                        // An older guest answers at once: do not spin.
                        if started.elapsed() < Duration::from_millis(30) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                    Err(()) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        });
        // The frame thread.
        let flag = stop.clone();
        let frames = std::thread::Builder::new().name("potato-lend-mirror".into()).spawn(move || {
            *worker.shared.waker.lock() = Some(std::thread::current());
            let epoch = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                if worker.step_frames(epoch.elapsed().as_millis() as i64) == Flow::Gone {
                    break;
                }
                std::thread::park_timeout(TICK);
            }
            flag.store(true, Ordering::SeqCst);
            worker.close();
        });
        if input.is_err() || frames.is_err() {
            stop.store(true, Ordering::SeqCst);
        }
        Mirror { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Mirror {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- which apps are mirrored ------------------------------------------------------------------------

/// A lent app, as much as the mirror needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lent {
    pub id: String,
    pub title: String,
    pub hwnd: Hwnd,
}

#[derive(Default)]
pub struct Registry {
    running: HashMap<String, (Hwnd, Mirror)>,
}

impl Registry {
    /// Start a mirror for each lent app that has none, stop the ones whose app is no longer lent.
    pub fn reconcile(&mut self, apps: &[Lent], make: &dyn Fn(&Lent) -> Mirror) {
        self.running.retain(|id, (hwnd, _)| apps.iter().any(|a| &a.id == id && a.hwnd == *hwnd)); // Drop stops it
        for app in apps {
            if !self.running.contains_key(&app.id) {
                self.running.insert(app.id.clone(), (app.hwnd, make(app)));
            }
        }
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.running.keys().cloned().collect();
        ids.sort();
        ids
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// Called by the session after every change (and when it releases everything): `apps` is the whole current list.
pub fn reconcile(apps: &[Lent]) {
    registry().lock().reconcile(apps, &|app| {
        Mirror::start(Worker::new(&app.id, &app.title, Arc::new(RealLink), Arc::new(RealWin { id: app.id.clone(), hwnd: app.hwnd })))
    });
}

// ---- the real sandbox and the real window -----------------------------------------------------------

struct RealLink;

impl Link for RealLink {
    fn ready(&self) -> bool {
        crate::potato::sandbox_host::existing_sandbox().is_some_and(|s| s.lend_ready())
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, LinkError> {
        let sandbox = crate::potato::sandbox_host::existing_sandbox().ok_or(LinkError::NotRunning)?;
        match sandbox.lend_post(path, body) {
            Ok(v) => Ok(v),
            Err(e) => match e.code {
                "SANDBOX_NOT_RUNNING" | "SANDBOX_UNREACHABLE" => Err(LinkError::NotRunning),
                // The guest took it but did not answer in JSON: that is fine for open, frame and close.
                "SANDBOX_BAD_REPLY" => Ok(Value::Null),
                code => Err(LinkError::Failed(code.to_string())),
            },
        }
    }
}

struct RealWin {
    id: String,
    hwnd: Hwnd,
}

impl RealWin {
    fn send(&self, who: Who, input: Input) -> Result<(), LendError> {
        let hub = super::session::hub();
        match who {
            Who::Bot => hub.bot_input(&self.id, &input),
            Who::Person => hub.person_input(&self.id, &input),
        }
    }
}

impl Win for RealWin {
    fn capture(&self) -> Result<Shot, LendError> {
        let (rgba, w, h) = super::capture::bgra(self.hwnd)?;
        Ok(Shot { rgba, w, h, bgra: true })
    }
    fn is_iconic(&self) -> bool {
        super::hide::is_iconic(self.hwnd)
    }
    fn report_picture(&self, picture: Picture) {
        let _ = super::session::hub().picture_report(&self.id, picture);
    }
    fn client_origin(&self) -> Result<(i32, i32), LendError> {
        super::win::client_origin_in_window(self.hwnd)
    }
    fn person_has_screen(&self) -> bool {
        use crate::potato::sandbox::lease::Banner;
        let hub = crate::potato::sandbox::screen::hub();
        let banner = hub.lease().lock().banner(hub.now_ms());
        matches!(banner, Banner::PersonHasControl)
    }
    fn click(&self, who: Who, x: i32, y: i32) -> Result<(), LendError> {
        self.send(who, Input::Click(x, y))
    }
    fn double_click(&self, who: Who, x: i32, y: i32) -> Result<(), LendError> {
        self.send(who, Input::DoubleClick(x, y))
    }
    fn double_click_second(&self, who: Who, x: i32, y: i32) -> Result<(), LendError> {
        self.send(who, Input::DoubleClickSecond(x, y))
    }
    fn type_text(&self, who: Who, text: &str) -> Result<(), LendError> {
        self.send(who, Input::Type(text.to_string()))
    }
    fn key(&self, who: Who, vk: u16) -> Result<(), LendError> {
        self.send(who, Input::Key(vk))
    }
    fn scroll(&self, who: Who, x: i32, y: i32, delta: i32) -> Result<(), LendError> {
        self.send(who, Input::Scroll(x, y, delta))
    }
}
