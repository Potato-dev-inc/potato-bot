//! Live check against a real Notepad window on this PC. Run by hand only (`#[ignore]`), never in normal test runs.
//!
//! Needs `LEND_LIVE_HWND` (decimal window handle of an open Notepad) and `LEND_LIVE_OUT` (folder for the PNGs).
//! Nothing here activates, focuses or sends input to any window other than the lent one's own input layer.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use serde_json::json;

use crate::potato::lend::{a11y, capture};
use crate::potato::lend::session::{hub, Mode};
use crate::potato::lend::Hwnd;
use crate::potato::lend_tools::{centre, is_password_field, observe_json, ObsElem};
use crate::potato::live;

fn step(msg: &str) {
    println!("[live] {msg}");
}

fn obs(e: &a11y::Element) -> ObsElem {
    ObsElem {
        r#ref: e.r#ref.clone(),
        role: e.role.clone(),
        name: e.name.clone(),
        value: e.value.clone(),
        rect: (e.rect.x, e.rect.y, e.rect.w, e.rect.h),
    }
}

/// The text area: the first edit or document control that is not a password box.
fn editor_of(snap: &a11y::Snapshot) -> Option<a11y::Element> {
    snap.elements.iter().find(|e| (e.role == "edit" || e.role == "document") && !is_password_field(&obs(e))).cloned()
}

fn editor_value(snap: &a11y::Snapshot) -> String {
    editor_of(snap).and_then(|e| e.value).unwrap_or_default()
}

/// Takes the lent app back when the test ends, even if a step panicked.
struct BringBack<'a>(&'a str);

impl Drop for BringBack<'_> {
    fn drop(&mut self) {
        let _ = hub().bring_back(self.0);
    }
}

#[test]
#[ignore = "live: needs a real Notepad window; set LEND_LIVE_HWND and LEND_LIVE_OUT and run with --ignored --nocapture"]
fn live_lend_notepad() {
    let Some(hwnd) = std::env::var("LEND_LIVE_HWND").ok().and_then(|s| s.trim().parse::<Hwnd>().ok()) else {
        println!("[live] skipped: set LEND_LIVE_HWND to the decimal handle of an open Notepad window");
        return;
    };
    let Some(out) = std::env::var("LEND_LIVE_OUT").ok().map(PathBuf::from) else {
        println!("[live] skipped: set LEND_LIVE_OUT to a folder for the PNGs");
        return;
    };
    std::fs::create_dir_all(&out).expect("make the output folder");
    let hub = hub();

    step("turn on the Lend an App Labs switch (through the settings path)");
    live::apply_settings(&json!({ live::LEND_LABS_KEY: true }));
    assert!(live::lend_labs_on(), "the Labs switch did not turn on");

    step(&format!("lend window {hwnd} visible"));
    let view = hub.lend(hwnd, Mode::Visible).expect("lend the window");
    let id = view.id.clone();
    let _back = BringBack(&id);
    println!("[live] view: {view:?}");
    sleep(Duration::from_secs(1));

    step("observe");
    let snap = hub.snapshot(&id).expect("snapshot");
    let elems: Vec<ObsElem> = snap.elements.iter().map(obs).collect();
    println!("[live] elements: {}", elems.len());
    for e in elems.iter().take(10) {
        println!("[live]   ({}, {})", e.role, e.name);
    }
    let png = hub.capture(&id).expect("capture");
    let size = crate::potato::sandbox::control::picture_size(&png);
    println!("[live] picture size: {size:?}");
    let _controls = observe_json(&view, &elems, snap.truncated, size);
    std::fs::write(out.join("1-visible.jpg"), &png).expect("save 1-visible.jpg");

    step("type \" lend live test\" into the editor (by ref)");
    let editor = editor_of(&snap).expect("no edit or document control in the snapshot: is the Notepad text area showing?");
    println!("[live] editor ref {} ({})", editor.r#ref, editor.role);
    let (cx, cy) = centre(&obs(&editor));
    hub.act_click(&id, cx, cy).expect("click the editor");
    hub.act_type(&id, " lend live test").expect("type");
    sleep(Duration::from_millis(500));
    let snap2 = hub.snapshot(&id).expect("snapshot after typing");
    println!("[live] editor value after typing: {:?}", editor_value(&snap2));
    let png2 = hub.capture(&id).expect("capture typed");
    std::fs::write(out.join("2-typed.jpg"), &png2).expect("save 2-typed.jpg");

    step("hide the window (see-through, click-through)");
    hub.set_mode(&id, Mode::Hidden).expect("hide");
    sleep(Duration::from_millis(500));
    hub.act_type(&id, " hidden").expect("type while hidden");
    sleep(Duration::from_millis(500));
    let png3 = hub.capture(&id).expect("capture hidden");
    std::fs::write(out.join("3-hidden.jpg"), &png3).expect("save 3-hidden.jpg");
    let snap3 = hub.snapshot(&id).expect("snapshot while hidden");
    let value = editor_value(&snap3);
    println!("[live] editor value while hidden: {value:?}");

    step("show again and bring the window back");
    hub.set_mode(&id, Mode::Visible).expect("show again");
    hub.bring_back(&id).expect("bring back");
    let left = hub.list();
    println!("[live] lent apps left: {}", left.len());

    assert!(value.contains("lend live test"), "editor value lacks \"lend live test\": {value:?}");
    assert!(value.contains("hidden"), "editor value lacks \"hidden\": {value:?}");
    assert!(left.is_empty(), "lent apps still listed at the end: {left:?}");
    step("done");
}

#[test]
#[ignore = "live: needs a real window on a secondary monitor; set LEND_LIVE_HWND and run with --ignored --nocapture"]
fn live_border_follows() {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };
    use crate::potato::lend::border::Border;

    let Some(target) = std::env::var("LEND_LIVE_HWND").ok().and_then(|s| s.trim().parse::<Hwnd>().ok()) else {
        println!("[live] skipped: set LEND_LIVE_HWND to the decimal handle of a window on a secondary monitor");
        return;
    };
    let h = |v: Hwnd| HWND(v as *mut std::ffi::c_void);
    let rect = |v: Hwnd| {
        let mut r = RECT::default();
        unsafe { GetWindowRect(h(v), &mut r) }.expect("GetWindowRect");
        r
    };
    let mv = |dx: i32, dy: i32| {
        let r = rect(target);
        unsafe { SetWindowPos(h(target), None, r.left + dx, r.top + dy, 0, 0, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE) }
            .expect("move target");
    };

    let border = Border::start(target, "Live test".into()).expect("start border");
    sleep(Duration::from_millis(400));
    let frame = border.frame_hwnd();
    assert_ne!(frame, 0, "no frame overlay window");
    let before = rect(frame);

    mv(60, 40);
    sleep(Duration::from_millis(400));
    let after = rect(frame);
    mv(-60, -40);
    sleep(Duration::from_millis(400));
    let back = rect(frame);
    border.stop();

    assert_eq!((after.left - before.left, after.top - before.top), (60, 40), "frame did not follow the move: {before:?} -> {after:?}");
    assert_eq!((back.left, back.top), (before.left, before.top), "frame did not follow the move back: {back:?}");
}

fn median_ms(label: &str, runs: usize, mut f: impl FnMut() -> String) {
    let mut times = Vec::new();
    let mut note = String::new();
    for _ in 0..runs {
        let t = std::time::Instant::now();
        note = f();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("[latency] {label:<42} median {:>7.1} ms  (min {:.1}, max {:.1})  {note}", times[times.len() / 2], times[0], times[times.len() - 1]);
}

/// Times each stage of looking at a lent window, in this build profile (run it in the same profile the app runs in).
#[test]
#[ignore = "live: needs a real window; set LEND_LIVE_HWND and run with --ignored --nocapture"]
fn live_latency() {
    let Some(hwnd) = std::env::var("LEND_LIVE_HWND").ok().and_then(|s| s.trim().parse::<Hwnd>().ok()) else {
        println!("[latency] skipped: set LEND_LIVE_HWND to the decimal handle of the window to time");
        return;
    };
    const RUNS: usize = 5;
    println!("[latency] profile: {}", if cfg!(debug_assertions) { "debug" } else { "release" });
    median_ms("(a) capture::png (old observe picture)", RUNS, || {
        let (png, w, h) = capture::png(hwnd).expect("png");
        format!("{w}x{h} {} KB", png.len() / 1024)
    });
    median_ms("(a2) capture::jpeg_scaled (new observe picture)", RUNS, || {
        let (jpeg, w, h, ww, wh) = capture::jpeg_scaled(hwnd, capture::OBSERVE_MAX_SIDE, capture::OBSERVE_JPEG_QUALITY).expect("jpeg");
        format!("{w}x{h} of {ww}x{wh}, {} KB", jpeg.len() / 1024)
    });
    median_ms("(b) mirror: capture raw + hash + scale + JPEG", RUNS, || {
        use crate::potato::lend::mirror::{encode_jpeg, Shot};
        use std::hash::{Hash, Hasher};
        let (px, w, h) = capture::bgra(hwnd).expect("bgra");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        px.hash(&mut hasher);
        let _ = hasher.finish();
        let (jpeg, jw, jh) = encode_jpeg(&Shot { rgba: px, w, h, bgra: true }).expect("jpeg");
        format!("{w}x{h} -> {jw}x{jh}, {} KB", jpeg.len() / 1024)
    });
    median_ms("(b2) mirror: capture::rgba only (old)", RUNS, || {
        let (_, w, h) = capture::rgba(hwnd).expect("rgba");
        format!("{w}x{h}")
    });
    median_ms("(c) a11y::snapshot (400 nodes, 3 s cap)", RUNS, || {
        let snap = a11y::snapshot(hwnd, 400, 3000).expect("snapshot");
        format!("{} elements, truncated {}", snap.elements.len(), snap.truncated)
    });

    // (d) the whole app_observe path: lend, then snapshot + capture through the hub, as lend_tools does.
    live::apply_settings(&json!({ live::LEND_LABS_KEY: true }));
    let hub = hub();
    let view = hub.lend(hwnd, Mode::Visible).expect("lend the window");
    let id = view.id.clone();
    let _back = BringBack(&id);
    sleep(Duration::from_millis(500));
    median_ms("(d) app_observe path: hub.snapshot + hub.capture", RUNS, || {
        let snap = hub.snapshot(&id).expect("snapshot");
        let jpeg = hub.capture(&id).expect("capture");
        let elems: Vec<ObsElem> = snap.elements.iter().map(obs).collect();
        let size = crate::potato::sandbox::control::picture_size(&jpeg);
        let json = observe_json(&view, &elems, snap.truncated, size);
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        format!("{} elements, answer {} KB + picture {} KB", elems.len(), json.to_string().len() / 1024, b64.len() / 1024)
    });
}

#[test]
#[ignore = "live: needs a real window; set LEND_LIVE_HWND and LEND_LIVE_OUT and run with --ignored --nocapture"]
fn live_minimize_hides() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowLongPtrW, IsIconic, ShowWindow, GWL_EXSTYLE, SW_SHOWMINNOACTIVE, WS_EX_TOPMOST,
    };
    use crate::potato::lend::mirror::{is_blank, Shot};

    let Some(target) = std::env::var("LEND_LIVE_HWND").ok().and_then(|s| s.trim().parse::<Hwnd>().ok()) else {
        println!("[live] skipped: set LEND_LIVE_HWND to the decimal handle of an open window");
        return;
    };
    let Some(out) = std::env::var("LEND_LIVE_OUT").ok().map(PathBuf::from) else {
        println!("[live] skipped: set LEND_LIVE_OUT to a folder for the PNGs");
        return;
    };
    let h = HWND(target as *mut std::ffi::c_void);
    let iconic = || unsafe { IsIconic(h) }.as_bool();
    let topmost = || (unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) } as u32) & WS_EX_TOPMOST.0 != 0;
    let ex = || unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) };
    let fg_before = unsafe { GetForegroundWindow() }.0 as isize;
    let was_topmost = topmost();
    let ex_before = ex();

    let v = hub().lend(target, Mode::Visible).expect("lend");
    let _guard = BringBack(&v.id);
    sleep(Duration::from_millis(500));

    step("minimizing without activating");
    let _ = unsafe { ShowWindow(h, SW_SHOWMINNOACTIVE) };
    let mut waited = 0;
    let hidden = || hub().view(&v.id).map(|x| x.mode == Mode::Hidden).unwrap_or(false);
    while (iconic() || !hidden()) && waited < 2000 {
        hub().tick(); // what the app's keeper thread does every 100 ms: border events become session changes
        sleep(Duration::from_millis(50));
        waited += 50;
    }
    assert!(!iconic(), "still minimized after {waited} ms");
    assert_eq!(hub().view(&v.id).unwrap().mode, Mode::Hidden, "not hidden after minimize");
    assert!(topmost(), "a hidden app must be always on top");
    let (bgra, w, ht) = capture::bgra(target).expect("capture");
    assert!(!is_blank(&Shot { rgba: bgra, w, h: ht, bgra: true }), "the picture is blank");
    let (png, ..) = capture::png(target).expect("png");
    std::fs::create_dir_all(&out).expect("out dir");
    std::fs::write(out.join("minimize-hidden.png"), png).expect("write png");

    step("bringing back");
    hub().bring_back(&v.id).expect("bring back");
    sleep(Duration::from_millis(200));
    assert_eq!(ex(), ex_before, "style not restored");
    assert_eq!(topmost(), was_topmost, "topmost not restored");
    assert_eq!(unsafe { GetForegroundWindow() }.0 as isize, fg_before, "the foreground window changed");
    step("done");
}
