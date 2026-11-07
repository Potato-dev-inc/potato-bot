//! Per-bot settings of Potato Bot (the bot panel's Advanced tab): which talk brain the bot uses (or the
//! default), its extra instructions with their history, and its tool switches. One small file per bot,
//! `<app data>/potato/bot-settings/<bot>.json`, written under the store's lock.
//!
//! - the brain: `None` is "Use the default" (the Settings brain). The engine (`service.rs`) reads it when a
//!   brain starts for that bot;
//! - the instructions: every save is a version (newest last, at most 50). Restore adds the old text as a new
//!   version, so nothing is ever lost. The text in use is written to `brain/<bot>/persona/instructions.md` when
//!   a brain starts, next to the persona files, and the brain reads it like them;
//! - the tools: three switches, all on by default. `helpers` = local helper agents (`ask_helper`); `computer` =
//!   the sandbox screen tools; `web` = the sandbox browser tools. The engine checks them before a tool call runs.
//!
//! Deleting a bot moves its chat, its settings and its brain folder into `potato/.removed/`, never deletes them.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::TimeZone;
use serde::{Deserialize, Serialize};

use super::risk::Level;
use super::store::{self, BrainKind, CustomProvider, Message, StoreError, StoreResult};
use crate::secure_file;

/// The most characters one instruction version may hold (the brain reads it as persona text).
pub const MAX_INSTRUCTION_CHARS: usize = 4000;
/// Versions kept per bot; the oldest go first.
pub const MAX_VERSIONS: usize = 50;
/// The most characters a bot's name may have (the companion name cap).
pub const MAX_NAME_CHARS: usize = 40;
/// The file the brain reads for a bot's extra instructions, in its persona folder.
pub const INSTRUCTIONS_FILE: &str = "instructions.md";
const SETTINGS_VERSION: u32 = 1;
/// The most messages a chat export reads (the old file and the current one together).
pub const EXPORT_MESSAGES: usize = 10_000;

/// The three tool switches of a bot. All on by default; a switch that is off refuses its tools at the engine.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolSwitches {
    /// Local helper agents: `ask_helper`.
    pub helpers: bool,
    /// The sandbox screen: `computer_observe`, `computer_act`, `ask_person_for_screen`.
    pub computer: bool,
    /// Reading the web in the sandbox: `browser_navigate`, `browser_snapshot`, `browser_act`, and `web_fetch` (page text).
    pub web: bool,
}

impl Default for ToolSwitches {
    fn default() -> Self {
        ToolSwitches { helpers: true, computer: true, web: true }
    }
}

/// A tool server the person added for the bot's computer (see `mcp.rs`). The env VALUES are not here: they are in the
/// credential store under `potato_mcp_env_<bot>_<name>`, and this keeps only their names.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env_keys: Vec<String>,
    pub enabled: bool,
    /// Added from the Connectors list (a service that needs the person's token). Its calls follow the connector rules
    /// in `sandbox_calls`: owner's private chat only, never in a room.
    pub connector: bool,
    /// Connector only: read-shaped calls run without a card. Off: each read asks, like a change does.
    pub always_allow_reads: bool,
    /// A remote tool server by link (`remote_mcp.rs`): the https link. Empty for a server that runs in the computer. A remote
    /// server is called by this app on the PC, so it has no command and never reaches the computer's list.
    pub url: String,
    /// Remote only: the person ticked Local, so a link to this computer or their own network (and plain http there) is allowed.
    pub local: bool,
    /// The tools of this server the bot may call. Empty: every tool. Checked by the host at call time (`sandbox_calls`), for
    /// a server in the computer and a server by link alike; no tool the model can call reads or writes it.
    pub allow_tools: Vec<String>,
    /// Tools of this server the bot may never call, even when the allow list names them.
    pub block_tools: Vec<String>,
    /// By link only: the person signed in with the server's own sign-in (`mcp_oauth.rs`). The tokens are in the credential store.
    pub oauth: bool,
}

impl McpServer {
    /// Whether the bot may call `tool` on this server: not blocked, and listed when there is an allow list.
    pub fn tool_allowed(&self, tool: &str) -> bool {
        !self.block_tools.iter().any(|t| t == tool) && (self.allow_tools.is_empty() || self.allow_tools.iter().any(|t| t == tool))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionVersion {
    /// When it was saved (epoch ms). Unique within a bot, and the key Restore uses.
    pub at: i64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BotSettings {
    pub v: u32,
    /// The bot's own talk brain. `None`: the default in Settings.
    pub brain: Option<BrainKind>,
    /// The bot's own OpenAI-compatible provider (base URL and model). `None`: the default in Settings.
    /// Its key lives in the credential store under `potato_provider_key_<bot>`.
    pub custom: Option<CustomProvider>,
    /// The model this bot uses on the Go plan. `None`: the default in Settings.
    pub go_model: Option<String>,
    pub tools: ToolSwitches,
    /// How much the bot asks before it acts in its computer. Set only by the owner, through the app command
    /// `potato_bot_level_set`; no tool the model can call reads or writes it. Missing: ask when risky.
    pub level: Level,
    /// Oldest first. The last one is the text in use (empty when none was ever saved).
    pub instructions: Vec<InstructionVersion>,
    /// The tool servers the bot may use inside its computer (besides the built-in `computer` one).
    pub mcp_servers: Vec<McpServer>,
    /// "Always allow" rules: one exact command class this bot may run without a card. Written only by the app command
    /// behind the button on an approval card (`cards::answer_always`) and removed only by the Advanced tab's Remove;
    /// no tool the model can call reads or writes it.
    pub always_allow: Vec<AlwaysRule>,
    /// The owner's limit on the sites the bot's web tools open. Off by default. Set only by the app command
    /// `potato_bot_sites_set`; no tool the model can call reads or writes it.
    pub site_limit: SiteLimit,
}

/// The most sites one list holds.
pub const MAX_SITES: usize = 50;

/// "Only these sites": the owner's allowlist for the bot's web tools (`web_fetch`, `browser_navigate` and opening a web address
/// with `computer_act`). Off by default. A site includes its subdomains. It limits what those tools OPEN; it is not a firewall
/// (see `site_limit_note`): the terminal, links inside pages and the apps in the computer are not covered.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SiteLimit {
    pub enabled: bool,
    pub sites: Vec<String>,
}

/// Hosting services where one name covers everyone's pages: listing the whole name would allow anybody's page on it.
const SHARED_HOSTS: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "org.au", "co.jp", "co.nz", "co.in", "co.kr", "co.za", "com.br", "com.cn", "com.mx",
    "github.io", "githubusercontent.com", "vercel.app", "netlify.app", "pages.dev", "workers.dev", "web.app", "firebaseapp.com",
    "herokuapp.com", "blogspot.com", "wordpress.com", "appspot.com", "azurewebsites.net", "cloudfront.net", "amazonaws.com",
];

/// One site as the owner typed it, made into a plain host name: no address start, no path, no port, lower case, without a
/// leading `www.`-style wildcard (`*.example.com` means `example.com`). Refused: numbers, hosting services that cover
/// everybody's pages, and anything that is not a plain name.
pub fn clean_site(raw: &str) -> StoreResult<String> {
    let bad = |message: &str| StoreError::new("POTATO_SITE_INVALID", message.to_string());
    let mut text = raw.trim().to_ascii_lowercase();
    for start in ["https://", "http://", "*."] {
        if let Some(rest) = text.strip_prefix(start).map(str::to_string) {
            text = rest;
        }
    }
    let end = text.find(['/', '?', '#']).unwrap_or(text.len());
    let host = text[..end].trim_end_matches('.').to_string();
    if host.is_empty() {
        return Err(bad("Type a site name like example.com."));
    }
    if host.contains(['@', ':']) || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad("Type just the site name, like example.com (no port, no password)."));
    }
    if !host.is_ascii() {
        return Err(bad("Type a site with a non-English name in its plain form (xn--...)."));
    }
    let labels: Vec<&str> = host.split('.').collect();
    let plain = labels.len() >= 2
        && host.len() <= 253
        && labels.iter().all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    if !plain {
        return Err(bad("Type a site name like example.com."));
    }
    if labels.iter().all(|l| l.bytes().all(|b| b.is_ascii_digit())) {
        return Err(bad("Type a site name, not a number."));
    }
    if SHARED_HOSTS.contains(&host.as_str()) {
        return Err(bad("That name covers everyone's pages on a hosting service. Type the exact site, like yourname.github.io."));
    }
    Ok(host)
}

/// A whole list: each site cleaned, repeats dropped, at most 50.
pub fn clean_sites(raw: &[String]) -> StoreResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw.iter().filter(|s| !s.trim().is_empty()) {
        let site = clean_site(entry)?;
        if !out.contains(&site) {
            out.push(site);
        }
    }
    if out.len() > MAX_SITES {
        return Err(StoreError::new("POTATO_SITE_INVALID", format!("A list can hold {MAX_SITES} sites at most.")));
    }
    Ok(out)
}

/// May a bot's web tool open this host? With the limit off, yes. With it on: the host is a listed site or a subdomain of one.
/// `www.example.com` and `a.b.example.com` are `example.com`'s; `example.com.evil.net` and `notexample.com` are not.
pub fn site_allowed(limit: &SiteLimit, host: &str) -> bool {
    if !limit.enabled {
        return true;
    }
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    !host.is_empty() && limit.sites.iter().any(|site| host == *site || host.ends_with(&format!(".{site}")))
}

/// The plain sentence a refused address gets.
pub fn site_refusal(host: &str) -> String {
    format!("The owner limits this bot to certain sites, and {host} is not one of them. Ask the owner to add it in this bot's Advanced tab.")
}

/// One "Always allow" rule: a command class (`risk.rs`) and when the owner chose it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AlwaysRule {
    pub class: String,
    pub at: i64,
}

/// A saved rule as the Advanced tab shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlwaysRuleView {
    pub class: String,
    /// The class in plain words ("pushes to a remote").
    pub description: String,
    pub at: i64,
}

/// Does the bot have a rule for every one of these classes, each a class that may be always allowed? An empty list never
/// is. A settings file that cannot be read says no.
pub fn always_allows(root: &Path, bot: &str, classes: &[&str]) -> bool {
    if classes.is_empty() {
        return false;
    }
    let Ok(settings) = load(root, bot) else { return false };
    classes.iter().all(|class| super::risk::always_allowable(class) && settings.always_allow.iter().any(|r| r.class == *class))
}

/// What the Advanced tab reads in one call. Versions come newest first.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BotOptions {
    pub brain: Option<BrainKind>,
    /// The bot's own provider. `None`: the default in Settings.
    pub custom: Option<CustomProvider>,
    /// The bot's own Go plan model. `None`: the default in Settings.
    pub go_model: Option<String>,
    pub tools: ToolSwitches,
    pub level: Level,
    /// The sandbox Labs switch is on: the computer switch is shown only then.
    pub sandbox_on: bool,
    pub instructions: String,
    pub versions: Vec<InstructionVersion>,
    /// The saved "Always allow" rules, oldest first.
    pub always_allow: Vec<AlwaysRuleView>,
    /// The owner's limit on the sites the bot's web tools open.
    pub site_limit: SiteLimit,
}

/// A brain's name in a note, as the person reads it. Generic code names no agent or provider (see `guard_tests`), so the
/// Go plan and the person's agents are named by what they are for; the Settings list has the full labels.
pub fn brain_name(kind: BrainKind) -> &'static str {
    match kind {
        BrainKind::Deepseek => "DeepSeek",
        BrainKind::Go => "your Go plan",
        BrainKind::Agents => "your agents",
        BrainKind::Custom => "your provider",
    }
}

pub fn settings_dir(root: &Path) -> PathBuf {
    root.join("bot-settings")
}

/// `<root>/bot-settings/<bot>.json`; refused for an id that is not well formed.
pub fn path(root: &Path, bot: &str) -> StoreResult<PathBuf> {
    if !crate::companion::model::valid_id(bot) {
        return Err(StoreError::new("POTATO_BOT_NOT_FOUND", "No bot has that name."));
    }
    Ok(settings_dir(root).join(format!("{bot}.json")))
}

/// A missing file reads as the defaults; a damaged one is an error (and is never overwritten).
pub fn load(root: &Path, bot: &str) -> StoreResult<BotSettings> {
    Ok(store::read_json(&path(root, bot)?, "the bot's settings")?.unwrap_or_default())
}

fn save(root: &Path, bot: &str, settings: &BotSettings) -> StoreResult<()> {
    let target = path(root, bot)?;
    secure_file::create_private_dir(&settings_dir(root)).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not make the settings folder: {e}.")))?;
    let mut copy = settings.clone();
    copy.v = SETTINGS_VERSION;
    store::write_json(&target, &copy, "the bot's settings")
}

/// Read, change and write under the store's one lock, so two changes cannot lose each other.
pub fn update(root: &Path, bot: &str, change: impl FnOnce(&mut BotSettings) -> StoreResult<()>) -> StoreResult<BotSettings> {
    let _files = store::files_lock();
    let mut settings = load(root, bot)?;
    change(&mut settings)?;
    save(root, bot, &settings)?;
    Ok(settings)
}

/// The brain a bot starts with: its own, or the default.
pub fn effective_brain(default: BrainKind, settings: &BotSettings) -> BrainKind {
    settings.brain.unwrap_or(default)
}

/// Trims a provider draft and keeps it unless it is empty. A half-filled one is kept, so the page can show what
/// the person typed; a brain that needs it refuses to start until it is ready (`CustomProvider::ready`).
pub fn clean_provider(custom: Option<CustomProvider>) -> Option<CustomProvider> {
    let custom = custom?;
    let base_url = custom.base_url.trim().trim_end_matches('/').to_string();
    let model = custom.model.trim().to_string();
    (!base_url.is_empty() || !model.is_empty()).then_some(CustomProvider { base_url, model })
}

// ---- instructions -------------------------------------------------------------------------------

/// The text in use: the newest version, or nothing.
pub fn current_instructions(settings: &BotSettings) -> &str {
    settings.instructions.last().map(|v| v.text.as_str()).unwrap_or("")
}

/// Trims the text and checks its length. The check is on what the person typed, before any cleaning.
pub fn clean_instructions(raw: &str) -> StoreResult<String> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > MAX_INSTRUCTION_CHARS {
        return Err(StoreError::new("POTATO_INSTRUCTIONS_TOO_LONG", format!("Instructions can be {MAX_INSTRUCTION_CHARS} characters at most.")));
    }
    Ok(store::clean_text(trimmed, None))
}

/// Adds a version with `text`, unless it is the text in use already. Returns whether a version was added.
pub fn save_instructions(settings: &mut BotSettings, raw: &str, now: i64) -> StoreResult<bool> {
    let text = clean_instructions(raw)?;
    if text == current_instructions(settings) {
        return Ok(false);
    }
    push_version(settings, text, now);
    Ok(true)
}

/// Makes the version saved at `at` the one in use, by adding its text as a new version. Returns whether that
/// changed anything (restoring the text already in use does nothing).
pub fn restore_instructions(settings: &mut BotSettings, at: i64, now: i64) -> StoreResult<bool> {
    let text = settings
        .instructions
        .iter()
        .find(|v| v.at == at)
        .map(|v| v.text.clone())
        .ok_or_else(|| StoreError::new("POTATO_VERSION_NOT_FOUND", "That version of the instructions is gone."))?;
    if text == current_instructions(settings) {
        return Ok(false);
    }
    push_version(settings, text, now);
    Ok(true)
}

fn push_version(settings: &mut BotSettings, text: String, now: i64) {
    // Two saves in the same millisecond still get two different keys.
    let at = match settings.instructions.last() {
        Some(last) if last.at >= now => last.at + 1,
        _ => now,
    };
    settings.instructions.push(InstructionVersion { at, text });
    let extra = settings.instructions.len().saturating_sub(MAX_VERSIONS);
    settings.instructions.drain(..extra);
}

/// The Advanced tab's view of a bot's settings: versions newest first.
pub fn options_of(settings: &BotSettings, sandbox_on: bool) -> BotOptions {
    BotOptions {
        brain: settings.brain,
        custom: settings.custom.clone(),
        go_model: settings.go_model.clone(),
        tools: settings.tools,
        level: settings.level,
        sandbox_on,
        instructions: current_instructions(settings).to_string(),
        versions: settings.instructions.iter().rev().cloned().collect(),
        always_allow: settings.always_allow.iter().map(|r| AlwaysRuleView { class: r.class.clone(), description: super::risk::class_description(&r.class), at: r.at }).collect(),
        site_limit: settings.site_limit.clone(),
    }
}

/// Writes the instructions in use into the bot's persona folder (`instructions.md`), or removes the file when
/// there are none, so a text that was cleared never comes back. Called when a brain starts.
pub fn write_instructions_file(persona_dir: &Path, settings: &BotSettings) -> StoreResult<()> {
    let file = persona_dir.join(INSTRUCTIONS_FILE);
    let text = current_instructions(settings);
    if text.is_empty() {
        return match fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::new("POTATO_STORE_INVALID", format!("Could not remove the instructions file: {e}."))),
        };
    }
    secure_file::write_private(&file, text).map_err(|e| StoreError::new("POTATO_STORE_INVALID", e))
}

// ---- tools --------------------------------------------------------------------------------------

/// The sandbox tools, by the switch that gates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Computer,
    Web,
}

pub fn sandbox_tool_area(tool: &str) -> Option<Area> {
    match tool {
        "computer_observe" | "computer_act" | "ask_person_for_screen" | "run_terminal" | "read_file" | "write_file" | "edit_file"
        | "list_files" | "set_wallpaper" | "mcp_list_tools" | "mcp_call_tool" | "send_file" => Some(Area::Computer),
        "browser_navigate" | "browser_snapshot" | "browser_act" | "web_fetch" => Some(Area::Web),
        _ => None,
    }
}

/// Whether a sandbox tool may run for a bot. A name that is not a sandbox tool is left to the tool layer
/// (it answers "no such tool").
pub fn sandbox_tool_on(tools: &ToolSwitches, tool: &str) -> bool {
    match sandbox_tool_area(tool) {
        Some(Area::Computer) => tools.computer,
        Some(Area::Web) => tools.web,
        None => true,
    }
}

// ---- names --------------------------------------------------------------------------------------

/// A bot's name as typed: trimmed, not empty, at most 40 characters.
pub fn clean_name(raw: &str) -> StoreResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(StoreError::new("POTATO_NAME_EMPTY", "Give the bot a name."));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(StoreError::new("POTATO_NAME_TOO_LONG", format!("Names can be {MAX_NAME_CHARS} characters at most.")));
    }
    Ok(name.to_string())
}

// ---- chat export --------------------------------------------------------------------------------

/// The chat as one Markdown file: a title, then one block per message, oldest first. The person's messages and
/// the bot's are labelled; notes are in italics. Times are local.
pub fn chat_markdown(bot_name: &str, messages: &[Message]) -> String {
    let mut out = format!("# Chat with {bot_name}\n\n");
    for m in messages {
        let when = chrono::Local
            .timestamp_millis_opt(m.at)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let who = match m.from.kind {
            store::FromKind::You => "You".to_string(),
            store::FromKind::Note => String::new(),
            _ => m.from.name.clone().unwrap_or_else(|| bot_name.to_string()),
        };
        match m.from.kind {
            store::FromKind::Note => out.push_str(&format!("*{when} · {}*\n\n", m.text.trim())),
            _ => out.push_str(&format!("**{who}** · {when}\n\n{}\n\n", m.text.trim())),
        }
    }
    out
}

// ---- deleting -----------------------------------------------------------------------------------

