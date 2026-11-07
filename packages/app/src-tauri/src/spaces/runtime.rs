//! Starts and stops Spaces with the Labs switch `spaces_enabled`.
//!
//! Off (the default): nothing is built. No link layer, no key read, no socket, no task. This module is only asked
//! when settings change, and a settings change with the switch off and nothing ever started returns at once.
//! On: build the link layer once, start the service on the app's async runtime. Off again: stop the service and
//! tell the link layer, which closes the endpoint.
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use super::net::AppLink;
use super::service::{Clock, SpacesService};
use crate::spaces::keystore::KeyStore;

/// What the service is made of. Built when the switch first goes on, never before.
pub struct Parts {
    pub link: AppLink,
    pub keystore: KeyStore,
    pub dir: PathBuf,
    pub clock: Clock,
}

pub type Spawner = Arc<dyn Fn(Pin<Box<dyn Future<Output = ()> + Send>>) + Send + Sync>;
pub type MakeParts = Arc<dyn Fn() -> Result<Parts, String> + Send + Sync>;

struct State {
    on: bool,
    generation: u64,
    parts: Option<Arc<Parts>>,
    service: Option<Arc<SpacesService>>,
}

pub struct Runtime {
    make: MakeParts,
    spawn: Spawner,
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(|p| p.into_inner()) }

impl Runtime {
    pub fn new(make: MakeParts, spawn: Spawner) -> Arc<Self> {
        Arc::new(Self { make, spawn, state: Mutex::new(State { on: false, generation: 0, parts: None, service: None }) })
    }

    pub fn service(&self) -> Option<Arc<SpacesService>> { lock(&self.state).service.clone() }
    pub fn link_built(&self) -> bool { lock(&self.state).parts.is_some() }
    pub fn endpoint_running(&self) -> bool { lock(&self.state).parts.as_ref().map(|p| p.link.running()).unwrap_or(false) }

    /// Called with the effective public settings every time they change.
    pub fn apply_settings(self: &Arc<Self>, public: &Value) {
        let want = super::labs_on(public);
        let mut st = lock(&self.state);
        if !want {
            st.on = false;
            st.generation += 1;
            if let Some(parts) = &st.parts { parts.link.apply_settings(public); }
            let service = st.service.take();
            drop(st);
            crate::potato::space_link::disconnect();
            if let Some(svc) = service {
                (self.spawn)(Box::pin(async move { svc.stop().await }));
            }
            return;
        }
        if st.on {
            if let Some(parts) = &st.parts { parts.link.apply_settings(public); }
            return;
        }
        let parts = match st.parts.clone() {
            Some(p) => p,
            None => match (self.make)() {
                Ok(p) => {
                    let p = Arc::new(p);
                    st.parts = Some(p.clone());
                    p
                }
                Err(_) => return,
            },
        };
        st.on = true;
        st.generation += 1;
        let generation = st.generation;
        parts.link.apply_settings(public);
        drop(st);
        let this = self.clone();
        (self.spawn)(Box::pin(async move {
            let svc = SpacesService::new(
                parts.link.net(), parts.link.perms(), parts.keystore.clone(), parts.dir.clone(), parts.clock.clone(),
            );
            if svc.start().await.is_err() {
                svc.stop().await;
                return;
            }
            let keep = {
                let mut st = lock(&this.state);
                let keep = st.on && st.generation == generation;
                if keep { st.service = Some(svc.clone()); }
                keep
            };
            if keep {
                // Potato Bot may be up already: bots can join as soon as both are.
                crate::potato::space_link::connect_live();
            }
            if !keep {
                // The switch went off while this was starting.
                svc.stop().await;
            }
        }));
    }
}

static APP: OnceLock<Arc<Runtime>> = OnceLock::new();

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The app's one runtime. Created only when the switch is first seen on.
fn app_runtime() -> &'static Arc<Runtime> {
    APP.get_or_init(|| {
        let make: MakeParts = Arc::new(|| {
            Ok(Parts {
                link: super::net::app_space_net()?,
                keystore: KeyStore::app(),
                dir: crate::app_paths::app_data_dir().join("spaces"),
                clock: Arc::new(now_ms),
            })
        });
        let spawn: Spawner = Arc::new(|f| {
            tauri::async_runtime::spawn(f);
        });
        Runtime::new(make, spawn)
    })
}

/// Settings hook (`settings_store::apply_public_settings`). With the switch off and nothing started this does
/// nothing at all: it does not even create the runtime object.
pub fn apply_settings(public: &Value) {
    if !super::labs_on(public) && APP.get().is_none() { return; }
    app_runtime().apply_settings(public);
}

/// The running service, if the switch is on and it has started. Commands read it; they never start it.
pub fn running_service() -> Option<Arc<SpacesService>> { APP.get().and_then(|r| r.service()) }

/// True once the app's runtime object exists (it is made only when the switch is first seen on).
pub fn app_runtime_exists() -> bool { APP.get().is_some() }
