//! Spaces (Labs `spaces_enabled`): small-group chat between people's own computers.
//! No server keeps messages. Only `net.rs` talks to the link layer (`crate::p2p`).
//! See docs/plans/1104plans/20-spaces-overview.md.

#[allow(dead_code)]
pub mod limits;

#[allow(dead_code)]
pub mod ids;

#[allow(dead_code)]
pub mod files;

#[allow(dead_code)]
pub mod frame;

#[allow(dead_code)]
pub mod presence;

#[allow(dead_code)]
pub mod identity;

#[allow(dead_code)]
pub mod keystore;

#[allow(dead_code)]
pub mod ingest;

#[allow(dead_code)]
pub mod invite;

#[allow(dead_code)]
pub mod roster;

#[allow(dead_code)]
pub mod wire;

#[allow(dead_code)]
pub mod join;

#[allow(dead_code)]
pub mod bot_bridge;

#[allow(dead_code)]
pub mod diary;

#[allow(dead_code)]
pub mod leak;

#[allow(dead_code)]
pub mod log;

#[allow(dead_code)]
pub mod net;

#[cfg(test)]
#[allow(dead_code)]
pub mod memnet;

#[allow(dead_code)]
pub mod links;

#[allow(dead_code)]
pub mod moderation;

#[allow(dead_code)]
pub mod ratelimit;

#[allow(dead_code)]
pub mod retention;

#[allow(dead_code)]
pub mod runtime;

#[allow(dead_code)]
pub mod service;

#[allow(dead_code)]
pub mod commands;

#[allow(dead_code)]
pub mod store;

#[allow(dead_code)]
pub mod sync;

#[allow(dead_code)]
pub mod words;

#[allow(dead_code)]
pub const LABS_KEY: &str = "spaces_enabled";

#[allow(dead_code)]
pub fn labs_on(public: &serde_json::Value) -> bool {
    public.get(LABS_KEY).and_then(serde_json::Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod guard_tests;

#[cfg(test)]
mod roster_tests;

#[cfg(test)]
mod store_tests;

#[cfg(test)]
mod net_tests;

#[cfg(test)]
mod join_tests;

#[cfg(test)]
mod log_tests;

#[cfg(test)]
mod ingest_tests;

#[cfg(test)]
mod sync_tests;

#[cfg(test)]
pub(crate) mod service_tests;

#[cfg(test)]
mod retention_tests;

#[cfg(test)]
mod moderation_tests;

#[cfg(test)]
mod ratelimit_tests;

#[cfg(test)]
mod files_tests;

#[cfg(test)]
mod presence_tests;

#[cfg(test)]
mod local_net_tests;

#[cfg(test)]
mod leak_tests;

#[cfg(test)]
mod diary_tests;

#[cfg(test)]
mod bot_bridge_tests;

#[cfg(test)]
mod bot_claim_tests;
