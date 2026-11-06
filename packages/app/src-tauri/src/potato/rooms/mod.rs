//! Groups of bots (plan 12). `store` (G2) keeps the rooms and their lines under `<app data>/potato/rooms/`; the root
//! is the potato folder, not a bot's folder, because a room belongs to no single bot. `fanout` (G7) decides who hears
//! a new line and builds the `room` item; the engine side is in `service/room_flow.rs`.

pub mod fanout;
pub mod store;

#[cfg(test)]
mod fanout_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod wire_tests;
