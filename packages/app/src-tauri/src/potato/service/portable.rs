//! Moving a bot in and out: one file per bot (name, voice, instructions, skills, routines, and the memory notes when the
//! person chooses), a copy of a bot, and a whole-account folder for a new PC. Our own format; the ideas (a preview before
//! anything is made, nothing sensitive in the file) come from Rakazo's import page and OpenMausBot's team sharing.
//!
//! What is never in a file: keys, tokens, connector secrets, cookies, passwords, tool servers (and their settings), "always
//! allow" rules, the ask level, the daily limit, the sites list. The writer does not read those, so they cannot leak, and every text
//! that is written goes through the same scrub as a chat message.
//!
//! What a file cannot do: it is data. The reader looks only at the fields below and ignores every other one, so a file that
//! carries `level`, `limitCents`, `mcpServers` or `connectors` changes none of them. Every name that becomes a file name is made
//! here from letters and digits; no name or path from the file is ever used as a path. A routine arrives PAUSED and without a
//! watch (the check of a watch is a command). The file is read with a size cap before it is parsed; a zip or a gzip is refused
//! without being opened.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Service;
use crate::potato::{bot_settings, memory_notes, routines, skills, store};
use crate::potato::store::{FromKind, Message, MsgKind, StoreError, StoreResult};

pub const FORMAT: &str = "potato-bot";
pub const VERSION: u32 = 1;
/// The most bytes of a bot file or an account file that is read at all.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_SKILLS: usize = 30;
pub const MAX_ROUTINES: usize = routines::MAX_ROUTINES_PER_BOT;
pub const MAX_NOTES: usize = 200;
pub const MAX_NOTE_CHARS: usize = 20_000;
/// All memory text of one file together.
pub const MAX_MEMORY_CHARS: usize = 600_000;
pub const MAX_CHAT_MESSAGES: usize = 2_000;
const INVALID: &str = "POTATO_IMPORT_INVALID";
const AVATARS: [&str; 8] = ["mochi", "bit", "nib", "tofu", "lumen", "hoot", "sprout", "kit"];
const PERSONAS: [&str; 5] = ["cheerful", "calm", "coach", "plain", "custom"];
const TALK: [&str; 3] = ["fish", "potato", "pro"];

fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::new(INVALID, message)
}

// ---- the file ---------------------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BotInfo {
    pub name: String,
    pub avatar: String,
    /// The companion's voice (cheerful, calm, ...).
    pub persona: String,
    /// Fish, Potato or Professional.
    pub talk_persona: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SkillItem {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutineItem {
    pub title: String,
    pub prompt: String,
    pub schedule: routines::Schedule,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteItem {
    /// One of the vault folders (`journal`, `diary`, `nodes`).
    pub folder: String,
    /// A label for the file name; only letters and digits are used.
    pub name: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub exported_ms: i64,
    pub bot: BotInfo,
    pub instructions: String,
    pub skills: Vec<SkillItem>,
    /// Read one by one from the file (`read_routines`), so a routine that does not parse is dropped and the rest stay.
    #[serde(skip)]
    pub routines: Vec<RoutineItem>,
    pub memory: Vec<NoteItem>,
}

/// What the person ticks before anything is made.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Choices {
    pub instructions: bool,
    pub skills: bool,
    pub routines: bool,
    pub memory: bool,
}

impl Choices {
    pub const ALL: Choices = Choices { instructions: true, skills: true, routines: true, memory: true };
}

/// The preview: the cleaned file, the lines to tell the person, and what was left out.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub bot: BotInfo,
    pub instructions: String,
    pub skills: Vec<SkillItem>,
    pub routines: Vec<RoutinePreview>,
    pub memory: Vec<NotePreview>,
    /// What was cut, dropped or changed on the way in, in plain words.
    pub notes: Vec<String>,
    /// The words that are always true of an import.
    pub promise: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutinePreview {
    pub title: String,
    pub prompt: String,
    pub summary: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotePreview {
    pub folder: String,
    pub name: String,
    pub chars: usize,
}

pub const PROMISE: &str = "A file adds only text. It cannot turn on tools, connect an account, or change how much this bot asks first, and routines come in paused.";

#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub instructions: bool,
    pub skills: usize,
    pub routines: usize,
    pub notes: usize,
    pub chat: usize,
}

// ---- reading ----------------------------------------------------------------------------------------------------------

/// Reads a file the person picked, never more than `MAX_FILE_BYTES`, and refuses a zip or gzip without opening it.
pub fn read_capped(path: &Path) -> StoreResult<String> {
    use std::io::Read;
    let meta = fs::symlink_metadata(path).map_err(|_| invalid("That file could not be opened."))?;
    if !meta.is_file() {
        return Err(invalid("That is not a file."));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(invalid("That file is too big to be a bot file (more than 2 MB). Nothing was imported."));
    }
    let mut head = Vec::new();
    fs::File::open(path).map_err(|_| invalid("That file could not be opened."))?.take(MAX_FILE_BYTES + 1).read_to_end(&mut head).map_err(|_| invalid("That file could not be read."))?;
    if head.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("That file is too big to be a bot file (more than 2 MB). Nothing was imported."));
    }
    text_of(head)
}

/// Bytes to text, refusing archives (a zip or a gzip is never unpacked here, so a zip bomb has nothing to blow up).
pub fn text_of(bytes: Vec<u8>) -> StoreResult<String> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(&[0x1f, 0x8b]) || bytes.starts_with(b"7z\xbc\xaf") || bytes.starts_with(b"Rar!") {
        return Err(invalid("That is a compressed file. Unzip it first, then pick the bot file inside. Nothing was imported."));
    }
    String::from_utf8(bytes).map_err(|_| invalid("That does not look like a bot file."))
}

/// Parses the file text into a cleaned bundle plus the notes about what changed. Nothing is written.
pub fn parse(text: &str) -> StoreResult<(Bundle, Vec<String>)> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("That file is too big to be a bot file. Nothing was imported."));
    }
    let value: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| invalid("That does not look like a bot file."))?;
    let mut bundle: Bundle = serde_json::from_value(value.clone()).map_err(|_| invalid("That does not look like a bot file."))?;
    if bundle.format != FORMAT {
        return Err(invalid("That is not a Potato Bot file."));
    }
    if bundle.version == 0 || bundle.version > VERSION {
        return Err(invalid("That file was made by a newer Puppet Master. Update first."));
    }
    bundle.routines = read_routines(&value);
    let mut notes = Vec::new();
    clean(&mut bundle, &mut notes);
    Ok((bundle, notes))
}

/// Routines one by one: a routine that does not parse is dropped, the rest stay.
fn read_routines(value: &serde_json::Value) -> Vec<RoutineItem> {
    value.get("routines").and_then(|v| v.as_array()).map(|list| list.iter().filter_map(|r| serde_json::from_value::<RoutineItem>(r.clone()).ok()).collect()).unwrap_or_default()
}

fn one_line(raw: &str, max: usize) -> String {
    let flat: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let text = store::clean_text(&flat.split_whitespace().collect::<Vec<_>>().join(" "), None);
    text.chars().take(max).collect::<String>().trim().to_string()
}

/// Letters and digits only, lower case, at most `max`: the only thing a file name is ever made from.
pub fn slug(raw: &str, max: usize) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn clean(b: &mut Bundle, notes: &mut Vec<String>) {
    b.format = FORMAT.to_string();
    b.bot.name = one_line(&b.bot.name, bot_settings::MAX_NAME_CHARS);
    if b.bot.name.is_empty() {
        b.bot.name = "Imported bot".to_string();
        notes.push("The file had no name, so the bot is called Imported bot.".into());
    }
    if !AVATARS.contains(&b.bot.avatar.as_str()) {
        b.bot.avatar = "bit".into();
    }
    if !PERSONAS.contains(&b.bot.persona.as_str()) {
        b.bot.persona = "plain".into();
    }
    if !TALK.contains(&b.bot.talk_persona.as_str()) {
        b.bot.talk_persona = "fish".into();
    }
    let before = b.instructions.chars().count();
    b.instructions = store::clean_text(b.instructions.trim(), None).chars().take(bot_settings::MAX_INSTRUCTION_CHARS).collect::<String>().trim().to_string();
    if before > bot_settings::MAX_INSTRUCTION_CHARS {
        notes.push(format!("The instructions were cut to {} characters.", bot_settings::MAX_INSTRUCTION_CHARS));
    }

    let asked = b.skills.len();
    let mut seen = Vec::new();
    let mut kept = Vec::new();
    for s in std::mem::take(&mut b.skills) {
        let name = one_line(&s.name, skills::NAME_MAX);
        let body = store::clean_text(s.body.replace("\r\n", "\n").trim(), None);
        let description = one_line(&s.description, skills::DESCRIPTION_MAX);
        let file = slug(&name, 40);
        if name.is_empty() || file.is_empty() || body.is_empty() || seen.contains(&file) {
            continue;
        }
        if skill_text(&name, &description, &body).chars().count() > skills::SKILL_CAP {
            notes.push(format!("The skill \"{name}\" is longer than {} characters and was left out.", skills::SKILL_CAP));
            continue;
        }
        if kept.len() >= MAX_SKILLS {
            break;
        }
        seen.push(file);
        kept.push(SkillItem { name, description, body });
    }
    if asked > kept.len() {
        notes.push(format!("{} of {} skills were left out (empty, repeated, too long, or more than {MAX_SKILLS}).", asked - kept.len(), asked));
    }
    b.skills = kept;

    let asked = b.routines.len();
    let now = routines::local_naive(chrono::Local::now().timestamp_millis());
    let mut kept = Vec::new();
    for r in std::mem::take(&mut b.routines) {
        let item = RoutineItem { title: r.title.trim().to_string(), prompt: store::clean_text(r.prompt.trim(), None), schedule: r.schedule };
        let input = routines::RoutineInput { title: item.title.clone(), prompt: item.prompt.clone(), schedule: item.schedule.clone(), watch: None };
        if routines::validate(&input, now).is_ok() && kept.len() < MAX_ROUTINES {
            kept.push(item);
        }
    }
    if asked > kept.len() {
        notes.push(format!("{} of {} routines were left out (not valid, or already in the past).", asked - kept.len(), asked));
    }
    b.routines = kept;

    let asked = b.memory.len();
    let mut total = 0usize;
    let mut kept = Vec::new();
    for n in std::mem::take(&mut b.memory) {
        let folder = n.folder.trim().to_string();
        let name = slug(&n.name, 60);
        let text: String = store::clean_text(n.text.replace("\r\n", "\n").trim(), None).chars().take(MAX_NOTE_CHARS).collect();
        if !memory_notes::FOLDERS.contains(&folder.as_str()) || name.is_empty() || text.trim().is_empty() {
            continue;
        }
        total += text.chars().count();
        if kept.len() >= MAX_NOTES || total > MAX_MEMORY_CHARS {
            break;
        }
        kept.push(NoteItem { folder, name, text });
    }
    if asked > kept.len() {
        notes.push(format!("{} of {} memory notes were left out (empty, odd folder, or over the size limit).", asked - kept.len(), asked));
    }
    b.memory = kept;
}

pub fn preview_of(b: &Bundle, notes: Vec<String>) -> Preview {
    Preview {
        bot: b.bot.clone(),
        instructions: b.instructions.clone(),
        skills: b.skills.clone(),
        routines: b.routines.iter().map(|r| RoutinePreview { title: r.title.clone(), prompt: r.prompt.clone(), summary: routines::summary(&r.schedule) }).collect(),
        memory: b.memory.iter().map(|n| NotePreview { folder: n.folder.clone(), name: n.name.clone(), chars: n.text.chars().count() }).collect(),
        notes,
        promise: PROMISE.to_string(),
    }
}

// ---- writing a bundle into a bot --------------------------------------------------------------------------------------

fn skill_text(name: &str, description: &str, body: &str) -> String {
    format!("# {name}\nDescription: {description}\n\n{body}\n")
}

/// Puts the ticked parts of a cleaned bundle into an existing bot. Instructions become a new version (Restore can bring the old
/// text back); a skill with a taken name is skipped, never overwritten; routines are made paused.
pub fn apply(root: &Path, bot: &str, b: &Bundle, choices: Choices, now_ms: i64) -> StoreResult<Applied> {
    store::require_bot(root, bot)?;
    let mut done = Applied::default();
    if choices.instructions && !b.instructions.is_empty() {
        let text = b.instructions.clone();
        bot_settings::update(root, bot, |s| bot_settings::save_instructions(s, &text, now_ms).map(|_| ()))?;
        done.instructions = true;
    }
    if choices.skills {
        let folder = skills::dir(root, bot)?;
        let _files = store::files_lock();
        for s in &b.skills {
            let file = slug(&s.name, 40);
            let target = folder.join(format!("{file}.md"));
            if file.is_empty() || target.exists() {
                continue;
            }
            crate::secure_file::create_private_dir(&folder).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not make the skills folder: {e}.")))?;
            crate::secure_file::write_private(&target, &skill_text(&s.name, &s.description, &s.body)).map_err(|e| StoreError::new("POTATO_STORE_INVALID", e))?;
            done.skills += 1;
        }
    }
    if choices.routines {
        for r in &b.routines {
            let input = routines::RoutineInput { title: r.title.clone(), prompt: r.prompt.clone(), schedule: r.schedule.clone(), watch: None };
            let made = routines::create(root, bot, &input, now_ms)?;
            routines::set_paused(root, &made.id, true, now_ms)?;
            done.routines += 1;
        }
    }
    if choices.memory {
        let vault = store::vault_dir(root, bot)?;
        let _files = store::files_lock();
        for n in &b.memory {
            if !memory_notes::FOLDERS.contains(&n.folder.as_str()) {
                continue;
            }
            let dir = vault.join(&n.folder).join("imported");
            crate::secure_file::create_private_dir(&dir).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not make the memory folder: {e}.")))?;
            let mut target = dir.join(format!("{}.md", slug(&n.name, 60)));
            let mut k = 2;
            while target.exists() {
                target = dir.join(format!("{}-{k}.md", slug(&n.name, 56)));
                k += 1;
            }
            crate::secure_file::write_private(&target, &format!("{}\n", n.text)).map_err(|e| StoreError::new("POTATO_STORE_INVALID", e))?;
            done.notes += 1;
        }
    }
    Ok(done)
}

// ---- making a bundle from a bot ---------------------------------------------------------------------------------------

/// The bundle of a bot. `memory`: include the notes (newest first, under the size limits).
pub fn export(root: &Path, bot: &str, info: BotInfo, memory: bool, now_ms: i64) -> StoreResult<Bundle> {
    store::require_bot(root, bot)?;
    let settings = bot_settings::load(root, bot)?;
    let mut b = Bundle { format: FORMAT.into(), version: VERSION, exported_ms: now_ms, bot: info, instructions: bot_settings::current_instructions(&settings).to_string(), ..Bundle::default() };
    for s in skills::list(root, bot)? {
        b.skills.push(SkillItem { name: s.name, description: s.description, body: s.body });
    }
    for r in routines::list(root, Some(bot))? {
        if !r.done && r.watch.is_none() {
            b.routines.push(RoutineItem { title: r.title, prompt: r.prompt, schedule: r.schedule });
        }
    }
    if memory {
        for note in memory_notes::list(root, bot)?.into_iter().take(MAX_NOTES) {
            let Ok(text) = memory_notes::read(root, bot, &note.path) else { continue };
            let (folder, rest) = note.path.split_once('/').unwrap_or(("journal", &note.path));
            let name = rest.trim_end_matches(".md").replace('/', "-");
            b.memory.push(NoteItem { folder: folder.to_string(), name, text });
        }
    }
    let mut notes = Vec::new();
    clean(&mut b, &mut notes);
    Ok(b)
}

/// The text of a bundle as it is written to disk.
pub fn to_text(b: &Bundle) -> StoreResult<String> {
    let mut value = serde_json::to_value(b).map_err(|e| invalid(format!("Could not write the file: {e}.")))?;
    value["routines"] = serde_json::to_value(&b.routines).map_err(|e| invalid(format!("Could not write the file: {e}.")))?;
    serde_json::to_string_pretty(&value).map_err(|e| invalid(format!("Could not write the file: {e}.")))
}

/// A file name for the bundle: `<name>.potato-bot.json`, made of letters and digits.
pub fn file_name(name: &str) -> String {
    let s = slug(name, 40);
    format!("{}.potato-bot.json", if s.is_empty() { "bot".to_string() } else { s })
}

/// A bundle made of drafts the person reviewed: the instructions, and each memory as a note in `nodes`.
pub fn drafts_bundle(instructions: &str, memories: &[String]) -> Bundle {
    let mut b = Bundle { format: FORMAT.into(), version: VERSION, ..Bundle::default() };
    b.instructions = instructions.to_string();
    for (i, m) in memories.iter().take(MAX_NOTES).enumerate() {
        b.memory.push(NoteItem { folder: "nodes".into(), name: format!("pasted-{}-{}", i + 1, slug(m, 24)), text: m.clone() });
    }
    let mut notes = Vec::new();
    clean(&mut b, &mut notes);
    b
}

// ---- the whole account ------------------------------------------------------------------------------------------------

pub const ACCOUNT_FORMAT: &str = "potato-account";
const README: &str = "This folder is a copy of your Potato Bots: for each bot its name, voice, instructions, skills, routines, memory notes and chat.\n\nIt does NOT hold any key, token, password, cookie or connector sign-in, and the words in it were scrubbed of anything that looks like one. On a new PC you sign in and add your keys again.\n\nTo bring it back: in Puppet Master open Potato Bot > New bot > Restore from a folder, and pick this folder.\n";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountManifest {
    pub format: String,
    pub version: u32,
    pub exported_ms: i64,
    /// Folder names under `bots/`.
    pub bots: Vec<String>,
    pub secrets_included: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountBot {
    /// The folder name, which is the old bot id.
    pub id: String,
    pub bot: BotInfo,
    pub skills: usize,
    pub routines: usize,
    pub notes: usize,
    pub messages: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountPreview {
    pub exported_ms: i64,
    pub bots: Vec<AccountBot>,
    pub notes: Vec<String>,
    pub promise: String,
}

pub const ACCOUNT_PROMISE: &str = "This copy holds no keys, tokens or sign-ins. You add those again on this PC.";

/// One chat line as written to a file: who said it and what, nothing else (no cards, files, costs or ids).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ChatLine {
    pub at: i64,
    /// `you` or `bot`.
    pub from: String,
    pub text: String,
}

pub fn chat_lines(messages: &[Message]) -> Vec<ChatLine> {
    messages
        .iter()
        .filter(|m| m.kind == MsgKind::Text && matches!(m.from.kind, FromKind::You | FromKind::Bot))
        .map(|m| ChatLine { at: m.at, from: if m.from.kind == FromKind::You { "you" } else { "bot" }.to_string(), text: store::clean_text(&m.text, None) })
        .collect()
}

/// Writes the account folder `<parent>/potato-bots-<stamp>` and returns its path. One entry per bot: `info` is looked up by id.
pub fn export_account(root: &Path, parent: &Path, infos: &dyn Fn(&str) -> BotInfo, chats: bool, now_ms: i64) -> StoreResult<PathBuf> {
    let dir = parent.join(format!("potato-bots-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    if dir.exists() {
        return Err(invalid("A copy with that name is already there. Try again in a second."));
    }
    let bots = store::load_bots(root)?;
    let io = |what: &str, e: std::io::Error| StoreError::new("POTATO_EXPORT_IO", format!("Could not {what}: {e}."));
    fs::create_dir_all(dir.join("bots")).map_err(|e| io("make the folder", e))?;
    let mut ids = Vec::new();
    for bot in &bots {
        let id = slug(&bot.id, 40);
        if id.is_empty() {
            continue;
        }
        let bundle = export(root, &bot.id, infos(&bot.id), true, now_ms)?;
        let mut bundle = bundle;
        bundle.bot.talk_persona = bot.talk_persona.clone();
        let bdir = dir.join("bots").join(&id);
        fs::create_dir_all(&bdir).map_err(|e| io("make a bot folder", e))?;
        fs::write(bdir.join("bot.potato-bot.json"), to_text(&bundle)?).map_err(|e| io("write a bot", e))?;
        if chats {
            let messages = store::read_messages(root, &store::app_conversation(&bot.id), bot_settings::EXPORT_MESSAGES, None)?;
            let lines = chat_lines(&messages);
            let skip = lines.len().saturating_sub(MAX_CHAT_MESSAGES);
            let mut out = String::new();
            for l in &lines[skip..] {
                out.push_str(&serde_json::to_string(l).map_err(|e| invalid(e.to_string()))?);
                out.push('\n');
            }
            fs::write(bdir.join("chat.jsonl"), out).map_err(|e| io("write a chat", e))?;
        }
        ids.push(id);
    }
    let manifest = AccountManifest { format: ACCOUNT_FORMAT.into(), version: VERSION, exported_ms: now_ms, bots: ids, secrets_included: false };
    fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).map_err(|e| invalid(e.to_string()))?).map_err(|e| io("write the list", e))?;
    fs::write(dir.join("README.txt"), README).map_err(|e| io("write the note", e))?;
    Ok(dir)
}

fn bot_dir(dir: &Path, id: &str) -> StoreResult<PathBuf> {
    if id.is_empty() || id != slug(id, 40) {
        return Err(invalid("That bot is not in this copy."));
    }
    Ok(dir.join("bots").join(id))
}

fn read_inside(dir: &Path, file: &Path) -> StoreResult<String> {
    // The folder the person picked is the only trusted part; every file below it is checked.
    let meta = fs::symlink_metadata(file).map_err(|_| invalid("A file in this copy is missing."))?;
    if meta.file_type().is_symlink() {
        return Err(invalid("This copy has a link in it, so it was not opened."));
    }
    let _ = dir;
    read_capped(file)
}

fn read_manifest(dir: &Path) -> StoreResult<AccountManifest> {
    let text = read_inside(dir, &dir.join("manifest.json"))?;
    let m: AccountManifest = serde_json::from_str(&text).map_err(|_| invalid("That folder is not a Potato Bot copy."))?;
    if m.format != ACCOUNT_FORMAT {
        return Err(invalid("That folder is not a Potato Bot copy."));
    }
    if m.version == 0 || m.version > VERSION {
        return Err(invalid("That copy was made by a newer Puppet Master. Update first."));
    }
    Ok(m)
}

fn read_chat(path: &Path) -> Vec<ChatLine> {
    let Ok(text) = read_capped(path) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str::<ChatLine>(l).ok()).filter(|l| l.from == "you" || l.from == "bot").take(MAX_CHAT_MESSAGES).collect()
}

/// What a copy folder holds, without writing anything.
pub fn inspect_account(dir: &Path) -> StoreResult<AccountPreview> {
    let manifest = read_manifest(dir)?;
    let mut bots = Vec::new();
    let mut notes = Vec::new();
    for id in manifest.bots.iter().take(store::MAX_BOTS) {
        let Ok(bdir) = bot_dir(dir, id) else { continue };
        match read_inside(dir, &bdir.join("bot.potato-bot.json")).and_then(|t| parse(&t)) {
            Ok((b, more)) => {
                notes.extend(more.into_iter().map(|n| format!("{}: {n}", b.bot.name)));
                let messages = if bdir.join("chat.jsonl").exists() { read_chat(&bdir.join("chat.jsonl")).len() } else { 0 };
                bots.push(AccountBot { id: id.clone(), skills: b.skills.len(), routines: b.routines.len(), notes: b.memory.len(), messages, bot: b.bot });
            }
            Err(e) => notes.push(format!("{id} could not be read: {}", e.message)),
        }
    }
    if manifest.bots.len() > store::MAX_BOTS {
        notes.push(format!("Only the first {} bots are listed.", store::MAX_BOTS));
    }
    Ok(AccountPreview { exported_ms: manifest.exported_ms, bots, notes, promise: ACCOUNT_PROMISE.to_string() })
}

/// The cleaned bundle of one bot of a copy folder.
pub fn account_bundle(dir: &Path, id: &str) -> StoreResult<Bundle> {
    let manifest = read_manifest(dir)?;
    if !manifest.bots.iter().any(|b| b == id) {
        return Err(invalid("That bot is not in this copy."));
    }
    let bdir = bot_dir(dir, id)?;
    Ok(parse(&read_inside(dir, &bdir.join("bot.potato-bot.json"))?)?.0)
}

/// Puts a copy's chat back into a bot's private chat, as plain lines. Cards, files and costs are not kept.
pub fn restore_chat(root: &Path, dir: &Path, id: &str, bot: &str) -> StoreResult<usize> {
    store::require_bot(root, bot)?;
    let manifest = read_manifest(dir)?;
    if !manifest.bots.iter().any(|b| b == id) {
        return Err(invalid("That bot is not in this copy."));
    }
    let path = bot_dir(dir, id)?.join("chat.jsonl");
    if !path.exists() {
        return Ok(0);
    }
    let lines = read_chat(&path);
    let conversation = store::app_conversation(bot);
    let mut n = 0;
    for l in lines {
        let (kind, name) = if l.from == "you" { (FromKind::You, None) } else { (FromKind::Bot, None) };
        let text: String = l.text.chars().take(store::MAX_MESSAGE_CHARS).collect();
        if text.trim().is_empty() {
            continue;
        }
        store::append_message(root, &conversation, Message::new(l.at, kind, name, MsgKind::Text, &text), None)?;
        n += 1;
    }
    Ok(n)
}

// ---- the engine's side ------------------------------------------------------------------------------------------------

impl Service {
    /// The bundle of a bot as file text, and the name to offer for it.
    pub fn export_bot_text(&self, bot: &str, info: BotInfo, memory: bool) -> StoreResult<(String, String)> {
        self.require_bot(bot)?;
        let mut info = info;
        info.name = self.name_of(bot);
        let talk = store::load_bots(&self.root)?.into_iter().find(|b| b.id == bot).map(|b| b.talk_persona).unwrap_or_default();
        info.talk_persona = talk;
        let bundle = export(&self.root, bot, info, memory, self.host.now_ms())?;
        Ok((to_text(&bundle)?, file_name(&bundle.bot.name)))
    }

    /// What a file holds, for the preview. Reads at most 2 MB; writes nothing.
    pub fn inspect_bot_file(&self, path: &Path) -> StoreResult<Preview> {
        let (bundle, notes) = parse(&read_capped(path)?)?;
        Ok(preview_of(&bundle, notes))
    }

    /// Puts the ticked parts of a file into a bot that was just made (the page makes the companion and the bot first).
    pub fn apply_bot_file(&self, bot: &str, path: &Path, choices: Choices) -> StoreResult<Applied> {
        self.require_bot(bot)?;
        let (bundle, _) = parse(&read_capped(path)?)?;
        let applied = apply(&self.root, bot, &bundle, choices, self.host.now_ms())?;
        let _ = store::set_talk_persona(&self.root, bot, &bundle.bot.talk_persona);
        self.host.changed(bot);
        Ok(applied)
    }

    /// A copy of a bot's content into another bot (the page made the other one): instructions, skills, routines (paused) and memory.
    pub fn copy_bot_into(&self, from: &str, to: &str) -> StoreResult<Applied> {
        self.require_bot(from)?;
        self.require_bot(to)?;
        let info = BotInfo { name: self.name_of(from), ..BotInfo::default() };
        let bundle = export(&self.root, from, info, true, self.host.now_ms())?;
        let applied = apply(&self.root, to, &bundle, Choices::ALL, self.host.now_ms())?;
        self.host.changed(to);
        Ok(applied)
    }

    /// The drafts a person reviewed after pasting from another assistant: instructions, and one memory note per line they kept.
    pub fn save_drafts(&self, bot: &str, instructions: &str, memories: &[String]) -> StoreResult<Applied> {
        self.require_bot(bot)?;
        let now = self.host.now_ms();
        let b = drafts_bundle(instructions, memories);
        let applied = apply(&self.root, bot, &b, Choices { instructions: true, skills: false, routines: false, memory: true }, now)?;
        self.host.changed(bot);
        Ok(applied)
    }

    pub fn account_export(&self, parent: &Path, infos: &dyn Fn(&str) -> BotInfo, chats: bool) -> StoreResult<PathBuf> {
        export_account(&self.root, parent, infos, chats, self.host.now_ms())
    }

    pub fn account_inspect(&self, dir: &Path) -> StoreResult<AccountPreview> {
        inspect_account(dir)
    }

    /// One bot of a copy into a bot the page just made: its parts and its chat.
    pub fn account_restore_bot(&self, dir: &Path, id: &str, bot: &str) -> StoreResult<Applied> {
        self.require_bot(bot)?;
        let bundle = account_bundle(dir, id)?;
        let mut applied = apply(&self.root, bot, &bundle, Choices::ALL, self.host.now_ms())?;
        applied.chat = restore_chat(&self.root, dir, id, bot)?;
        self.host.changed(bot);
        Ok(applied)
    }
}
