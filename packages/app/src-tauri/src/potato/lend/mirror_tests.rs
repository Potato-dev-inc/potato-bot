//! `mirror.rs` with a fake sandbox link and a fake window: no WSL, no real window. The clock is a number the test moves.

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::mirror::*;
use super::session::Picture;
use super::LendError;

#[derive(Default)]
struct FakeLink {
    up: Mutex<bool>,
    posts: Mutex<Vec<(String, Value)>>,
    events: Mutex<VecDeque<Value>>,
    /// The next frame post is refused like the guest's 400 "app is not open".
    forget: Mutex<bool>,
    /// An old guest: the raw route answers 404.
    old_guest: Mutex<bool>,
    /// Every raw post that reached the guest: (path, headers, body).
    raws: Mutex<Vec<(String, Vec<(String, String)>, Vec<u8>)>>,
    /// How many raw posts were tried (also the refused ones).
    raw_tries: Mutex<usize>,
}

impl FakeLink {
    fn running() -> Arc<Self> {
        let l = Arc::new(Self::default());
        *l.up.lock() = true;
        l
    }
    fn paths(&self) -> Vec<String> {
        self.posts.lock().iter().map(|(p, _)| p.clone()).collect()
    }
    fn count(&self, path: &str) -> usize {
        self.paths().iter().filter(|p| *p == path).count()
    }
    fn queue(&self, events: Value) {
        self.events.lock().push_back(json!({ "events": events }));
    }
}

impl Link for FakeLink {
    fn ready(&self) -> bool {
        *self.up.lock()
    }
    fn post(&self, path: &str, body: &Value) -> Result<Value, LinkError> {
        if !*self.up.lock() {
            return Err(LinkError::NotRunning);
        }
        if path == "/v1/lend/frame" && std::mem::take(&mut *self.forget.lock()) {
            return Err(LinkError::Failed("SANDBOX_REJECTED".into()));
        }
        self.posts.lock().push((path.to_string(), body.clone()));
        if path == "/v1/lend/events" {
            return Ok(self.events.lock().pop_front().unwrap_or_else(|| json!({ "events": [] })));
        }
        Ok(json!({ "ok": true }))
    }
    fn post_raw(&self, path: &str, headers: &[(&str, String)], body: &[u8]) -> Result<Value, LinkError> {
        use base64::Engine;
        if !*self.up.lock() {
            return Err(LinkError::NotRunning);
        }
        *self.raw_tries.lock() += 1;
        if *self.old_guest.lock() {
            return Err(LinkError::NotFound);
        }
        if std::mem::take(&mut *self.forget.lock()) {
            return Err(LinkError::Failed("SANDBOX_REJECTED".into()));
        }
        let head: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        self.raws.lock().push((path.to_string(), head, body.to_vec()));
        // Shown to the older tests as the frame it is.
        let get = |name: &str| headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.parse::<u64>().unwrap_or(0)).unwrap_or(0);
        let frame = json!({
            "id": "l1", "seq": get("X-Seq"), "w": get("X-W"), "h": get("X-H"), "raw": true,
            "jpeg_b64": base64::engine::general_purpose::STANDARD.encode(body),
        });
        self.posts.lock().push(("/v1/lend/frame".to_string(), frame));
        Ok(json!({ "ok": true }))
    }
}

struct FakeWin {
    pixel: Mutex<u8>,
    size: Mutex<(u32, u32)>,
    origin: (i32, i32),
    person_has_it: Mutex<bool>,
    /// The person took control of the sandbox screen: viewer input is theirs.
    person_screen: Mutex<bool>,
    gone: Mutex<bool>,
    acts: Mutex<Vec<String>>,
    /// Cannot be captured and says it is minimized.
    iconic: Mutex<bool>,
    /// Cannot be captured for another reason (busy).
    fail: Mutex<bool>,
    reports: Mutex<Vec<Picture>>,
}

impl FakeWin {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            pixel: Mutex::new(10),
            size: Mutex::new((16, 8)),
            origin: (8, 31),
            person_has_it: Mutex::new(false),
            person_screen: Mutex::new(false),
            gone: Mutex::new(false),
            acts: Mutex::new(Vec::new()),
            iconic: Mutex::new(false),
            fail: Mutex::new(false),
            reports: Mutex::new(Vec::new()),
        })
    }
    fn act(&self, who: Who, what: String) -> Result<(), LendError> {
        // Only the bot is stopped by the lease; the person's own input always goes through.
        if who == Who::Bot && *self.person_has_it.lock() {
            return Err(LendError::PersonHasControl);
        }
        let tag = if who == Who::Person { "P:" } else { "" };
        self.acts.lock().push(format!("{tag}{what}"));
        Ok(())
    }
    fn acts(&self) -> Vec<String> {
        self.acts.lock().clone()
    }
}

impl Win for FakeWin {
    fn capture(&self) -> Result<Shot, LendError> {
        if *self.gone.lock() {
            return Err(LendError::Gone);
        }
        if *self.iconic.lock() || *self.fail.lock() {
            return Err(LendError::Os("cannot capture".into()));
        }
        let (w, h) = *self.size.lock();
        Ok(Shot { rgba: vec![*self.pixel.lock(); w as usize * h as usize * 4], w, h, bgra: false })
    }
    fn is_iconic(&self) -> bool {
        *self.iconic.lock()
    }
    fn report_picture(&self, picture: Picture) {
        self.reports.lock().push(picture);
    }
    fn client_origin(&self) -> Result<(i32, i32), LendError> {
        Ok(self.origin)
    }
    fn person_has_screen(&self) -> bool {
        *self.person_screen.lock()
    }
    fn click(&self, who: Who, x: i32, y: i32) -> Result<(), LendError> {
        self.act(who, format!("click {x},{y}"))
    }
    fn double_click(&self, who: Who, x: i32, y: i32) -> Result<(), LendError> {
        self.act(who, format!("double {x},{y}"))
    }
    fn double_click_second(&self, who: Who, x: i32, y: i32) -> Result<(), LendError> {
        self.act(who, format!("dblsecond {x},{y}"))
    }
    fn type_text(&self, who: Who, text: &str) -> Result<(), LendError> {
        self.act(who, format!("type {text}"))
    }
    fn key(&self, who: Who, vk: u16) -> Result<(), LendError> {
        self.act(who, format!("key {vk:#x}"))
    }
    fn scroll(&self, who: Who, x: i32, y: i32, delta: i32) -> Result<(), LendError> {
        self.act(who, format!("scroll {x},{y},{delta}"))
    }
}

fn worker(link: &Arc<FakeLink>, win: &Arc<FakeWin>) -> Worker {
    Worker::new("l1", "Notepad", link.clone(), win.clone())
}

/// Steps the worker every 50 ms from `from` to `to`.
fn run(w: &mut Worker, from: i64, to: i64) -> Flow {
    let mut t = from;
    while t <= to {
        if w.step(t) == Flow::Gone {
            return Flow::Gone;
        }
        t += 50;
    }
    Flow::Go
}

#[test]
fn opens_the_viewer_then_sends_the_first_frame() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 300);
    assert!(w.is_open());
    let posts = link.posts.lock().clone();
    assert_eq!(posts[0].0, "/v1/lend/open");
    assert_eq!(posts[0].1, json!({ "id": "l1", "title": "Notepad", "w": 16, "h": 8 }));
    let frame = posts.iter().find(|(p, _)| p == "/v1/lend/frame").expect("a frame was sent");
    assert_eq!(frame.1["seq"], 1);
    assert_eq!((frame.1["w"].as_u64(), frame.1["h"].as_u64()), (Some(16), Some(8)));
    use base64::Engine;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(frame.1["jpeg_b64"].as_str().unwrap()).unwrap();
    assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "a JPEG");
}

#[test]
fn unchanged_pixels_are_skipped_until_the_keep_alive() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 4000);
    assert_eq!(link.count("/v1/lend/frame"), 1, "same picture: one frame only");
    run(&mut w, 4050, 5800);
    assert_eq!(link.count("/v1/lend/frame"), 2, "keep-alive after 5 s");
    *win.pixel.lock() = 200;
    run(&mut w, 5850, 6700);
    assert_eq!(link.count("/v1/lend/frame"), 3, "a changed picture goes out at the next look");
    let seqs: Vec<u64> = link.posts.lock().iter().filter(|(p, _)| p == "/v1/lend/frame").map(|(_, b)| b["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
}

#[test]
fn a_resized_window_reopens_with_the_new_size_and_sends_a_frame() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 600);
    *win.size.lock() = (20, 10);
    run(&mut w, 650, 1000);
    assert_eq!(link.count("/v1/lend/open"), 2);
    let last_open = link.posts.lock().iter().rfind(|(p, _)| p == "/v1/lend/open").unwrap().1.clone();
    assert_eq!((last_open["w"].as_u64(), last_open["h"].as_u64()), (Some(20), Some(10)));
    assert_eq!(link.count("/v1/lend/frame"), 2);
}

#[test]
fn events_every_100ms_become_act_calls_in_client_pixels() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    link.queue(json!([
        { "t": "click", "x": 108, "y": 131, "button": "left", "count": 1 },
        { "t": "click", "x": 18, "y": 41, "button": "left", "count": 2 },
        { "t": "click", "x": 50, "y": 50, "button": "right", "count": 1 },
        { "t": "text", "s": "hello" },
        { "t": "key", "k": "enter" },
        { "t": "key", "k": "f13" },
        { "t": "scroll", "x": 28, "y": 51, "dy": 2 },
        { "t": "scroll", "x": 28, "y": 51, "dy": -1 },
        { "t": "bogus" },
    ]));
    run(&mut w, 150, 400);
    assert_eq!(
        win.acts(),
        vec!["click 100,100", "double 10,10", "type hello", "key 0xd", "scroll 20,20,-240", "scroll 20,20,120"],
        "right click, unknown key and unknown event are dropped; scroll down is a negative wheel"
    );
    assert!(link.count("/v1/lend/events") >= 3, "asked about every 100 ms");
}

#[test]
fn events_are_dropped_while_the_person_has_control() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    *win.person_has_it.lock() = true;
    link.queue(json!([{ "t": "click", "x": 10, "y": 40, "button": "left", "count": 1 }, { "t": "text", "s": "x" }]));
    assert_eq!(run(&mut w, 150, 300), Flow::Go, "the mirror keeps running");
    assert!(win.acts().is_empty());
    *win.person_has_it.lock() = false;
    link.queue(json!([{ "t": "key", "k": "tab" }]));
    run(&mut w, 350, 600);
    assert_eq!(win.acts(), vec!["key 0x9"], "the dropped events are not replayed; new ones work after the hand back");
}

#[test]
fn close_posts_once_and_only_when_opened() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut never = worker(&link, &win);
    never.close();
    assert_eq!(link.count("/v1/lend/close"), 0);
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    w.close();
    w.close();
    assert_eq!(link.count("/v1/lend/close"), 1);
    let close = link.posts.lock().iter().find(|(p, _)| p == "/v1/lend/close").unwrap().1.clone();
    assert_eq!(close, json!({ "id": "l1" }));
}

#[test]
fn waits_for_the_sandbox_and_retries_every_few_seconds() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    *link.up.lock() = false;
    let mut w = worker(&link, &win);
    run(&mut w, 0, 9000);
    assert!(!w.is_open());
    assert!(link.paths().is_empty(), "nothing is posted while it is down");
    *link.up.lock() = true;
    run(&mut w, 9050, 14_000);
    assert!(w.is_open());
    assert_eq!(link.count("/v1/lend/open"), 1);
    assert!(link.count("/v1/lend/frame") >= 1);
}

#[test]
fn reopens_after_the_sandbox_restarts() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 300);
    *link.up.lock() = false;
    run(&mut w, 350, 600);
    assert!(!w.is_open(), "a failed post means the viewer is gone");
    *link.up.lock() = true;
    run(&mut w, 650, 6000);
    assert!(w.is_open());
    assert_eq!(link.count("/v1/lend/open"), 2);
}

#[test]
fn a_vanished_window_ends_the_worker() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 300);
    *win.gone.lock() = true;
    assert_eq!(run(&mut w, 350, 800), Flow::Gone);
}

#[test]
fn notches_follow_the_browser_direction() {
    assert_eq!(notches_for(0.0), 0);
    assert_eq!(notches_for(1.0), -1);
    assert_eq!(notches_for(-3.0), 3);
    assert_eq!(notches_for(0.2), -1);
    assert_eq!(notches_for(1.0e9), -20);
}

#[test]
fn registry_starts_and_stops_mirrors_to_match_the_list() {
    let link = FakeLink::running();
    let win = FakeWin::new();
    let made = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut reg = Registry::default();
    let make = {
        let (link, win, made) = (link.clone(), win.clone(), made.clone());
        move |app: &Lent| {
            made.lock().push(app.id.clone());
            Mirror::start(Worker::new(&app.id, &app.title, link.clone(), win.clone()))
        }
    };
    let a = Lent { id: "l1".into(), title: "A".into(), hwnd: 1 };
    let b = Lent { id: "l2".into(), title: "B".into(), hwnd: 2 };
    reg.reconcile(&[a.clone()], &make);
    reg.reconcile(&[a.clone(), b.clone()], &make);
    reg.reconcile(&[a.clone(), b.clone()], &make);
    assert_eq!(*made.lock(), vec!["l1", "l2"], "started once each");
    assert_eq!(reg.ids(), vec!["l1", "l2"]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while link.count("/v1/lend/open") < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    reg.reconcile(&[b], &make);
    assert_eq!(reg.ids(), vec!["l2"]);
    // The real thread closes the viewer after it is stopped.
    while link.count("/v1/lend/close") == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(link.posts.lock().iter().any(|(p, b)| p == "/v1/lend/close" && b["id"] == "l1"), "l1 closed");
    reg.reconcile(&[], &make);
    assert!(reg.ids().is_empty());
}

#[test]
fn a_frame_refused_as_not_open_reopens_and_resends() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    *link.forget.lock() = true;
    *win.pixel.lock() = 99;
    run(&mut w, 150, 400);
    assert_eq!(link.count("/v1/lend/open"), 2, "opened again");
    assert!(w.is_open());
    let paths = link.paths();
    let last_open = paths.iter().rposition(|p| p == "/v1/lend/open").unwrap();
    assert!(paths[last_open..].iter().any(|p| p == "/v1/lend/frame"), "the frame was resent after the reopen");
}

#[test]
fn a_double_click_event_is_only_the_second_half() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    link.queue(json!([
        { "t": "click", "x": 18, "y": 41, "button": "left", "count": 1 },
        { "t": "dblclick", "x": 18, "y": 41 },
        { "t": "dblclick", "x": 18 },
    ]));
    run(&mut w, 150, 300);
    assert_eq!(win.acts(), vec!["click 10,10", "dblsecond 10,10"], "click at once, then DBLCLK only; a malformed one is skipped");
}

#[test]
fn input_while_the_person_has_the_sandbox_screen_is_theirs_and_not_dropped() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    // The lend lease says the person has the app, but the person is the one typing at the sandbox screen.
    *win.person_has_it.lock() = true;
    *win.person_screen.lock() = true;
    link.queue(json!([{ "t": "click", "x": 18, "y": 41, "button": "left", "count": 1 }, { "t": "text", "s": "hi" }]));
    run(&mut w, 150, 300);
    assert_eq!(win.acts(), vec!["P:click 10,10", "P:type hi"]);
    // Not the person at the screen: it is the bot's, so the lease drops it.
    *win.person_screen.lock() = false;
    link.queue(json!([{ "t": "key", "k": "tab" }]));
    run(&mut w, 350, 500);
    assert_eq!(win.acts().len(), 2, "bot input is dropped while the person has the app");
}

#[test]
fn applied_input_sends_a_frame_at_once_and_again_shortly_after() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    // Settle: idle cadence (500 ms), the picture is not changing.
    run(&mut w, 150, 6000);
    let before = link.count("/v1/lend/frame");
    *win.pixel.lock() = 77; // the app redraws in answer to the input
    link.queue(json!([{ "t": "key", "k": "tab" }]));
    // The poll that applies it happens at t=6100; the frame goes out in the same step or the next.
    run(&mut w, 6050, 6200);
    assert_eq!(link.count("/v1/lend/frame"), before + 1, "a frame right after the input");
    *win.pixel.lock() = 78; // the redraw finished a moment later
    run(&mut w, 6250, 6350);
    let after = link.count("/v1/lend/frame");
    assert!((before + 2..=before + 3).contains(&after), "and again about 120 ms later, then back to the normal pace: {after} vs {before}");
}

#[test]
fn frames_are_100ms_while_changing_and_500ms_when_still() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    let mut t = 0;
    let mut px = 10u8;
    while t <= 1000 {
        px = px.wrapping_add(1);
        *win.pixel.lock() = px;
        w.step(t);
        t += 50;
    }
    let changing = link.count("/v1/lend/frame");
    assert!(changing >= 8, "about 10 frames in a second while changing, got {changing}");
    run(&mut w, 1050, 9000);
    let still = link.count("/v1/lend/frame") - changing;
    assert!(still <= 14, "idle: about 2 per second plus the keep-alive, got {still}");
}

#[test]
fn big_windows_are_shrunk_to_1280_but_events_keep_window_pixels() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    *win.size.lock() = (2560, 1440);
    let mut w = worker(&link, &win);
    run(&mut w, 0, 200);
    let posts = link.posts.lock().clone();
    let open = posts.iter().find(|(p, _)| p == "/v1/lend/open").unwrap();
    assert_eq!((open.1["w"].as_u64(), open.1["h"].as_u64()), (Some(2560), Some(1440)), "open reports the window size");
    let frame = posts.iter().find(|(p, _)| p == "/v1/lend/frame").unwrap();
    assert_eq!((frame.1["w"].as_u64(), frame.1["h"].as_u64()), (Some(2560), Some(1440)), "frame w/h stay in window pixels");
    use base64::Engine;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(frame.1["jpeg_b64"].as_str().unwrap()).unwrap();
    assert_eq!(crate::potato::sandbox::control::picture_size(&jpeg), Some((1280, 720)));
}

#[test]
fn the_long_poll_asks_the_guest_to_wait_and_a_failed_poll_closes_the_viewer() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    let pump = w.pump();
    assert_eq!(pump.poll(true), Ok(Flow::Go));
    let last = link.posts.lock().iter().rfind(|(p, _)| p == "/v1/lend/events").unwrap().1.clone();
    assert_eq!(last, json!({ "id": "l1", "wait_ms": EVENTS_WAIT_MS }));
    *link.up.lock() = false;
    assert_eq!(pump.poll(true), Err(()));
    assert!(!w.is_open());
}

fn shot(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Shot {
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&f(x, y));
        }
    }
    Shot { rgba, w, h, bgra: false }
}

#[test]
fn blank_pictures_are_one_colour_or_near_black() {
    assert!(is_blank(&shot(64, 48, |_, _| [255, 255, 255, 255])), "all white");
    assert!(is_blank(&shot(64, 48, |_, _| [0, 0, 0, 0])), "all black");
    assert!(is_blank(&shot(64, 48, |x, y| [(x + y) as u8 % 5, 2, 0, 255])), "near black, not identical");
    assert!(is_blank(&shot(1, 1, |_, _| [9, 9, 9, 255])), "one pixel");
    assert!(!is_blank(&shot(64, 48, |x, _| if x < 32 { [255, 255, 255, 255] } else { [0, 0, 0, 255] })), "two colours");
    assert!(!is_blank(&shot(64, 48, |_, y| if y == 0 { [40, 40, 40, 255] } else { [20, 20, 20, 255] })), "not black and not one colour");
    assert!(!is_blank(&shot(0, 0, |_, _| [0; 4])), "no size is not blank");
    assert!(!is_blank(&Shot { rgba: vec![0; 8], w: 10, h: 10, bgra: false }), "a short buffer is not blank");
}

#[test]
fn the_worker_tells_the_session_whether_each_picture_is_paused() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    assert!(!win.reports.lock().is_empty());
    assert!(win.reports.lock().iter().all(|p| *p == Picture::Blank), "the fake paints one colour: blank");
    // minimized: nothing can be captured, and that is reported as paused
    win.reports.lock().clear();
    *win.iconic.lock() = true;
    run(&mut w, 150, 700);
    assert!(!win.reports.lock().is_empty() && win.reports.lock().iter().all(|p| *p == Picture::Minimized));
    // a failed capture of a window that is not minimized says nothing
    win.reports.lock().clear();
    *win.iconic.lock() = false;
    *win.fail.lock() = true;
    run(&mut w, 750, 1300);
    assert!(win.reports.lock().is_empty());
}

// ---- raw frames, the fallback, the change hash --------------------------------------------------------

#[test]
fn frames_go_out_as_raw_jpeg_bytes_with_headers() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 300);
    let raws = link.raws.lock().clone();
    assert_eq!(raws.len(), 1);
    let (path, headers, body) = &raws[0];
    assert_eq!(path, "/v1/lend/frame-raw");
    assert_eq!(&body[..3], &[0xFF, 0xD8, 0xFF], "the body is the JPEG itself");
    let get = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap();
    assert_eq!((get("X-Lend-Id").as_str(), get("X-Seq").as_str(), get("X-W").as_str(), get("X-H").as_str()), ("l1", "1", "16", "8"));
    let sent: u64 = get("X-Sent-Ms").parse().unwrap();
    assert!(sent > 1_600_000_000_000, "unix ms, got {sent}");
}

#[test]
fn an_old_guest_404_falls_back_to_json_for_the_session() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    *link.old_guest.lock() = true;
    let mut w = worker(&link, &win);
    run(&mut w, 0, 300);
    assert_eq!(*link.raw_tries.lock(), 1, "raw tried once");
    assert_eq!(link.count("/v1/lend/frame"), 1, "the first frame still arrived, as JSON");
    let frame = link.posts.lock().iter().find(|(p, _)| p == "/v1/lend/frame").unwrap().1.clone();
    assert!(frame.get("raw").is_none() && frame["jpeg_b64"].is_string() && frame["ts"].is_u64());
    *win.pixel.lock() = 200;
    run(&mut w, 350, 800);
    assert_eq!(*link.raw_tries.lock(), 1, "no more raw tries for this session");
    assert_eq!(link.count("/v1/lend/frame"), 2);
}

#[test]
fn raw_frames_that_the_guest_forgot_reopen_and_resend_raw() {
    let (link, win) = (FakeLink::running(), FakeWin::new());
    let mut w = worker(&link, &win);
    run(&mut w, 0, 100);
    *link.forget.lock() = true;
    *win.pixel.lock() = 99;
    run(&mut w, 150, 400);
    assert_eq!(link.count("/v1/lend/open"), 2);
    assert_eq!(link.raws.lock().len(), 2, "the refused one is not counted, the resend is");
}

#[test]
fn raw_headers_are_plain_decimal() {
    let h = raw_frame_headers("abc", 7, 1920, 1080, 1_700_000_000_123);
    let names: Vec<&str> = h.iter().map(|(k, _)| *k).collect();
    assert_eq!(names, ["X-Lend-Id", "X-Seq", "X-W", "X-H", "X-Sent-Ms"]);
    let values: Vec<&str> = h.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(values, ["abc", "7", "1920", "1080", "1700000000123"]);
}

fn flat(w: u32, h: u32) -> Shot {
    shot(w, h, |_, _| [250, 250, 250, 255])
}

#[test]
fn the_change_hash_is_stable_and_sees_size_changes() {
    assert_eq!(hash_of(&flat(640, 480)), hash_of(&flat(640, 480)));
    assert_ne!(hash_of(&flat(640, 480)), hash_of(&flat(641, 480)));
    assert_ne!(hash_of(&flat(640, 480)), hash_of(&flat(480, 640)));
    let empty = Shot { rgba: Vec::new(), w: 0, h: 0, bgra: false };
    let _ = hash_of(&empty); // no panic
}

#[test]
fn the_change_hash_sees_a_typed_character_anywhere_in_a_text_box() {
    let (pw, ph) = (160u32, 120u32);
    let base = hash_of(&flat(pw, ph));
    // A glyph is about 7 px wide and 14 px tall. Slide one over the whole picture: every position must change the hash.
    let (gw, gh) = (7u32, 14u32);
    for y in (0..ph - gh).step_by(2) {
        for x in (0..pw - gw).step_by(2) {
            let with_glyph = shot(pw, ph, |px, py| if px >= x && px < x + gw && py >= y && py < y + gh { [0, 0, 0, 255] } else { [250, 250, 250, 255] });
            assert_ne!(hash_of(&with_glyph), base, "glyph at ({x}, {y}) went unseen");
        }
    }
}

#[test]
fn the_change_hash_reads_far_less_than_every_byte() {
    // A one-pixel change off the sampled grid may slip through (the forced frame after input and the keep-alive cover it);
    // that is the price of not hashing 8 MB per look.
    let a = flat(640, 480);
    let b = shot(640, 480, |x, y| if x == 1 && y == 1 { [0, 0, 0, 255] } else { [250, 250, 250, 255] });
    assert_eq!(hash_of(&a), hash_of(&b));
}
