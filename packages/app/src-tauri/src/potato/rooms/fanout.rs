//! Who hears a new line in a group, and the `room` inbox item they are given. Pure: no files, no clock.
//!
//! The wire shape is written in `packages/potato-brain/src/rooms/types.ts` (`RoomInboxItem`); this file writes
//! exactly that: `type:"room"`, `conversationId:"app:room:<id>"`, `roster` with the person first, `recent` at most
//! 20 lines, `line`, and `sandboxOnly`. The caller adds `capabilities` the way a `send` gets them.

use serde_json::{json, Value};

use super::store::{Room, RoomFrom, RoomLine, Speaker};
use crate::potato::protocol::{room_item, Tools};

/// A bot's line this deep is saved but not passed on. The brain's `MAX_BOT_HOPS` is the same number.
pub const MAX_BOT_HOPS: u8 = 3;
/// The most earlier lines one item carries.
pub const RECENT_FOR_BRAIN: usize = 20;
/// The hop of a line that is a note from the app about a bot (it stopped, it is out of budget). Never passed on and
/// never shown to a brain as earlier talk.
pub const NOTE_HOP: u8 = u8::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    Sender,
    HopLimit,
    BotOff,
    DailyLimit,
    Held,
    /// Switched off in this group only (group settings).
    Muted,
}

/// Which bots get this line, and which are skipped and why. Never the sender. Nobody gets a line at or past the hop
/// limit. One bot being off or over its limit never stops the others.
pub fn targets(
    room: &Room,
    line: &RoomLine,
    bot_on: &dyn Fn(&str) -> bool,
    over_limit: &dyn Fn(&str) -> bool,
    held: bool,
) -> (Vec<String>, Vec<(String, Skip)>) {
    let mut go = Vec::new();
    let mut skip = Vec::new();
    for bot in &room.bot_ids {
        let why = if line.from.kind == Speaker::Bot && &line.from.id == bot {
            Some(Skip::Sender)
        } else if !room.bot_listens(bot) {
            Some(Skip::Muted)
        } else if held {
            Some(Skip::Held)
        } else if line.hop >= MAX_BOT_HOPS {
            Some(Skip::HopLimit)
        } else if !bot_on(bot) {
            Some(Skip::BotOff)
        } else if over_limit(bot) {
            Some(Skip::DailyLimit)
        } else {
            None
        };
        match why {
            Some(why) => skip.push((bot.clone(), why)),
            None => go.push(bot.clone()),
        }
    }
    (go, skip)
}

/// The JSON inbox item for one bot. `roster` lists everyone, the person first. `recent` is the lines before `line`,
/// oldest first; only the newest 20 are sent. `sandbox_only` is true when the author is not the owner (a later,
/// shared group); in version 1 every person line is the owner's, so it is false.
pub fn inbox_item(
    item_id: &str,
    room: &Room,
    me: &RoomFrom,
    roster: &[RoomFrom],
    recent: &[RoomLine],
    line: &RoomLine,
    tools: Tools,
    sandbox_only: bool,
) -> Value {
    let recent = &recent[recent.len().saturating_sub(RECENT_FOR_BRAIN)..];
    room_item(json!({
            "id": item_id,
            "roomId": room.id,
            "roomName": room.name,
            "conversationId": crate::potato::store::app_room_conversation(&room.id),
            "me": me,
            "roster": roster,
            "recent": recent,
            "line": line,
            "tools": tools.as_str(),
            "sandboxOnly": sandbox_only,
            "mode": room.mode.as_str(),
            "instructions": room.instructions,
            "folder": room.folder_path(),
    }))
}
