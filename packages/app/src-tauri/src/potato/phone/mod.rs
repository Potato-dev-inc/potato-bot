//! The phone: a small web app and the authenticated server that feeds it, so the owner can use the bots away
//! from the PC. See `docs/PHONE-API.md` for the wire format.
//!
//! - `gate`: the one door every request passes (Host, Origin, header, token, abilities), pairing, hello, app files
//! - `api`: the v1 routes behind the door; `push_routes`: web push subscription; `ws`: the WebSocket twin of the long poll
//! - `http`: sockets and parsing; `net`: who may connect, host names, the addresses a phone can try
//! - `proof`: the token-bound answer that shows a phone it reached its own PC
//! - `rate`: sliding-window limits; `config`: the saved setting (on/off, port, own address)
//! - `tunnel`: the Cloudflare quick tunnel kept alive across app restarts
//! - `firewall`: reading and (on a press, with Windows asking) fixing the Windows Firewall rule
//! - `live`: the app's one phone server, the real `Env`, and the status the Settings page shows
//!
//! Nothing here trusts a caller by where it comes from. Every route but the app files, `health`, `hello` and
//! `pair` needs a device token, from a phone on the Wi-Fi or through a tunnel alike.

pub mod api;
pub mod config;
pub mod firewall;
pub mod gate;
pub mod http;
pub mod live;
pub mod net;
pub mod notify;
pub mod proof;
pub mod push_routes;
pub mod rate;
pub mod tunnel;
pub mod ws;

#[cfg(test)]
mod testkit;

#[cfg(test)]
mod tcp_tests;

#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod api_tests;

#[cfg(test)]
mod proof_tests;
