//! Shared test rig for the phone server: a fake `Env`, a pairing store in a temp folder, and request helpers.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use super::gate::{dispatch, Changes, Ctx, Env};
use super::http::{Request, Response};
use super::net::Hosts;
use super::rate::Limiter;
use crate::mobile_pairing::{Caps, MobilePairingStore, SharedPairingStore};
use crate::potato::hold::Hold;
use crate::potato::service::Service;

pub struct TestEnv {
    pub service: Option<Arc<Service>>,
    pub groups: AtomicBool,
    pub held: AtomicBool,
    pub hosts: Mutex<Hosts>,
    pub candidates: Mutex<Vec<String>>,
}

impl Env for TestEnv {
    fn service(&self) -> Option<Arc<Service>> {
        self.service.clone()
    }
    fn groups_on(&self) -> bool {
        self.groups.load(Ordering::SeqCst)
    }
    fn pc_name(&self) -> String {
        "TEST-PC".into()
    }
    fn candidates(&self) -> Vec<String> {
        self.candidates.lock().clone()
    }
    fn hosts(&self) -> Hosts {
        self.hosts.lock().clone()
    }
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
    }
    fn pause_all(&self) -> Hold {
        self.held.store(true, Ordering::SeqCst);
        Hold { held: true, ..Hold::default() }
    }
    fn resume_all(&self) -> Result<Hold, String> {
        self.held.store(false, Ordering::SeqCst);
        Ok(Hold::default())
    }
    fn paused(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }
}

pub struct Kit {
    pub ctx: Arc<Ctx>,
    pub env: Arc<TestEnv>,
    pub pairing: SharedPairingStore,
    pub dir: PathBuf,
}

pub const LAN: &str = "192.168.1.5:1420";

pub fn kit(name: &str, service: Option<Arc<Service>>) -> Kit {
    let dir = std::env::temp_dir().join(format!("pm-phone-kit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let assets = dir.join("app");
    std::fs::create_dir_all(assets.join("tools")).unwrap();
    std::fs::write(assets.join("index.html"), "<!doctype html><title>Potato</title>").unwrap();
    std::fs::write(assets.join("app.js"), "export {}").unwrap();
    std::fs::write(assets.join("sw.js"), "// sw").unwrap();
    std::fs::write(assets.join("tools").join("make.mjs"), "secret tool").unwrap();
    std::fs::write(assets.join(".hidden.js"), "hidden").unwrap();
    std::fs::write(dir.join("secret.js"), "outside").unwrap();
    let pairing: SharedPairingStore = Arc::new(Mutex::new(MobilePairingStore::load_or_create(dir.join("pairing.json")).unwrap()));
    let env = Arc::new(TestEnv {
        service,
        groups: AtomicBool::new(true),
        held: AtomicBool::new(false),
        hosts: Mutex::new(Hosts { machine: Some("test-pc".into()), ..Hosts::default() }),
        candidates: Mutex::new(vec!["http://192.168.1.5:1420".into()]),
    });
    let ctx = Arc::new(Ctx { pairing: pairing.clone(), env: env.clone(), limiter: Limiter::default(), changes: Arc::new(Changes::new()), assets: Some(assets) });
    Kit { ctx, env, pairing, dir }
}

pub fn req(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Request {
    let (path, query) = path.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((path.to_string(), String::new()));
    let mut hs: Vec<(String, String)> = vec![("host".into(), LAN.into())];
    if method != "GET" {
        hs.push(("x-potato-phone".into(), "1".into()));
    }
    for (k, v) in headers {
        hs.retain(|(name, _)| name != &k.to_ascii_lowercase());
        hs.push((k.to_ascii_lowercase(), v.to_string()));
    }
    Request { method: method.into(), path, query, headers: hs, body: body.as_bytes().to_vec(), peer: IpAddr::from([192, 168, 1, 9]) }
}

impl Kit {
    pub fn call(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Response {
        dispatch(&self.ctx, &req(method, path, headers, body))
    }

    /// A paired phone with every ability: its id and bearer token.
    pub fn pair(&self) -> (String, String) {
        self.pair_with(Caps::all())
    }

    pub fn pair_with(&self, caps: Caps) -> (String, String) {
        let mut s = self.pairing.lock();
        let code = s.mint_phone_code("http://x".into(), caps, None).unwrap();
        let g = s.redeem_phone_code(&code.code, "Test phone").unwrap();
        (g.device_id, g.token)
    }

    pub fn authed(&self, token: &str, method: &str, path: &str, body: &str) -> Response {
        let bearer = format!("Bearer {token}");
        self.call(method, path, &[("authorization", bearer.as_str())], body)
    }
}
