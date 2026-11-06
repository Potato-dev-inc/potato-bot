//! `<app data>/agent_run_mode.json`: the run-mode choice per agent id.
//!
//! A missing, unreadable or half-written file, or a value this build does not know, means the
//! default: a visible pane. Only agents that were changed are listed, so the file stays tiny and a
//! newer build's extra values are simply ignored by an older one.

use super::RunMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const FILE: &str = "agent_run_mode.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StoreFile {
    version: u32,
    /// Agent id (lower case) -> `visible` | `one_task`. Values are kept as text so one unknown
    /// value never discards the others.
    agents: BTreeMap<String, String>,
}

#[cfg(not(test))]
fn path() -> std::path::PathBuf {
    crate::app_paths::app_data_dir().join(FILE)
}

fn key(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

fn parse(text: &str) -> StoreFile {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(not(test))]
static CACHE: once_cell::sync::Lazy<parking_lot::RwLock<Option<StoreFile>>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(None));

#[cfg(test)]
thread_local! {
    /// A test sets the choices (and what is verified) for its own thread only.
    static TEST_ENV: std::cell::RefCell<Option<TestEnv>> = const { std::cell::RefCell::new(None) };
}

/// What a test fakes: the saved choices and, per agent id, whether its headless mode is verified.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct TestEnv {
    pub modes: BTreeMap<String, RunMode>,
    pub verified: BTreeMap<String, bool>,
}

#[cfg(test)]
pub fn with_test_env<T>(env: TestEnv, body: impl FnOnce() -> T) -> T {
    let before = TEST_ENV.with(|slot| slot.replace(Some(env)));
    let out = body();
    TEST_ENV.with(|slot| *slot.borrow_mut() = before);
    out
}

/// The faked verification of `id` when a test environment is active.
#[cfg(test)]
pub(super) fn test_verified(id: &str) -> Option<bool> {
    TEST_ENV.with(|slot| slot.borrow().as_ref().map(|env| env.verified.get(&key(id)).copied().unwrap_or(false)))
}

pub(super) fn read_file(path: &Path) -> StoreFile {
    std::fs::read_to_string(path).map(|text| parse(&text)).unwrap_or_default()
}

#[cfg(not(test))]
fn loaded() -> StoreFile {
    if let Some(file) = CACHE.read().as_ref() {
        return file.clone();
    }
    let file = read_file(&path());
    *CACHE.write() = Some(file.clone());
    file
}

pub(super) fn mode_in(file: &StoreFile, id: &str) -> RunMode {
    file.agents.get(&key(id)).and_then(|value| RunMode::parse(value)).unwrap_or_default()
}

/// The choice for `id`; a visible pane unless the person changed it.
pub fn setting_for(id: &str) -> RunMode {
    #[cfg(test)]
    {
        // Tests never read the real app data folder: no fake environment means the default.
        TEST_ENV
            .with(|slot| slot.borrow().as_ref().map(|env| env.modes.get(&key(id)).copied()))
            .flatten()
            .unwrap_or_default()
    }
    #[cfg(not(test))]
    {
        mode_in(&loaded(), id)
    }
}

#[cfg(test)]
pub(super) fn file_name() -> &'static str {
    FILE
}

#[cfg(test)]
pub(super) fn parse_text(text: &str) -> StoreFile {
    parse(text)
}

