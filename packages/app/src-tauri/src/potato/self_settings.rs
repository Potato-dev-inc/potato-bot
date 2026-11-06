//! Settings from chat: a bot changing a few of its own settings when its owner asks it to in their private chat, with a card
//! in the thread that has an Undo button. The model's tool is `change_my_setting`; the engine (`service/self_flow.rs`)
//! checks who is asking and then applies the change here and in the bot's files.
//!
//! What the bot may change, and only these four: its `name`, its `instructions`, its `persona` (Fish, Potato or Professional)
//! and its `model` (only when it already uses its own API provider: the name of the model, never the address or the key).
//! It can never change its "When to ask" level, "No limit" (the money cap), its connectors, its tool switches (helpers,
//! computer, web), its saved "Always allow" rules, or anything of Settings: those are the owner's, in the app, and no word
//! the model sends reaches them. A `setting` that is not one of the four is refused with a sentence, not guessed at.
//!
//! The journal (`self-settings.json` in the Potato folder) keeps, for each change, the value before and the value after, so
//! Undo can put the old one back. Only the app's own command undoes a change; the model has no tool for it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::bot_settings;
use super::store::{self, StoreError, StoreResult};

/// The tool name the brain sends.
pub const TOOL: &str = "change_my_setting";
pub const NOT_ALLOWED: &str = "POTATO_SETTING_NOT_ALLOWED";
const BAD: &str = "POTATO_SETTING_BAD_VALUE";
/// The longest model name.
pub const MAX_MODEL_CHARS: usize = 80;
/// Journal entries kept; the oldest go first.
pub const KEEP: usize = 300;
/// Characters of a text shown in a card.
const SHOWN: usize = 80;

pub fn is_tool(name: &str) -> bool {
    name == TOOL
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Instructions,
    Persona,
    Model,
}

impl Field {
    pub fn parse(raw: &str) -> Option<Field> {
        match raw.trim().to_lowercase().as_str() {
            "name" => Some(Field::Name),
            "instructions" => Some(Field::Instructions),
            "persona" => Some(Field::Persona),
            "model" => Some(Field::Model),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Instructions => "instructions",
            Field::Persona => "persona",
            Field::Model => "model",
        }
    }

    /// Does this change need the brain to start again? (Its persona and instructions are read when it starts.)
    pub fn restarts(self) -> bool {
        !matches!(self, Field::Name)
    }
}

/// A refusal: a code and one plain sentence the model can read.
pub type Refusal = (&'static str, String);

fn bad(message: impl Into<String>) -> Refusal {
    (BAD, message.into())
}

/// The model's arguments, checked: which of the four, and the new value, cleaned.
pub fn parse_request(args: &Value) -> Result<(Field, String), Refusal> {
    let setting = args.get("setting").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let Some(field) = Field::parse(&setting) else {
        let message = if setting.is_empty() {
            "Say which setting: name, instructions, persona or model.".to_string()
        } else {
            "That is not something a bot can change for itself. Only its owner can, in the app, under the bot's Advanced tab. You can change your name, instructions, persona or model.".to_string()
        };
        return Err((NOT_ALLOWED, message));
    };
    let raw = args.get("value").and_then(Value::as_str).ok_or_else(|| bad("Give the new value."))?;
    if raw.chars().count() > bot_settings::MAX_INSTRUCTION_CHARS + 100 {
        return Err(bad("That value is too long."));
    }
    Ok((field, clean_value(field, raw)?))
}

/// The value for a field, or why it is refused. The same checks the app's own forms make, plus a few for a text the model wrote.
pub fn clean_value(field: Field, raw: &str) -> Result<String, Refusal> {
    match field {
        Field::Name => {
            if raw.chars().any(char::is_control) {
                return Err(bad("A name is one line of plain text."));
            }
            bot_settings::clean_name(raw).map_err(|e| bad(e.message))
        }
        Field::Instructions => {
            let text = bot_settings::clean_instructions(raw).map_err(|e| bad(e.message))?;
            if text.is_empty() {
                return Err(bad("Give the instructions to keep. Your owner clears them in the app."));
            }
            Ok(text)
        }
        Field::Persona => {
            let pack = raw.trim().to_lowercase();
            if store::valid_pack(&pack) {
                Ok(pack)
            } else {
                Err(bad("The persona is one of fish, potato or pro."))
            }
        }
        Field::Model => {
            let model = raw.trim();
            let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-' | '@');
            // A name like `openai/gpt-4o` or `llama3:8b` is fine; an address (`https://...`) is not a model.
            if model.is_empty() || model.chars().count() > MAX_MODEL_CHARS || !model.chars().all(plain) || model.contains("//") || model.contains("..") {
                return Err(bad(format!("A model name is {MAX_MODEL_CHARS} characters at most: letters, digits and . _ : / - @.")));
            }
            Ok(model.to_string())
        }
    }
}

/// The words on the card for a change.
pub fn describe(field: Field, prior: &str, current: &str) -> String {
    let shown = |t: &str| {
        let flat: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
        let cut: String = flat.chars().take(SHOWN).collect();
        if flat.chars().count() > SHOWN {
            format!("{cut}…")
        } else {
            cut
        }
    };
    match field {
        Field::Name => format!("Changed its name from \u{201c}{}\u{201d} to \u{201c}{}\u{201d}.", shown(prior), shown(current)),
        Field::Instructions => {
            if prior.is_empty() {
                format!("Set its instructions: \u{201c}{}\u{201d}.", shown(current))
            } else {
                format!("Changed its instructions to \u{201c}{}\u{201d} (it had {} characters).", shown(current), prior.chars().count())
            }
        }
        Field::Persona => format!("Changed its persona from {} to {}.", persona_name(prior), persona_name(current)),
        Field::Model => format!("Changed its model from {} to {}.", shown(prior), shown(current)),
    }
}

pub fn persona_name(pack: &str) -> &'static str {
    match pack {
        "potato" => "Potato",
        "pro" => "Professional",
        _ => "Fish",
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub bot: String,
    pub field: String,
    pub prior: String,
    pub current: String,
    pub at: i64,
    pub undone: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Journal {
    #[serde(default)]
    entries: Vec<Entry>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join("self-settings.json")
}

fn read(root: &Path) -> StoreResult<Journal> {
    Ok(store::read_json(&path(root), "self-settings.json")?.unwrap_or_default())
}

/// Adds a change to the journal and returns its entry.
pub fn record(root: &Path, bot: &str, field: Field, prior: &str, current: &str, now: i64) -> StoreResult<Entry> {
    let _files = store::files_lock();
    let mut journal = read(root)?;
    let entry = Entry {
        id: format!("sc-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]),
        bot: bot.to_string(),
        field: field.word().to_string(),
        prior: prior.to_string(),
        current: current.to_string(),
        at: now,
        undone: false,
    };
    journal.entries.push(entry.clone());
    let extra = journal.entries.len().saturating_sub(KEEP);
    journal.entries.drain(..extra);
    store::write_json(&path(root), &journal, "self-settings.json")?;
    Ok(entry)
}

/// One change of this bot, by id.
pub fn find(root: &Path, bot: &str, id: &str) -> StoreResult<Entry> {
    read(root)?
        .entries
        .into_iter()
        .find(|e| e.id == id && e.bot == bot)
        .ok_or_else(|| StoreError::new("POTATO_SETTING_CHANGE_GONE", "That change is not in the record any more."))
}

pub fn mark_undone(root: &Path, id: &str) -> StoreResult<()> {
    let _files = store::files_lock();
    let mut journal = read(root)?;
    if let Some(entry) = journal.entries.iter_mut().find(|e| e.id == id) {
        entry.undone = true;
    }
    store::write_json(&path(root), &journal, "self-settings.json")
}

/// The ids of the changes that were undone. A missing or damaged journal reads as none.
pub fn undone_ids(root: &Path) -> std::collections::HashSet<String> {
    read(root).map(|j| j.entries.into_iter().filter(|e| e.undone).map(|e| e.id).collect()).unwrap_or_default()
}
