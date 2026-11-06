//! The owner's emoji reactions to messages in a bot's thread. A small fixed set (mirrored in the app's TypeScript as
//! `REACTIONS`); anything else is refused. They live in a sidecar file (`reactions.json` in the Potato root), keyed by
//! message id, so the conversation files stay as they are; `service` adds them to the messages when listing. Only the
//! app's own command sets them: no brain tool or channel reaches this file, so the model cannot react.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::store::{self, StoreError, StoreResult};

/// The only reactions there are, in the order they are shown.
pub const EMOJI: [&str; 6] = ["👍", "❤️", "😂", "🎉", "😮", "🙏"];

/// The newest messages that keep their reactions. Older ones are dropped.
pub const KEEP: usize = 2000;

#[derive(Serialize, Deserialize, Default)]
struct Reactions {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    id: String,
    emoji: Vec<String>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join("reactions.json")
}

pub fn allowed(emoji: &str) -> bool {
    EMOJI.contains(&emoji)
}

fn read(root: &Path) -> StoreResult<Reactions> {
    Ok(store::read_json(&path(root), "reactions.json")?.unwrap_or_default())
}

/// Turn `emoji` on for the message, or off when it is already on. Returns whether it is on now.
pub fn toggle(root: &Path, message_id: &str, emoji: &str) -> StoreResult<bool> {
    if !allowed(emoji) {
        return Err(StoreError::new("POTATO_REACTION_INVALID", "That reaction is not one of the choices."));
    }
    let _files = store::files_lock();
    let mut all = read(root)?;
    let at = all.entries.iter().position(|e| e.id == message_id);
    let on = match at {
        Some(i) => {
            let entry = &mut all.entries[i];
            let on = !entry.emoji.iter().any(|e| e == emoji);
            if on {
                entry.emoji.push(emoji.to_string());
                entry.emoji.sort_by_key(|e| EMOJI.iter().position(|x| x == e));
            } else {
                entry.emoji.retain(|e| e != emoji);
            }
            if all.entries[i].emoji.is_empty() {
                all.entries.remove(i);
            }
            on
        }
        None => {
            all.entries.push(Entry { id: message_id.to_string(), emoji: vec![emoji.to_string()] });
            true
        }
    };
    let extra = all.entries.len().saturating_sub(KEEP);
    all.entries.drain(..extra);
    store::write_json(&path(root), &all, "reactions.json")?;
    Ok(on)
}

/// Message id to its reactions, for the messages that have any. A missing or damaged file reads as none.
pub fn load(root: &Path) -> HashMap<String, Vec<String>> {
    read(root).map(|r| r.entries.into_iter().map(|e| (e.id, e.emoji)).collect()).unwrap_or_default()
}
