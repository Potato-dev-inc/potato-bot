//! The app's one phone server: the saved setting, the listener, the tunnel, and what the Settings page shows.
//!
//! `Phone` holds everything and has no Tauri types, so tests build their own (`Phone::new`). `start(app)` is the
//! one place the real app wires it: where the app files and the tunnel program are, how to read the Labs switch for
//! groups, and how to tell the Settings page that something changed (`phone://changed`).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use serde::Serialize;

use super::config::{self, Config};
use super::firewall;
use super::gate::{Changes, Ctx, Env};
use super::http::{self, Server};
use super::net::{self, Hosts};
use super::rate::Limiter;
use super::tunnel::{Deps, TunnelManager, TunnelStatus};
use crate::mobile_pairing::{Caps, SharedPairingStore};

static CHANGES: OnceLock<Arc<Changes>> = OnceLock::new();
static PHONE: OnceLock<Arc<Phone>> = OnceLock::new();

/// The counter the engine bumps (`Host::changed`) so phones waiting on `/changes` wake up.
pub fn changes() -> Arc<Changes> {
    CHANGES.get_or_init(|| Arc::new(Changes::new())).clone()
}

pub fn phone() -> Option<Arc<Phone>> {
    PHONE.get().cloned()
}

fn machine_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "your PC".to_string())
}

/// The real `Env`: the engine's service, the saved setting and the tunnel.
pub struct LiveEnv {
    root: PathBuf,
    /// The port the listener really holds (0: none).
    bound: Arc<AtomicU16>,
    config: Arc<RwLock<Config>>,
    tunnel: Arc<TunnelManager>,
    groups: Arc<dyn Fn() -> bool + Send + Sync>,
    push: Mutex<Option<Arc<crate::push::hub::PushHub>>>,
    lan: Mutex<Option<(Instant, Vec<Ipv4Addr>)>>,
    service: Box<dyn Fn() -> Option<Arc<crate::potato::service::Service>> + Send + Sync>,
}

impl LiveEnv {
    fn lan_ips(&self) -> Vec<Ipv4Addr> {
        let mut cache = self.lan.lock();
        if let Some((at, ips)) = cache.as_ref() {
            if at.elapsed() < Duration::from_secs(10) {
                return ips.clone();
            }
        }
        let ips = net::lan_ipv4s();
        *cache = Some((Instant::now(), ips.clone()));
        ips
    }

    fn lan_origins(&self) -> Vec<String> {
        net::lan_origins(&self.lan_ips(), self.live_port())
    }

    fn live_port(&self) -> u16 {
        match self.bound.load(Ordering::SeqCst) {
            0 => self.config.read().port,
            port => port,
        }
    }

    fn custom_origin(&self) -> Option<String> {
        self.config.read().custom_address.clone()
    }

    fn tunnel_url(&self) -> Option<String> {
        self.tunnel.status().url
    }
}

impl Env for LiveEnv {
    fn service(&self) -> Option<Arc<crate::potato::service::Service>> {
        (self.service)()
    }
    fn groups_on(&self) -> bool {
        (self.groups)()
    }
    fn pc_name(&self) -> String {
        machine_name()
    }
    fn candidates(&self) -> Vec<String> {
        let mut all = self.lan_origins();
        all.extend(self.custom_origin());
        all.extend(self.tunnel_url());
        all
    }
    fn hosts(&self) -> Hosts {
        let host_of = |origin: Option<String>| origin.and_then(|o| o.split_once("://").map(|(_, a)| net::host_only(a)));
        let tunnel = self.tunnel.status();
        Hosts {
            custom: host_of(self.custom_origin()),
            tunnel_starting: tunnel.on && tunnel.url.is_none(),
            tunnel: host_of(tunnel.url),
            machine: Some(machine_name().to_ascii_lowercase()),
        }
    }
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }
    fn pause_all(&self) -> crate::potato::hold::Hold {
        crate::potato::live::pause_all(Duration::from_millis(300))
    }
    fn resume_all(&self) -> Result<crate::potato::hold::Hold, String> {
        crate::potato::hold::resume(&self.root).map_err(|e| format!("Could not start the bots again: {e}"))
    }
    fn paused(&self) -> bool {
        crate::potato::hold::is_held(&self.root)
    }
    fn push(&self) -> Option<Arc<crate::push::hub::PushHub>> {
        self.push.lock().clone()
    }
}

// ---- what Settings shows -----------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOut {
    pub id: String,
    pub name: String,
    pub paired_at_secs: u64,
    pub last_seen_secs: Option<u64>,
    pub expired: bool,
    pub caps: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneStatus {
    pub enabled: bool,
    pub port: u16,
    pub port_error: Option<String>,
    pub pc_name: String,
    pub lan_urls: Vec<String>,
    pub custom_address: Option<String>,
    pub custom_warning: Option<String>,
    pub tunnel: TunnelStatus,
    pub firewall: firewall::Status,
    pub devices: Vec<DeviceOut>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PairingOut {
    pub code: String,
    pub expires_at_secs: u64,
    pub qr_url: String,
    pub candidates: Vec<String>,
    pub via: String,
}

struct Inner {
    server: Option<Server>,
    port_error: Option<String>,
    /// The last firewall reading and when (reading runs a program; the Settings page asks about once a second).
    firewall: Option<(Instant, u16, firewall::Status)>,
}

pub struct Phone {
    data: PathBuf,
    config: Arc<RwLock<Config>>,
    inner: Mutex<Inner>,
    pub ctx: Arc<Ctx>,
    env: Arc<LiveEnv>,
    pub tunnel: Arc<TunnelManager>,
    pairing: SharedPairingStore,
    notify: Arc<dyn Fn() + Send + Sync>,
    bind: IpAddr,
    started: AtomicU64,
}

pub struct Wiring {
    pub data: PathBuf,
    pub assets: Option<PathBuf>,
    pub tunnel_exe: Option<PathBuf>,
    pub tunnel_deps: Deps,
    pub groups: Arc<dyn Fn() -> bool + Send + Sync>,
    pub notify: Arc<dyn Fn() + Send + Sync>,
    pub pairing: SharedPairingStore,
    pub changes: Arc<Changes>,
    pub bind: IpAddr,
    pub service: Box<dyn Fn() -> Option<Arc<crate::potato::service::Service>> + Send + Sync>,
    pub config: Config,
}

impl Phone {
    pub fn new(w: Wiring) -> Arc<Phone> {
        let config = Arc::new(RwLock::new(w.config));
        let tunnel = Arc::new(TunnelManager::new(w.data.clone(), w.tunnel_exe, w.tunnel_deps));
        let env = Arc::new(LiveEnv {
            root: w.data.join("potato"),
            bound: Arc::new(AtomicU16::new(0)),
            config: config.clone(),
            tunnel: tunnel.clone(),
            groups: w.groups,
            push: Mutex::new(None),
            lan: Mutex::new(None),
            service: w.service,
        });
        let ctx = Arc::new(Ctx { pairing: w.pairing.clone(), env: env.clone(), limiter: Limiter::default(), changes: w.changes, assets: w.assets });
        let phone = Arc::new(Phone {
            data: w.data,
            config,
            inner: Mutex::new(Inner { server: None, port_error: None, firewall: None }),
            ctx,
            env,
            tunnel,
            pairing: w.pairing,
            notify: w.notify,
            bind: w.bind,
            started: AtomicU64::new(0),
        });
        let notify = phone.notify.clone();
        phone.tunnel.on_url(move |_| notify());
        phone
    }

    pub fn set_push(&self, hub: Arc<crate::push::hub::PushHub>) {
        *self.env.push.lock() = Some(hub);
    }

    pub fn push_hub(&self) -> Option<Arc<crate::push::hub::PushHub>> {
        self.env.push()
    }

    fn port(&self) -> u16 {
        self.config.read().port
    }

    /// The port the listener really holds (differs from the setting only when the setting is 0, in tests).
    pub fn bound_port(&self) -> Option<u16> {
        self.inner.lock().server.as_ref().map(|s| s.addr.port())
    }

    /// Binds the port (when the phone server is on), reattaches the tunnel, and runs the tunnel's supervisor.
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(1, Ordering::SeqCst) == 1 {
            return;
        }
        if self.config.read().enabled {
            self.bind_server();
        }
        if self.env.push().is_some() {
            let (env, changes) = (self.env.clone() as Arc<dyn Env>, self.ctx.changes.clone());
            let _ = std::thread::Builder::new().name("phone-alerts".into()).spawn(move || super::notify::run(env, changes));
        }
        self.tunnel.on_app_start(self.bound_port().unwrap_or_else(|| self.port()));
        let tunnel = self.tunnel.clone();
        let _ = std::thread::Builder::new().name("phone-tunnel".into()).spawn(move || tunnel.run_supervisor());
    }

    fn bind_server(&self) {
        let port = self.port();
        match http::start(self.bind, port, self.ctx.clone()) {
            Ok(server) => {
                self.env.bound.store(server.addr.port(), Ordering::SeqCst);
                let mut inner = self.inner.lock();
                inner.server = Some(server);
                inner.port_error = None;
            }
            Err(e) => {
                tracing::warn!(%e, "the phone server could not start");
                self.inner.lock().port_error = Some(e);
            }
        }
    }

    #[cfg(test)]
    pub fn stop(&self) {
        if let Some(server) = self.inner.lock().server.take() {
            server.stop();
        }
        self.env.bound.store(0, Ordering::SeqCst);
        self.tunnel.shutdown_supervisor();
    }

    /// Changes the port. Only the phone server restarts; the tunnel follows. A port that is taken leaves the old one running.
    pub fn set_port(&self, port: u16) -> Result<(), String> {
        if port < 1024 {
            return Err("PHONE_PORT_IN_USE: Pick a port from 1024 to 65535.".to_string());
        }
        if port == self.port() && self.inner.lock().server.is_some() {
            return Ok(());
        }
        // The new port is bound first: if it is taken, the old server never stopped.
        match http::start(self.bind, port, self.ctx.clone()) {
            Ok(server) => {
                self.env.bound.store(server.addr.port(), Ordering::SeqCst);
                let old_server = {
                    let mut inner = self.inner.lock();
                    inner.port_error = None;
                    inner.server.replace(server)
                };
                if let Some(s) = old_server {
                    s.stop();
                }
                let mut cfg = self.config.write();
                cfg.port = port;
                let saved = config::save(&self.data, &cfg);
                drop(cfg);
                self.inner.lock().firewall = None;
                self.tunnel.set_port(port);
                (self.notify)();
                saved
            }
            Err(e) => Err(e),
        }
    }

    pub fn set_custom_address(&self, raw: Option<String>) -> Result<(), String> {
        let address = match raw.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(text) => Some(net::normalize_custom_address(text)?.0),
            None => None,
        };
        let mut cfg = self.config.write();
        cfg.custom_address = address;
        let saved = config::save(&self.data, &cfg);
        drop(cfg);
        (self.notify)();
        saved
    }

    pub fn set_tunnel(&self, on: bool) -> Result<(), String> {
        let port = self.bound_port().unwrap_or_else(|| self.port());
        let result = self.tunnel.set_on(on, port).map_err(|e| format!("PHONE_TUNNEL: {e}"));
        (self.notify)();
        result
    }

    fn firewall_status(&self, port: u16) -> firewall::Status {
        if let Some((at, p, status)) = self.inner.lock().firewall.clone() {
            if p == port && at.elapsed() < Duration::from_secs(8) {
                return status;
            }
        }
        let status = firewall::status(port);
        self.inner.lock().firewall = Some((Instant::now(), port, status.clone()));
        status
    }

    pub fn firewall_allow(&self) -> Result<(), String> {
        let result = firewall::allow(self.port());
        self.inner.lock().firewall = None;
        (self.notify)();
        result
    }

    pub fn devices(&self) -> Vec<DeviceOut> {
        self.pairing
            .lock()
            .phone_devices()
            .into_iter()
            .map(|d| DeviceOut {
                id: d.id,
                name: d.name,
                paired_at_secs: d.paired_at,
                last_seen_secs: d.last_seen,
                expired: d.expired,
                caps: d.caps.iter().map(|c| c.as_str().to_string()).collect(),
            })
            .collect()
    }

    pub fn revoke(&self, device_id: &str) -> Result<bool, String> {
        let revoked = self.pairing.lock().revoke_phone_device(device_id)?;
        if let Some(hub) = self.push_hub() {
            let _ = hub.store().lock().remove(device_id);
        }
        self.ctx.changes.bump();
        (self.notify)();
        Ok(revoked)
    }

    pub fn status(&self) -> PhoneStatus {
        let cfg = self.config.read().clone();
        let (server_port, port_error) = {
            let inner = self.inner.lock();
            (inner.server.as_ref().map(|s| s.addr.port()), inner.port_error.clone())
        };
        let port = server_port.unwrap_or(cfg.port);
        PhoneStatus {
            enabled: cfg.enabled,
            port,
            port_error,
            pc_name: machine_name(),
            lan_urls: net::lan_origins(&self.env.lan_ips(), port),
            custom_warning: cfg.custom_address.as_deref().and_then(|a| net::normalize_custom_address(a).ok()).and_then(|(_, w)| w),
            custom_address: cfg.custom_address,
            tunnel: self.tunnel.status(),
            firewall: self.firewall_status(port),
            devices: self.devices(),
        }
    }

    /// A pairing code and the QR address that carries every place the phone can try. `via` picks which address the
    /// QR opens (`lan`, `custom`, `tunnel`); the rest ride along in the fragment as fallbacks.
    pub fn start_pairing(&self, via: Option<&str>, relink: Option<String>) -> Result<PairingOut, String> {
        let candidates = self.env.candidates();
        let lan = self.env.lan_origins();
        let custom = self.env.custom_origin();
        let tunnel = self.env.tunnel_url();
        let via = via.unwrap_or(if !lan.is_empty() { "lan" } else if custom.is_some() { "custom" } else { "tunnel" });
        let primary = match via {
            "lan" => lan.first().cloned(),
            "custom" => custom,
            "tunnel" => tunnel,
            _ => None,
        }
        .ok_or_else(|| "PHONE_NO_ADDRESS: There is no address for that yet. Turn on Reach from outside your Wi-Fi, or set your own address.".to_string())?;
        let code = self.pairing.lock().mint_phone_code(primary.clone(), Caps::all(), relink)?;
        let mut url = format!("{primary}/#/pair/{}", code.code);
        let mut sep = '?';
        for c in &candidates {
            url.push(sep);
            sep = '&';
            url.push_str("a=");
            url.push_str(&urlencode(c));
        }
        crate::audit::record(crate::audit::Kind::Pairing, crate::audit::Who::Desktop, "You made a pairing code for a phone", serde_json::json!({ "via": via }));
        Ok(PairingOut { code: code.code, expires_at_secs: code.expires_at, qr_url: url, candidates, via: via.to_string() })
    }

}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

// ---- the real app ---------------------------------------------------------------------------------------------

/// Builds and starts the app's phone server. Called once, after the pairing store exists.
#[cfg(not(test))]
pub fn start(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    let Some(pairing) = crate::mobile_pairing::pairing_store() else {
        return Err("the pairing store is not ready".to_string());
    };
    let data = crate::app_paths::app_data_dir();
    let assets = crate::potato::resource_dirs::installed(app, "phone", "index.html").or_else(|| {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("phone");
        (cfg!(debug_assertions) && dev.join("index.html").exists()).then_some(dev)
    });
    let tunnel_exe = crate::potato::resource_dirs::installed(app, "bin", "cloudflared.exe").map(|d| d.join("cloudflared.exe"));
    let groups_app = app.clone();
    let notify_app = app.clone();
    let phone = Phone::new(Wiring {
        config: config::load(&data),
        data,
        assets,
        tunnel_exe,
        tunnel_deps: Deps::real(),
        groups: Arc::new(move || crate::potato::live::groups_labs_on(&crate::settings_store::read_public_settings(&groups_app))),
        notify: Arc::new(move || {
            let _ = notify_app.emit("phone://changed", serde_json::json!({}));
        }),
        pairing,
        changes: changes(),
        bind: IpAddr::from([0, 0, 0, 0]),
        service: Box::new(crate::potato::live::service),
    });
    if let Some(hub) = crate::push::hub() {
        phone.set_push(hub);
    }
    Ok(attach(phone))
}

#[cfg(not(test))]
fn attach(phone: Arc<Phone>) {
    if PHONE.set(phone.clone()).is_ok() {
        phone.start();
        probe_if_asked(phone);
    }
}

/// With `PM_PHONE_PROBE=<file>` set: once the phone server holds its port, makes one pairing code for this PC's own loopback
/// address and writes `{port, code}` to the file. It is how a packaged build is shown to answer, refuse an unpaired request and
/// accept a code once, without a person scanning a QR. Off unless the variable is set; it adds no way in for anyone else.
#[cfg(not(test))]
fn probe_if_asked(phone: Arc<Phone>) {
    let Some(file) = std::env::var_os("PM_PHONE_PROBE").filter(|f| !f.is_empty()) else { return };
    let _ = std::thread::Builder::new().name("phone-probe".into()).spawn(move || {
        for _ in 0..300 {
            if let Some(port) = phone.bound_port() {
                let minted = phone.pairing.lock().mint_phone_code(format!("http://127.0.0.1:{port}"), Caps::all(), None);
                if let Ok(code) = minted {
                    let _ = std::fs::write(&file, serde_json::json!({ "port": port, "code": code.code }).to_string());
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

/// Unit tests never start the app's phone server (it needs the windowing runtime).
#[cfg(test)]
pub fn start(_app: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobile_pairing::MobilePairingStore;
    use std::sync::atomic::AtomicUsize;

    fn phone(name: &str) -> (Arc<Phone>, Arc<AtomicUsize>, PathBuf) {
        let data = std::env::temp_dir().join(format!("pm-phone-live-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let pairing: SharedPairingStore = Arc::new(Mutex::new(MobilePairingStore::load_or_create(data.join("pairing.json")).unwrap()));
        let pings = Arc::new(AtomicUsize::new(0));
        let counter = pings.clone();
        let phone = Phone::new(Wiring {
            data: data.clone(),
            assets: None,
            tunnel_exe: None,
            tunnel_deps: Deps::real(),
            groups: Arc::new(|| false),
            notify: Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            pairing,
            changes: Arc::new(Changes::new()),
            bind: IpAddr::from([127, 0, 0, 1]),
            service: Box::new(|| None),
            config: Config { enabled: true, port: 0, custom_address: None },
        });
        (phone, pings, data)
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
    }

    fn health(port: u16) -> bool {
        use std::io::{Read, Write};
        let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return false };
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = s.write_all(format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes());
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out.contains("potato-phone")
    }

    #[test]
    fn the_qr_carries_every_address_in_its_fragment_and_the_code_is_not_among_them() {
        let (p, _, _) = phone("qr");
        assert!(p.start_pairing(Some("tunnel"), None).unwrap_err().starts_with("PHONE_NO_ADDRESS:"), "no tunnel yet");
        p.set_custom_address(Some("https://bot.example.com".into())).unwrap();
        let out = p.start_pairing(Some("custom"), None).unwrap();
        assert!(out.qr_url.starts_with(&format!("https://bot.example.com/#/pair/{}?a=", out.code)), "{}", out.qr_url);
        assert!(out.candidates.contains(&"https://bot.example.com".to_string()));
        assert!(out.qr_url.contains("a=https%3A%2F%2Fbot.example.com"));
        assert!(out.candidates.iter().all(|c| !c.contains(&out.code)), "no address carries the code");
        // The custom address comes before the tunnel in the list.
        let custom_at = out.candidates.iter().position(|c| c == "https://bot.example.com").unwrap();
        assert!(out.candidates[custom_at..].iter().all(|c| !c.contains("lan")));
        assert!(p.start_pairing(Some("elsewhere"), None).is_err());
        assert!(p.start_pairing(Some("custom"), Some("nobody".into())).is_err(), "re-link needs a paired phone");
    }

    #[test]
    fn the_own_address_is_checked_saved_and_added_to_the_host_list_alone() {
        let (p, pings, data) = phone("custom");
        for bad in ["bot.example.com", "ftp://x", "https://a.b/path"] {
            assert!(p.set_custom_address(Some(bad.into())).unwrap_err().starts_with("PHONE_BAD_ADDRESS:"), "{bad}");
        }
        assert!(!p.env.hosts().allows("bot.example.com"));
        p.set_custom_address(Some("https://bot.example.com/".into())).unwrap();
        assert!(p.env.hosts().allows("bot.example.com") && !p.env.hosts().allows("evil.example.com"));
        assert_eq!(config::load(&data).custom_address.as_deref(), Some("https://bot.example.com"));
        assert!(pings.load(Ordering::SeqCst) >= 1, "the Settings page is told");
        p.set_custom_address(Some("http://bot.example.com".into())).unwrap();
        let status = serde_json::to_value(p.status()).unwrap();
        assert!(status["customWarning"].as_str().unwrap().contains("plain http"));
        p.set_custom_address(None).unwrap();
        assert!(!p.env.hosts().allows("bot.example.com"));
        assert_eq!(serde_json::to_value(p.status()).unwrap()["customAddress"], serde_json::Value::Null);
    }

    #[test]
    fn changing_the_port_moves_only_the_phone_server_and_a_taken_port_keeps_the_old_one() {
        let (p, _, data) = phone("port");
        p.bind_server();
        let first = p.bound_port().expect("the server is up");
        assert!(health(first));
        assert!(p.env.lan_origins().iter().all(|o| o.ends_with(&format!(":{first}"))), "the QR addresses use the port really held, not the setting (0 here)");

        let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let err = p.set_port(taken_port).unwrap_err();
        assert!(err.starts_with("PHONE_PORT_IN_USE:") && err.contains(&taken_port.to_string()), "{err}");
        assert!(health(first), "the old server still answers");
        assert_eq!(p.bound_port(), Some(first));
        drop(taken);

        assert!(p.set_port(80).unwrap_err().contains("1024"));
        let next = free_port().max(2000);
        p.set_port(next).unwrap();
        assert_eq!(p.bound_port(), Some(next));
        assert!(health(next));
        std::thread::sleep(Duration::from_millis(300));
        assert!(!health(first), "the old port is let go");
        assert_eq!(config::load(&data).port, next, "the setting is saved");
        assert_eq!(serde_json::to_value(p.status()).unwrap()["port"], next);
        p.stop();
    }

    #[test]
    fn revoking_a_phone_removes_it_and_wakes_the_others() {
        let (p, _, _) = phone("revoke");
        let code = p.pairing.lock().mint_phone_code("x".into(), Caps::all(), None).unwrap().code;
        let grant = p.pairing.lock().redeem_phone_code(&code, "Ren's phone").unwrap();
        let before = p.ctx.changes.rev();
        let devices = serde_json::to_value(p.devices()).unwrap();
        assert_eq!(devices[0]["name"], "Ren's phone");
        assert!(devices[0].get("token").is_none() && devices[0]["caps"].as_array().unwrap().len() == 4);
        assert!(p.revoke(&grant.device_id).unwrap());
        assert!(p.devices().is_empty());
        assert!(p.ctx.changes.rev() > before);
        assert!(!p.revoke(&grant.device_id).unwrap());
    }

    #[test]
    fn the_status_the_settings_page_reads_has_the_agreed_shape() {
        let (p, _, _) = phone("status");
        let v = serde_json::to_value(p.status()).unwrap();
        for key in ["enabled", "port", "portError", "pcName", "lanUrls", "customAddress", "customWarning", "tunnel", "firewall", "devices"] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["tunnel"]["phase"], "off");
        assert!(["ok", "missing", "unknown", "not_windows"].contains(&v["firewall"]["state"].as_str().unwrap()));
    }
}
