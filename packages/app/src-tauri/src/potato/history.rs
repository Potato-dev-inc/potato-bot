//! The bot's own past chats, for `search_history` and `read_history` (spec Batch 9). Only the owner's chat with
//! this bot is read: `app:dm:<bot>`. Another bot's chat, a shared channel and a stranger's thread are never read here.
//! The answer keeps who said each line (`owner`, `bot`, `person` or `note`) so the brain can label it; the brain
//! fences the whole answer as data.
//!
//! The caller checks that the turn is the owner's (`service.rs`). Nothing is written.

use std::path::Path;

use serde_json::{json, Value};

use super::store::{self, FromKind, Message, StoreError, StoreResult};

pub const SEARCH_TOOL: &str = "search_history";
pub const READ_TOOL: &str = "read_history";
/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 200;
/// Most hits one search returns (newest first).
pub const MAX_HITS: usize = 8;
/// Characters of a message a search hit shows.
pub const MAX_SNIPPET_CHARS: usize = 300;
/// Messages either side of the one `read_history` names.
pub const READ_AROUND: usize = 4;
/// Characters of each message `read_history` shows.
pub const MAX_READ_CHARS: usize = 600;
const MAX_NAME_CHARS: usize = 60;
const BAD_ARGS: &str = "POTATO_HISTORY_BAD_ARGS";
const NOT_FOUND: &str = "POTATO_HISTORY_NOT_FOUND";

/// True for the two history tool names, and only those.
pub fn is_tool(name: &str) -> bool {
    matches!(name, SEARCH_TOOL | READ_TOOL)
}

/// The model's `search_history` and `read_history` for one bot. Returns JSON, or a code and one plain sentence.
pub fn call(root: &Path, bot: &str, tool: &str, args: &Value) -> StoreResult<Value> {
    // Everything in the owner's chat with this bot, oldest first, redacted by the store on the way out.
    let all = store::read_messages(root, &store::app_conversation(bot), usize::MAX, None)?;
    match tool {
        SEARCH_TOOL => search(&all, args),
        READ_TOOL => read(&all, args),
        _ => Err(StoreError::new("UNKNOWN_TOOL", "There is no such history tool.")),
    }
}

fn search(all: &[Message], args: &Value) -> StoreResult<Value> {
    let query = args.get("query").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return Err(StoreError::new(BAD_ARGS, format!("Give the words to look for, {MAX_QUERY_CHARS} characters at most.")));
    }
    let needle = query.to_lowercase();
    let hits: Vec<Value> = all
        .iter()
        .rev()
        .filter(|m| m.text.to_lowercase().contains(&needle))
        .take(MAX_HITS)
        .map(|m| json!({ "id": m.id, "at": m.at, "speaker": speaker(m), "text": cut(&m.text, MAX_SNIPPET_CHARS) }))
        .collect();
    Ok(json!({ "hits": hits, "maxHits": MAX_HITS, "searched": all.len() }))
}

fn read(all: &[Message], args: &Value) -> StoreResult<Value> {
    let id = args.get("id").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let at = all
        .iter()
        .position(|m| !id.is_empty() && m.id == id)
        .ok_or_else(|| StoreError::new(NOT_FOUND, "No message in your chat has that id. Use search_history to find one."))?;
    let from = at.saturating_sub(READ_AROUND);
    let to = (at + READ_AROUND + 1).min(all.len());
    let messages: Vec<Value> = all[from..to]
        .iter()
        .map(|m| json!({ "id": m.id, "at": m.at, "speaker": speaker(m), "text": cut(&m.text, MAX_READ_CHARS), "target": m.id == id }))
        .collect();
    Ok(json!({ "messages": messages }))
}

/// Who wrote a line, in words the brain can label: the owner, the bot, another person (with the name), or a note.
fn speaker(m: &Message) -> Value {
    let kind = match m.from.kind {
        FromKind::You => "owner",
        FromKind::Bot => "bot",
        FromKind::Person => "person",
        FromKind::Note => "note",
    };
    let name = m.from.name.as_deref().map(|n| cut(n, MAX_NAME_CHARS));
    json!({ "kind": kind, "name": name })
}

fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
