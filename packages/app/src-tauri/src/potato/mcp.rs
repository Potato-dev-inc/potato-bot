//! MCP servers for Potato Bot's computer: the tool servers that run inside the sandbox.
//!
//! - The guest always has the built-in `computer` server. The person can add more: a catalog entry in one click, or
//!   their own (`name`, `command`, `args`, `env`). The list is per bot in `bot_settings` (`mcpServers`). Each env VALUE
//!   is a secret: it lives in the credential store under `potato_mcp_env_<bot>_<server>` (the same place as the bot's
//!   provider key) and the settings file keeps only the key names. Values are never logged and never returned.
//! - The guest's list is one list for the whole sandbox: `guest_config` joins the enabled servers of every bot,
//!   first name wins. The list is pushed (PUT /v1/mcp/config) after each change while the sandbox runs, and after
//!   each start (`Manager::start`).
//! - `answer` turns the guest's `{content, isError}` into what the brain reads: text, and the first picture lifted out
//!   the way `computer_observe` lifts its own.
//!
//! The calls themselves go through `sandbox_calls` (the same gates as `computer_act`); this file only holds the
//! names, the catalog, the settings and the answers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::bot_settings::{self, McpServer};
use super::remote_mcp::{self, RemoteError};
use super::sandbox::{Sandbox, State};
use super::store::{StoreError, StoreResult};
use crate::secrets;

/// The longest tool name the brain may send.
pub const MAX_TOOL_CHARS: usize = 200;
/// Most servers one list (per bot, and the guest's whole list) may hold.
pub const MAX_SERVERS: usize = 16;
/// Most characters of text one tool answer keeps.
const MAX_TEXT_CHARS: usize = 20_000;
/// A picture bigger than this (base64 characters) is not passed on.
const MAX_IMAGE_CHARS: usize = 2_000_000;
const MAX_ARGS: usize = 32;
const MAX_ARG_CHARS: usize = 512;
const MAX_COMMAND_CHARS: usize = 256;
const MAX_ENV_VALUE_CHARS: usize = 4096;
/// The server the guest always has.
pub const BUILTIN: &str = "computer";

/// `^[a-z0-9_-]{1,32}$`: the name a server is known by (and the `server` a call names).
pub fn valid_server_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// An environment variable name: a letter or `_`, then letters, digits or `_`, up to 64.
pub fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && key.len() <= 64
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One one-click server. The catalog file (`packages/potato-sandbox/recipes/mcp-catalog.json`) is the one source: the
/// guest installs the npm packages it names in the base recipe, and `command` is the bin name they provide.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: String,
    pub title: String,
    pub description: String,
    /// A connector's command is worked out from its `install` (see `CatalogEntry::command_path`); the file leaves it out.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// A service that needs the person's token (GitHub, Notion): listed under Connectors, installed once on connect.
    #[serde(default)]
    pub connector: bool,
    /// The values the person types for a connector, one field each (the env variable name and what to call it).
    #[serde(default)]
    pub env: Vec<EnvField>,
    #[serde(default)]
    pub install: Option<Install>,
    /// Who makes the server, in plain words (and whether it is still looked after).
    #[serde(default)]
    pub maker: Option<String>,
    /// The maker's own page the entry was taken from.
    #[serde(default)]
    pub docs: Option<String>,
}

/// One value a connector needs: the env variable the server reads, and how the page labels it.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct EnvField {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub help: String,
}

/// How a connector's server gets onto the computer, once, from a fixed pinned source in the catalog file.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Install {
    /// `npm install` of one exact version, scripts off.
    Npm { spec: String, bin: String },
    /// A release archive from github.com, checked against the sha256 written in the catalog, one file taken out of it.
    Release { url: String, sha256: String, member: String, dir: String },
}

/// Where connector servers live on the computer: a root-owned folder (the bot is user pm and must not be able to change a
/// program that will hold a token), outside the image.
pub const CONNECTORS_DIR: &str = "/opt/pm-connectors";

impl CatalogEntry {
    /// The program the guest runs: for an installed connector its path under `CONNECTORS_DIR`, else `command`.
    pub fn command_path(&self) -> String {
        match &self.install {
            Some(Install::Npm { bin, .. }) => format!("{CONNECTORS_DIR}/node_modules/.bin/{bin}"),
            Some(Install::Release { dir, member, .. }) => format!("{CONNECTORS_DIR}/{dir}/{member}"),
            None => self.command.clone(),
        }
    }

    /// The one shell line that installs a connector (nothing when it is already there). Built only from the catalog's own
    /// fixed fields after they pass `install_fields_ok`; it never holds a token. `None` for an entry with no install.
    pub fn install_command(&self) -> Option<String> {
        let command = self.command_path();
        match self.install.as_ref()? {
            Install::Npm { spec, .. } => Some(format!(
                "umask 022 && test -x {command} || (mkdir -p {CONNECTORS_DIR} && npm install --prefix {CONNECTORS_DIR} --no-audit --no-fund --ignore-scripts {spec})"
            )),
            Install::Release { url, sha256, member, dir } => {
                let folder = format!("{CONNECTORS_DIR}/{dir}");
                Some(format!(
                    "umask 022 && test -x {command} || (mkdir -p {folder} && curl -fsSL {url} -o {folder}/download.tgz && echo '{sha256}  {folder}/download.tgz' | sha256sum -c - && tar -xzf {folder}/download.tgz -C {folder} {member} && rm -f {folder}/download.tgz && chmod 0755 {command})"
                ))
            }
        }
    }

    /// The install fields are plain characters only, so the line above cannot be bent into another command.
    pub fn install_fields_ok(&self) -> bool {
        let plain = |text: &str, extra: &str| !text.is_empty() && text.len() <= 300 && text.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
        match &self.install {
            None => true,
            Some(Install::Npm { spec, bin }) => {
                let (name, version) = spec.rsplit_once('@').unwrap_or(("", ""));
                plain(name, "@/._-") && !name.is_empty() && version.split('.').count() == 3 && version.split('.').all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) && plain(bin, "._-")
            }
            Some(Install::Release { url, sha256, member, dir }) => {
                url.starts_with("https://github.com/") && plain(url, ":/._-") && sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) && plain(member, "._-") && plain(dir, "._-")
            }
        }
    }
}

#[derive(Deserialize)]
struct CatalogFile {
    entries: Vec<CatalogEntry>,
}

const CATALOG_JSON: &str = include_str!("../../../../potato-sandbox/recipes/mcp-catalog.json");

/// The one-click entries. The file is checked by a test, so a broken one cannot ship.
pub fn catalog() -> &'static [CatalogEntry] {
    static ENTRIES: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| serde_json::from_str::<CatalogFile>(CATALOG_JSON).map(|file| file.entries).unwrap_or_default())
}

// ---- answers --------------------------------------------------------------------------------------

/// The brain's view of one MCP tool call: `ok`, `text`, and the first picture as `image` (data and mime), the same
/// place `computer_observe` puts its own. Other pictures are counted in `note`.
pub fn answer(reply: &Value) -> Value {
    let is_error = reply.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let mut texts: Vec<&str> = Vec::new();
    let mut image: Option<Value> = None;
    let mut dropped = 0usize;
    for item in reply.get("content").and_then(Value::as_array).into_iter().flatten() {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    texts.push(text);
                }
            }
            Some("image") => {
                let data = item.get("data").and_then(Value::as_str).unwrap_or("");
                let mime = item.get("mimeType").and_then(Value::as_str).unwrap_or("image/png");
                let fits = !data.is_empty() && data.len() <= MAX_IMAGE_CHARS && matches!(mime, "image/png" | "image/jpeg" | "image/webp");
                if fits && image.is_none() {
                    image = Some(json!({ "data": data, "mime": mime }));
                } else if !data.is_empty() {
                    dropped += 1;
                }
            }
            _ => {}
        }
    }
    let mut text = texts.join("\n");
    if text.chars().count() > MAX_TEXT_CHARS {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
    }
    let mut out = json!({ "ok": !is_error, "text": text });
    if dropped > 0 {
        out["note"] = json!(format!("{dropped} picture(s) the tool sent were not shown."));
    }
    if let Some(image) = image {
        out["image"] = image;
    }
    out
}

// ---- settings and secrets -------------------------------------------------------------------------

/// The credential-store name of one server's env values (the same store as the bot's provider key).
pub fn env_secret(bot: &str, server: &str) -> String {
    format!("potato_mcp_env_{bot}_{server}")
}

fn no_store() -> StoreError {
    StoreError::new("MCP_NO_STORE", "The credential store is not available, so the values cannot be kept.")
}

fn store_env(bot: &str, server: &str, env: &BTreeMap<String, String>) -> StoreResult<()> {
    let backend = secrets::backend();
    if !backend.available() {
        return Err(no_store());
    }
    let text = serde_json::to_string(env).map_err(|_| StoreError::new("MCP_NO_STORE", "The values could not be saved."))?;
    backend.set(&env_secret(bot, server), &text).map_err(|_| StoreError::new("MCP_NO_STORE", "The values could not be saved in the credential store."))
}

fn drop_env(bot: &str, server: &str) {
    let _ = secrets::backend().delete(&env_secret(bot, server));
}

/// The env values of one server, read for the guest's config. Missing or unreadable: none (the server then fails to start,
/// and its row says so). Never logged.
fn read_env(bot: &str, server: &McpServer) -> BTreeMap<String, String> {
    if server.env_keys.is_empty() {
        return BTreeMap::new();
    }
    secrets::backend()
        .get(&env_secret(bot, &server.name))
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<BTreeMap<String, String>>(&text).ok())
        .unwrap_or_default()
}

/// Bot ids that have a settings file, sorted.
fn bots(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(bot_settings::settings_dir(root))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().is_some_and(|e| e == "json")).then(|| path.file_stem().map(|s| s.to_string_lossy().to_string())).flatten()
        })
        .collect();
    names.sort();
    names
}

/// The servers to run, from the enabled servers of every bot, first name wins, at most `MAX_SERVERS`: the plain ones for
/// the control service (which the bot can reach, so it is never given a value) and the ones with keys for the secret hub
/// (a root process only the host talks to), each with its values read from the credential store.
fn plan(root: &Path) -> StoreResult<(Value, Value)> {
    let mut seen: Vec<String> = Vec::new();
    let mut plain: Vec<Value> = Vec::new();
    let mut secret: Vec<Value> = Vec::new();
    for bot in bots(root) {
        let settings = bot_settings::load(root, &bot)?;
        for server in settings.mcp_servers.iter().filter(|s| s.enabled && s.url.is_empty()) {
            if seen.contains(&server.name) || seen.len() >= MAX_SERVERS {
                continue;
            }
            seen.push(server.name.clone());
            if server.env_keys.is_empty() {
                plain.push(json!({ "name": server.name, "command": server.command, "args": server.args, "env": {} }));
            } else {
                secret.push(json!({ "name": server.name, "command": server.command, "args": server.args, "env": read_env(&bot, server) }));
            }
        }
    }
    Ok((Value::Array(plain), Value::Array(secret)))
}

/// The list the control service gets (no value in it, ever). Tests read it; the app sends it through `push`.
#[cfg(test)]
pub fn guest_config(root: &Path) -> StoreResult<Value> {
    plan(root).map(|(plain, _)| plain)
}

fn running(sandbox: &dyn Sandbox) -> bool {
    sandbox.status().map(|s| s.state == State::Running).unwrap_or(false)
}

/// Sends the guest's list when the sandbox runs. A failure is returned as text for the caller to log (never the values).
pub fn push(root: &Path, sandbox: &dyn Sandbox) -> Result<(), String> {
    if !running(sandbox) {
        return Ok(());
    }
    let (plain, secret) = plan(root).map_err(|e| e.message)?;
    // The servers with keys go to the root process over its own pipe; the control service never sees them.
    sandbox.secret_config(&secret).map_err(|e| e.message)?;
    sandbox.mcp_config(&plain).map_err(|e| e.message)
}

// ---- what the settings page shows -----------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinView {
    pub name: &'static str,
    pub ready: bool,
    pub reason: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    /// The same as `name`: the catalog is keyed by name.
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub command: String,
    pub args: Vec<String>,
    pub env_keys: Vec<String>,
    /// A service that needs a token (listed under Connectors) rather than a plain tool server.
    pub connector: bool,
    /// What the person types for it: the variable name, a label and a hint. Empty for a plain entry.
    pub fields: Vec<EnvField>,
    pub maker: Option<String>,
    pub docs: Option<String>,
    pub installed: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// The names of the env values only. The values are never returned.
    pub env_keys: Vec<String>,
    pub enabled: bool,
    pub connector: bool,
    pub always_allow_reads: bool,
    /// A remote tool server by link: the link (never a token). `None` for a server in the computer.
    pub url: Option<String>,
    /// Remote only: Local was ticked.
    pub local: bool,
    /// The tools the bot may call (empty: all) and the tools it may never call.
    pub allow_tools: Vec<String>,
    pub block_tools: Vec<String>,
    /// By link only: signed in with the server's own sign-in.
    pub signed_in: bool,
    /// `off` (switched off), `ready`, `failed`, `unknown` (the sandbox is not running, or it did not answer).
    pub status: &'static str,
    pub tool_count: Option<usize>,
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub builtin: BuiltinView,
    pub catalog: Vec<CatalogView>,
    pub servers: Vec<ServerView>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

/// What the page sends for a custom server. The env values leave the page once, here, and are not kept in the page.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// Tool names per server from the guest's `GET /v1/mcp/tools` answer: `servers: [{name, tools: [{name} | "name"]}]`.
/// One server as the guest reports it: `status` is `starting`, `ready` or `failed` (the hub's words), with its `error`
/// when it failed and its tool names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestServer {
    pub status: String,
    pub error: Option<String>,
    pub tools: Vec<String>,
}

fn tools_by_server(reply: &Value) -> BTreeMap<String, GuestServer> {
    let mut out = BTreeMap::new();
    for server in reply.get("servers").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = server.get("name").and_then(Value::as_str) else { continue };
        let tools = server
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.get("name").and_then(Value::as_str).or_else(|| t.as_str()).map(str::to_string))
            .collect();
        let status = server.get("status").and_then(Value::as_str).unwrap_or("unknown").to_string();
        let error = server.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(|e| crate::redact::text(e).into_inner());
        out.insert(name.to_string(), GuestServer { status, error, tools });
    }
    out
}

pub fn overview(root: &Path, bot: &str, sandbox: &dyn Sandbox) -> StoreResult<Overview> {
    let settings = bot_settings::load(root, bot)?;
    let on = running(sandbox);
    // The guest's tools, only when it runs (asking a stopped sandbox would start it).
    let tools: Result<BTreeMap<String, GuestServer>, String> = if on {
        sandbox.mcp_tools().map(|reply| tools_by_server(&reply)).map_err(|e| crate::redact::text(&e.message).into_inner())
    } else {
        Err(String::new())
    };
    let servers = settings
        .mcp_servers
        .iter()
        .map(|server| {
            let (status, tool_count, note) = if !server.enabled {
                ("off", None, None)
            } else if !server.url.is_empty() {
                // A remote server is called from this PC: the computer's state does not matter, and the page does not call it
                // just to draw a row. Test asks it.
                ("unknown", None, Some("A tool server by link. Press Test to ask it.".to_string()))
            } else if !on {
                ("unknown", None, Some("Starts with the bot's computer.".to_string()))
            } else {
                match &tools {
                    Ok(by) => match by.get(&server.name) {
                        Some(guest) if guest.status == "ready" => ("ready", Some(guest.tools.len()), None),
                        Some(guest) if guest.status == "failed" => {
                            ("failed", None, Some(guest.error.clone().unwrap_or_else(|| "It did not start in the computer. Test it to see why.".to_string())))
                        }
                        Some(_) => ("unknown", None, Some("It is starting in the computer.".to_string())),
                        None => ("failed", None, Some("It is not running in the computer. Test it to see why.".to_string())),
                    },
                    Err(why) => ("unknown", None, Some(if why.is_empty() { "The computer did not answer.".to_string() } else { why.clone() })),
                }
            };
            let note = note.map(|text| scrub(&text, &read_env(bot, server)));
            ServerView {
                name: server.name.clone(),
                command: server.command.clone(),
                args: server.args.clone(),
                env_keys: server.env_keys.clone(),
                enabled: server.enabled,
                connector: server.connector,
                always_allow_reads: server.always_allow_reads,
                url: (!server.url.is_empty()).then(|| server.url.clone()),
                local: server.local,
                allow_tools: server.allow_tools.clone(),
                block_tools: server.block_tools.clone(),
                signed_in: server.oauth && super::mcp_oauth::load(bot, &server.name).is_some(),
                status,
                tool_count,
                note,
            }
        })
        .collect();
    let catalog = catalog()
        .iter()
        .map(|entry| CatalogView {
            id: entry.name.clone(),
            name: entry.name.clone(),
            title: entry.title.clone(),
            description: entry.description.clone(),
            command: entry.command_path(),
            args: entry.args.clone(),
            env_keys: entry.env.iter().map(|f| f.key.clone()).collect(),
            connector: entry.connector,
            fields: entry.env.clone(),
            maker: entry.maker.clone(),
            docs: entry.docs.clone(),
            installed: settings.mcp_servers.iter().any(|s| s.name == entry.name),
        })
        .collect();
    Ok(Overview {
        builtin: BuiltinView {
            name: BUILTIN,
            ready: on,
            reason: if on { None } else { Some("Starts with the bot's computer.".to_string()) },
        },
        catalog,
        servers,
    })
}

// ---- changes --------------------------------------------------------------------------------------

fn bad(code: &'static str, message: &str) -> StoreError {
    StoreError::new(code, message.to_string())
}

/// Checks a new server before anything is saved: name, command, arguments, env names and values, and room in the list.
pub fn validate_new(server: &NewServer, existing: &[McpServer]) -> StoreResult<()> {
    if !valid_server_name(&server.name) {
        return Err(bad("MCP_BAD_NAME", "A server name is 1 to 32 letters, digits, - or _ (no capitals or spaces)."));
    }
    if server.name == BUILTIN || existing.iter().any(|s| s.name == server.name) {
        return Err(bad("MCP_NAME_TAKEN", "A tool server with that name already exists."));
    }
    let command = server.command.trim();
    if command.is_empty() || command.chars().count() > MAX_COMMAND_CHARS || command.contains(['\n', '\r', '\0']) {
        return Err(bad("MCP_BAD_COMMAND", "Give the program to run (one line, for example npx)."));
    }
    if server.args.len() > MAX_ARGS || server.args.iter().any(|a| a.chars().count() > MAX_ARG_CHARS || a.contains(['\0', '\n', '\r'])) {
        return Err(bad("MCP_BAD_ARGS", "Each argument must be one short line, and there can be at most 32."));
    }
    for (key, value) in &server.env {
        if !valid_env_key(key) {
            return Err(bad("MCP_BAD_ENV", "Environment names are letters, digits and _, starting with a letter or _."));
        }
        if value.chars().count() > MAX_ENV_VALUE_CHARS || value.contains('\0') {
            return Err(bad("MCP_BAD_ENV", "An environment value is too long or has a character that is not allowed."));
        }
    }
    if existing.len() >= MAX_SERVERS {
        return Err(bad("MCP_FULL", "A bot can have at most 16 tool servers. Remove one first."));
    }
    Ok(())
}

/// After a start: a failure is logged (never the values) and does not fail the start.
pub fn push_at_start(root: &Path, sandbox: &dyn Sandbox) {
    push_quietly(root, sandbox);
}

fn push_quietly(root: &Path, sandbox: &dyn Sandbox) {
    if let Err(why) = push(root, sandbox) {
        tracing::warn!(error = %why, "tool servers not sent to the computer");
    }
}

/// Adds a custom server (enabled) and sends the list when the computer runs.
pub fn add_server(root: &Path, bot: &str, server: NewServer, sandbox: &dyn Sandbox) -> StoreResult<()> {
    add_entry(root, bot, server, false, sandbox)
}

fn add_entry(root: &Path, bot: &str, server: NewServer, connector: bool, sandbox: &dyn Sandbox) -> StoreResult<()> {
    let existing = bot_settings::load(root, bot)?.mcp_servers;
    validate_new(&server, &existing)?;
    let entry = McpServer {
        name: server.name.clone(),
        command: server.command.trim().to_string(),
        args: server.args.clone(),
        env_keys: server.env.keys().cloned().collect(),
        enabled: true,
        connector,
        always_allow_reads: false,
        url: String::new(),
        local: false,
        allow_tools: Vec::new(),
        block_tools: Vec::new(),
        oauth: false,
    };
    // The values go into the store first, so the settings never name a secret that is not there.
    if !server.env.is_empty() {
        store_env(bot, &server.name, &server.env)?;
    }
    let saved = bot_settings::update(root, bot, |settings| {
        if settings.mcp_servers.iter().any(|s| s.name == entry.name) || settings.mcp_servers.len() >= MAX_SERVERS {
            return Err(bad("MCP_NAME_TAKEN", "A tool server with that name already exists, or the list is full."));
        }
        settings.mcp_servers.push(entry);
        Ok(())
    });
    if let Err(error) = saved {
        drop_env(bot, &server.name);
        return Err(error);
    }
    push_quietly(root, sandbox);
    Ok(())
}

/// The brain's `request_connector` tool name: a card that says which connector the bot wants (no token is asked for).
pub const CONNECTOR_TOOL: &str = "request_connector";

/// The brain's `add_mcp_server` tool name.
pub const ADD_TOOL: &str = "add_mcp_server";

/// `add_mcp_server` from the brain: adds a tool server with no env values. A secret is never taken through the
/// brain (the schema has no env, and any `env` is refused); the owner adds a key in settings. The caller checks the
/// turn (the owner's own chat, not a routine) and the computer switches.
pub fn call_add(root: &Path, bot: &str, args: &Value, sandbox: &dyn Sandbox) -> StoreResult<Value> {
    if args.get("env").is_some_and(|v| !v.is_null()) {
        return Err(bad("MCP_NO_SECRETS", "This tool takes no secret values. Ask the owner to add the server's keys in the bot's settings, under tool servers."));
    }
    let name = args.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let command = args.get("command").and_then(Value::as_str).unwrap_or("").to_string();
    let argv: Vec<String> = match args.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string).ok_or_else(|| bad("MCP_BAD_ARGS", "Each argument must be text.")))
            .collect::<StoreResult<Vec<String>>>()?,
        Some(_) => return Err(bad("MCP_BAD_ARGS", "The arguments must be a list of text.")),
    };
    let server = NewServer { name: name.clone(), command, args: argv, env: BTreeMap::new() };
    add_server(root, bot, server, sandbox)?;
    Ok(json!({
        "added": name,
        "note": "It is in the bot's tool servers in settings now. If it needs a key or token, ask the owner to add it there; do not ask for it in chat.",
    }))
}

/// Adds a catalog entry (no env values) and sends the list when the computer runs.
pub fn add_catalog(root: &Path, bot: &str, id: &str, sandbox: &dyn Sandbox) -> StoreResult<()> {
    let entry = catalog().iter().find(|e| e.name == id).ok_or_else(|| bad("MCP_UNKNOWN_CATALOG", "There is no catalog entry with that name."))?;
    if entry.connector {
        return Err(bad("MCP_USE_CONNECTORS", "This one needs a token. Connect it under Connectors."));
    }
    add_server(
        root,
        bot,
        NewServer {
            name: entry.name.clone(),
            command: entry.command_path(),
            args: entry.args.clone(),
            env: BTreeMap::new(),
        },
        sandbox,
    )
}

/// Removes a server and its env values, then sends the list when the computer runs.
pub fn remove_server(root: &Path, bot: &str, name: &str, sandbox: &dyn Sandbox) -> StoreResult<()> {
    let mut found = false;
    bot_settings::update(root, bot, |settings| {
        let before = settings.mcp_servers.len();
        settings.mcp_servers.retain(|s| s.name != name);
        found = settings.mcp_servers.len() != before;
        Ok(())
    })?;
    if !found {
        return Err(bad("MCP_NOT_FOUND", "No tool server has that name."));
    }
    drop_env(bot, name);
    super::mcp_oauth::forget(bot, name);
    push_quietly(root, sandbox);
    Ok(())
}

/// Switches a server on or off, then sends the list when the computer runs.
pub fn set_enabled(root: &Path, bot: &str, name: &str, enabled: bool, sandbox: &dyn Sandbox) -> StoreResult<()> {
    if enabled && is_connector(root, bot, name) && !is_remote_server(root, bot, name) {
        check_not_in_use(root, bot, name)?;
    }
    let mut found = false;
    bot_settings::update(root, bot, |settings| {
        if let Some(server) = settings.mcp_servers.iter_mut().find(|s| s.name == name) {
            server.enabled = enabled;
            found = true;
        }
        Ok(())
    })?;
    if !found {
        return Err(bad("MCP_NOT_FOUND", "No tool server has that name."));
    }
    push_quietly(root, sandbox);
    Ok(())
}

/// Sends the list, then asks the computer which tools the server offers. Off servers and a stopped computer are errors
/// in the answer (never a start).
pub fn test_server(root: &Path, bot: &str, name: &str, sandbox: &dyn Sandbox) -> StoreResult<TestResult> {
    let settings = bot_settings::load(root, bot)?;
    let Some(server) = settings.mcp_servers.iter().find(|s| s.name == name) else {
        return Err(bad("MCP_NOT_FOUND", "No tool server has that name."));
    };
    if !server.enabled {
        return Ok(TestResult { ok: false, tools: vec![], error: Some("Turn the server on first.".to_string()) });
    }
    if !server.url.is_empty() {
        // Called from this PC; the computer need not run.
        let server = server.clone();
        return Ok(match list_remote(bot, &server, &remote_mcp::LiveWire::new(server.local)) {
            Ok(tools) if tools.is_empty() => TestResult { ok: false, tools: vec![], error: Some("The server answered but offers no tools.".to_string()) },
            Ok(tools) => TestResult { ok: true, tools: tools.into_iter().map(|t| t.name).collect(), error: None },
            Err(e) => TestResult { ok: false, tools: vec![], error: Some(remote_message(bot, &server, &e)) },
        });
    }
    if !running(sandbox) {
        return Ok(TestResult { ok: false, tools: vec![], error: Some("The bot's computer is not running. Start it, then test again.".to_string()) });
    }
    push(root, sandbox).map_err(|why| StoreError::new("MCP_CONFIG_FAILED", why))?;
    match sandbox.mcp_tools() {
        Ok(reply) => match tools_by_server(&reply).remove(name) {
            Some(guest) if guest.status == "ready" && !guest.tools.is_empty() => Ok(TestResult { ok: true, tools: guest.tools, error: None }),
            Some(guest) if guest.status == "ready" => {
                Ok(TestResult { ok: false, tools: vec![], error: Some("The server started but offers no tools.".to_string()) })
            }
            Some(guest) if guest.status == "failed" => Ok(TestResult {
                ok: false,
                tools: vec![],
                error: Some(guest.error.unwrap_or_else(|| "The server did not start in the computer.".to_string())),
            }),
            Some(_) => Ok(TestResult { ok: false, tools: vec![], error: Some("The server is still starting. Test again in a moment.".to_string()) }),
            None => Ok(TestResult { ok: false, tools: vec![], error: Some("The server did not start in the computer.".to_string()) }),
        },
        Err(e) => Ok(TestResult { ok: false, tools: vec![], error: Some(crate::redact::text(&e.message).into_inner()) }),
    }
}

// ---- connectors -----------------------------------------------------------------------------------

/// Replaces every non-trivial value in `text` with a mark. The values are the ones the server was started with: a tool
/// that prints its environment, or an error that quotes a header, cannot carry a token into a note or an answer.
pub fn scrub(text: &str, values: &BTreeMap<String, String>) -> String {
    use base64::Engine;
    let mut out = text.to_string();
    for value in values.values().map(|v| v.trim()).filter(|v| v.len() >= 4) {
        // Plain, base64 (both alphabets, with and without padding) and percent-encoded: a tool that echoes the value in
        // one of those shapes is caught too.
        let bytes = value.as_bytes();
        let forms = [
            value.to_string(),
            base64::engine::general_purpose::STANDARD.encode(bytes),
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes),
            base64::engine::general_purpose::URL_SAFE.encode(bytes),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
            percent_encode(value),
        ];
        for form in forms.iter().filter(|f| f.len() >= 4) {
            out = out.replace(form.as_str(), "[hidden]");
        }
    }
    out
}

pub fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Every secret value of one of the bot's servers: its env values (a token the person pasted) and the tokens of a sign-in.
fn secret_map(bot: &str, server: &McpServer) -> BTreeMap<String, String> {
    let mut values = read_env(bot, server);
    if server.oauth {
        for (i, value) in super::mcp_oauth::secret_values(bot, &server.name).into_iter().enumerate() {
            values.insert(format!("oauth{i}"), value);
        }
    }
    values
}

/// `scrub` for a whole tool answer (text and note), with the values of the bot's own server `server`.
pub fn scrub_answer(bot: &str, settings: &bot_settings::BotSettings, server: &str, answer: &mut Value) {
    let Some(entry) = settings.mcp_servers.iter().find(|s| s.name == server) else { return };
    let values = secret_map(bot, entry);
    if values.is_empty() {
        return;
    }
    for key in ["text", "note"] {
        if let Some(text) = answer.get(key).and_then(Value::as_str) {
            let clean = scrub(text, &values);
            answer[key] = Value::String(clean);
        }
    }
}

fn is_connector(root: &Path, bot: &str, name: &str) -> bool {
    bot_settings::load(root, bot).map(|s| s.mcp_servers.iter().any(|x| x.name == name && x.connector)).unwrap_or(false)
}

/// The server of this bot that a connector call is allowed to use: it is the bot's own, switched on, and a connector.
pub fn connector_of(settings: &bot_settings::BotSettings, server: &str) -> Option<McpServer> {
    settings.mcp_servers.iter().find(|s| s.name == server && s.connector).cloned()
}

/// The computer has one connector of each name and so one token for it. A second bot may not switch the same name on
/// with its own token: the guest would run only the first bot's server and the second bot would act with the first
/// bot's account.
fn check_not_in_use(root: &Path, bot: &str, name: &str) -> StoreResult<()> {
    for other in bots(root).into_iter().filter(|b| b != bot) {
        let settings = bot_settings::load(root, &other)?;
        if settings.mcp_servers.iter().any(|s| s.name == name && s.enabled) {
            return Err(bad("CONNECTOR_IN_USE", "Another bot on this computer already has this connector on. Turn it off there first, because a connector uses one account for the whole computer."));
        }
    }
    Ok(())
}

/// The person typed the values in the app. Names must be exactly the entry's, each value one non-empty line.
fn check_tokens(entry: &CatalogEntry, tokens: &BTreeMap<String, String>) -> StoreResult<()> {
    let mut wanted: Vec<&str> = entry.env.iter().map(|f| f.key.as_str()).collect();
    wanted.sort_unstable();
    let given: Vec<&str> = tokens.keys().map(String::as_str).collect();
    if wanted != given {
        return Err(bad("CONNECTOR_BAD_FIELDS", "Fill in every field of the connector."));
    }
    for value in tokens.values() {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_ENV_VALUE_CHARS || trimmed.chars().any(|c| c.is_control()) {
            return Err(bad("CONNECTOR_BAD_FIELDS", "A value is empty, too long, or has a character that is not allowed (paste it on one line)."));
        }
    }
    Ok(())
}

/// Connects a catalog connector for the bot: checks the values, installs the server on the running computer once, keeps
/// the values in the credential store, switches the server on and sends the list. The values come from the app's own field
/// and go nowhere else: not the settings file, not a log, not an answer, not the brain.
pub fn connect(root: &Path, bot: &str, id: &str, tokens: BTreeMap<String, String>, sandbox: &dyn Sandbox) -> StoreResult<()> {
    let entry = catalog().iter().find(|e| e.name == id && e.connector).ok_or_else(|| bad("MCP_UNKNOWN_CATALOG", "There is no connector with that name."))?;
    if !entry.install_fields_ok() {
        return Err(bad("CONNECTOR_BAD_CATALOG", "This connector's entry is not valid, so it was not installed."));
    }
    let tokens: BTreeMap<String, String> = tokens.into_iter().map(|(k, v)| (k, v.trim().to_string())).collect();
    check_tokens(entry, &tokens)?;
    if bot_settings::load(root, bot)?.mcp_servers.iter().any(|s| s.name == entry.name) {
        return Err(bad("CONNECTOR_EXISTS", "This connector is already connected. Disconnect it first to use a different token."));
    }
    check_not_in_use(root, bot, &entry.name)?;
    if !running(sandbox) {
        return Err(bad("CONNECTOR_NEEDS_COMPUTER", "The bot's computer is not running. Start it, then connect."));
    }
    if let Some(line) = entry.install_command() {
        let done = sandbox
            .root_shell(&line, std::time::Duration::from_secs(300))
            .map_err(|e| StoreError::new("CONNECTOR_INSTALL_FAILED", scrub(&e.message, &tokens)))?;
        if !done.success() {
            let why = if done.timed_out { "It took too long to download.".to_string() } else { last_line(&done.stderr) };
            return Err(StoreError::new("CONNECTOR_INSTALL_FAILED", format!("It could not be installed on the computer. {why}")));
        }
    }
    let server = NewServer { name: entry.name.clone(), command: entry.command_path(), args: entry.args.clone(), env: tokens };
    add_entry(root, bot, server, true, sandbox)
}

fn last_line(text: &str) -> String {
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    crate::redact::text(&line.chars().take(200).collect::<String>()).into_inner()
}

/// Per connector: whether read-shaped calls run without a card.
pub fn set_always_allow_reads(root: &Path, bot: &str, name: &str, on: bool) -> StoreResult<()> {
    let mut found = false;
    bot_settings::update(root, bot, |settings| {
        if let Some(server) = settings.mcp_servers.iter_mut().find(|s| s.name == name && s.connector) {
            server.always_allow_reads = on;
            found = true;
        }
        Ok(())
    })?;
    if found {
        Ok(())
    } else {
        Err(bad("MCP_NOT_FOUND", "No connector has that name."))
    }
}

/// The name a `request_connector` card shows: the catalog title when the service is one we list, else the words the bot
/// used, cleaned and cut. A service we do not list makes the card say so; the person then uses Add your own.
pub fn connector_label(service: &str) -> (String, bool) {
    let lower = service.trim().to_ascii_lowercase();
    if let Some(entry) = catalog().iter().find(|e| e.connector && (e.name == lower || e.title.to_ascii_lowercase() == lower)) {
        return (entry.title.clone(), true);
    }
    (service.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string(), false)
}

// ---- servers by link ------------------------------------------------------------------------------

/// A bot's server that is called by this app over the network (not run in the computer).
pub fn remote_of(settings: &bot_settings::BotSettings, server: &str) -> Option<McpServer> {
    settings.mcp_servers.iter().find(|s| s.name == server && !s.url.is_empty()).cloned()
}

fn is_remote_server(root: &Path, bot: &str, name: &str) -> bool {
    bot_settings::load(root, bot).map(|s| remote_of(&s, name).is_some()).unwrap_or(false)
}

/// The token the person gave for this server (`None`: none was given). Read from the credential store at the moment of a call.
fn bearer(bot: &str, server: &McpServer) -> Option<String> {
    read_env(bot, server).get("TOKEN").map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// A remote error as a plain sentence, with any form of the server's own token taken out.
pub fn remote_message(bot: &str, server: &McpServer, error: &RemoteError) -> String {
    scrub(&error.message, &secret_map(bot, server))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The token for a call: the sign-in's access token (renewed when it is about to end, or after a 401) when the server was
/// signed in to, else the token the person pasted.
fn token_source<'a>(bot: &'a str, server: &'a McpServer) -> impl Fn(bool) -> Result<Option<String>, RemoteError> + 'a {
    move |refresh| {
        if server.oauth {
            let wire = wire_for(server);
            if let Some(token) = super::mcp_oauth::access_token(wire.as_ref(), bot, server, now_ms(), refresh)? {
                return Ok(Some(token));
            }
        }
        Ok(bearer(bot, server))
    }
}

/// The tools a remote server offers, asked now.
pub fn list_remote(bot: &str, server: &McpServer, wire: &dyn remote_mcp::Wire) -> Result<Vec<remote_mcp::ToolInfo>, RemoteError> {
    let source = token_source(bot, server);
    remote_mcp::list_tools(wire, &server.url, server.local, &source)
}

/// The wire a server by link is spoken over. The tests' server is on 127.0.0.1 and so needs its Local tick, like a real one.
pub fn wire_for(server: &McpServer) -> Box<dyn remote_mcp::Wire> {
    Box::new(remote_mcp::LiveWire::new(server.local))
}

/// One call to a remote server: its `{content, isError}` result.
pub fn call_remote(bot: &str, server: &McpServer, wire: &dyn remote_mcp::Wire, tool: &str, arguments: &Value) -> Result<Value, RemoteError> {
    let source = token_source(bot, server);
    remote_mcp::call_tool(wire, &server.url, server.local, &source, tool, arguments)
}

/// Adds a remote tool server by link, enabled. The token (when the server wants one) is typed in the app, kept in the
/// credential store and used only by this app when it calls the server. It is never part of the link and never leaves
/// the PC for any other address.
pub fn add_remote(root: &Path, bot: &str, name: &str, url: &str, local: bool, token: Option<String>) -> StoreResult<()> {
    let name = name.trim();
    if !valid_server_name(name) || name == BUILTIN {
        return Err(bad("MCP_BAD_NAME", "A server name is 1 to 32 letters, digits, - or _ (lower case), and not \"computer\"."));
    }
    let url = remote_mcp::check_endpoint(url, local).map_err(|e| StoreError::new(e.code, e.message))?;
    let token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if let Some(t) = &token {
        if t.chars().count() > MAX_ENV_VALUE_CHARS || t.chars().any(|c| c.is_control()) {
            return Err(bad("MCP_BAD_ENV", "The token is too long or has a character that is not allowed (paste it on one line)."));
        }
    }
    let entry = McpServer {
        name: name.to_string(),
        command: String::new(),
        args: Vec::new(),
        env_keys: if token.is_some() { vec!["TOKEN".to_string()] } else { Vec::new() },
        enabled: true,
        connector: true,
        always_allow_reads: false,
        url,
        local,
        allow_tools: Vec::new(),
        block_tools: Vec::new(),
        oauth: false,
    };
    if bot_settings::load(root, bot)?.mcp_servers.iter().any(|s| s.name == name) {
        return Err(bad("MCP_NAME_TAKEN", "A tool server with that name already exists."));
    }
    if let Some(t) = &token {
        store_env(bot, name, &BTreeMap::from([("TOKEN".to_string(), t.clone())]))?;
    }
    let saved = bot_settings::update(root, bot, |settings| {
        if settings.mcp_servers.iter().any(|s| s.name == entry.name) || settings.mcp_servers.len() >= MAX_SERVERS {
            return Err(bad("MCP_NAME_TAKEN", "A tool server with that name already exists, or the list is full."));
        }
        settings.mcp_servers.push(entry);
        Ok(())
    });
    if let Err(error) = saved {
        drop_env(bot, name);
        return Err(error);
    }
    Ok(())
}

// ---- per-bot tool lists ---------------------------------------------------------------------------

/// Most names in one list.
pub const MAX_LISTED_TOOLS: usize = 200;

fn clean_tool_list(list: Vec<String>) -> StoreResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for name in list {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > MAX_TOOL_CHARS || name.chars().any(|c| c.is_control()) {
            return Err(bad("MCP_BAD_TOOL", "A tool name is empty, too long, or has a character that is not allowed."));
        }
        if !out.contains(&name) {
            out.push(name);
        }
    }
    if out.len() > MAX_LISTED_TOOLS {
        return Err(bad("MCP_BAD_TOOL", "That is too many tools in one list."));
    }
    Ok(out)
}

/// Sets which tools of one of the bot's servers it may call (`allow`: empty means all) and which it may never call (`block`).
/// Only the app command calls this; the host checks both lists on every call and hides what they refuse from the tools list.
pub fn set_tool_lists(root: &Path, bot: &str, name: &str, allow: Vec<String>, block: Vec<String>) -> StoreResult<()> {
    let allow = clean_tool_list(allow)?;
    let block = clean_tool_list(block)?;
    let mut found = false;
    bot_settings::update(root, bot, |settings| {
        if let Some(server) = settings.mcp_servers.iter_mut().find(|s| s.name == name) {
            server.allow_tools = allow.clone();
            server.block_tools = block.clone();
            found = true;
        }
        Ok(())
    })?;
    if found {
        Ok(())
    } else {
        Err(bad("MCP_NOT_FOUND", "No tool server has that name."))
    }
}

/// Takes the tools the bot may not call out of a tools list (`{servers:[{name, tools:[...]}]}`), for the servers in the bot's own
/// settings. A server the bot does not have is left as it is.
pub fn filter_listed_tools(settings: &bot_settings::BotSettings, reply: &mut Value) {
    let Some(servers) = reply.get_mut("servers").and_then(Value::as_array_mut) else { return };
    for server in servers {
        let Some(mine) = server.get("name").and_then(Value::as_str).and_then(|n| settings.mcp_servers.iter().find(|s| s.name == n)) else { continue };
        if mine.allow_tools.is_empty() && mine.block_tools.is_empty() {
            continue;
        }
        if let Some(tools) = server.get_mut("tools").and_then(Value::as_array_mut) {
            tools.retain(|t| t.get("name").and_then(Value::as_str).or_else(|| t.as_str()).is_some_and(|n| mine.tool_allowed(n)));
        }
    }
}

// ---- sign-in ----------------------------------------------------------------------------------------

/// Signs a tool server by link in, in the person's browser (`mcp_oauth.rs`). `open` shows an address in the browser. Blocks until
/// the person finishes or `limit` runs out. `client_id`: needed only for a server that cannot register apps by itself.
pub fn oauth_sign_in(
    root: &Path,
    bot: &str,
    name: &str,
    client_id: Option<&str>,
    wire: &dyn remote_mcp::Wire,
    limit: std::time::Duration,
    open: &dyn Fn(&str) -> Result<(), String>,
) -> StoreResult<()> {
    let settings = bot_settings::load(root, bot)?;
    let Some(server) = remote_of(&settings, name) else {
        return Err(bad("MCP_NOT_FOUND", "No tool server by link has that name."));
    };
    super::mcp_oauth::sign_in(wire, bot, &server, client_id, now_ms(), limit, open).map_err(|e| StoreError::new(e.code, remote_message(bot, &server, &e)))?;
    bot_settings::update(root, bot, |s| {
        if let Some(own) = s.mcp_servers.iter_mut().find(|x| x.name == name) {
            own.oauth = true;
        }
        Ok(())
    })?;
    Ok(())
}

/// Forgets the sign-in tokens of a server (the server stays in the list; a pasted token, if any, is left alone).
pub fn oauth_sign_out(root: &Path, bot: &str, name: &str) -> StoreResult<()> {
    let mut found = false;
    bot_settings::update(root, bot, |s| {
        if let Some(own) = s.mcp_servers.iter_mut().find(|x| x.name == name && !x.url.is_empty()) {
            own.oauth = false;
            found = true;
        }
        Ok(())
    })?;
    if !found {
        return Err(bad("MCP_NOT_FOUND", "No tool server by link has that name."));
    }
    super::mcp_oauth::forget(bot, name);
    Ok(())
}

// ---- add from chat ----------------------------------------------------------------------------------

/// Query names that carry a key. A link the bot suggests may not have one: the bot never takes a secret, and a secret in a link
/// would be shown in a card and kept in the settings file.
const SECRET_QUERY_WORDS: [&str; 10] = ["token", "key", "secret", "password", "passwd", "auth", "sig", "bearer", "credential", "session"];

/// A path piece that looks like a key (long, with letters and digits mixed): some services put the key in the link itself.
fn looks_like_key(piece: &str) -> bool {
    piece.len() >= 24
        && piece.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && piece.bytes().any(|b| b.is_ascii_digit())
        && piece.bytes().any(|b| b.is_ascii_alphabetic())
}

/// Checks a link the bot suggested: a public https link, no sign-in in it, nothing that looks like a key. Local is never on for a
/// suggestion; the owner ticks it in Settings. Returns the link as it will be shown and used.
pub fn check_suggested_link(raw: &str) -> Result<String, StoreError> {
    let url = remote_mcp::check_endpoint(raw, false).map_err(|e| StoreError::new(e.code, e.message))?;
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    let rest = &url[after..];
    let path_start = rest.find('/').unwrap_or(rest.len());
    let (path, query) = match rest[path_start..].split_once('?') {
        Some((p, q)) => (p, q),
        None => (&rest[path_start..], ""),
    };
    let secret_query = query.split('&').filter(|p| !p.is_empty()).any(|p| {
        let name = p.split('=').next().unwrap_or("").to_ascii_lowercase();
        SECRET_QUERY_WORDS.iter().any(|w| name.contains(w))
    });
    if secret_query || path.split('/').any(looks_like_key) {
        return Err(bad("MCP_SECRET_IN_LINK", "That link seems to hold a key or token. This tool takes no secret values: ask your owner to add the server in Settings > Connectors and paste the token there."));
    }
    Ok(url)
}

/// `add_mcp_server` with a link: checks the name and the link and returns what the card will ask. Adds nothing.
pub fn suggest_link(root: &Path, bot: &str, args: &Value) -> Result<(String, String), StoreError> {
    for key in args.as_object().into_iter().flat_map(|o| o.keys()) {
        let lower = key.to_ascii_lowercase();
        if matches!(lower.as_str(), "name" | "url" | "why") {
            continue;
        }
        return Err(bad("MCP_NO_SECRETS", "A tool server by link takes only a name and a link. If it needs a key or token, tell your owner to add it in Settings > Connectors; do not ask for it in chat."));
    }
    let name = args.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    if !valid_server_name(&name) || name == BUILTIN {
        return Err(bad("MCP_BAD_NAME", "Give the server a short name: 1 to 32 letters, digits, - or _ (lower case)."));
    }
    let url = check_suggested_link(args.get("url").and_then(Value::as_str).unwrap_or(""))?;
    let settings = bot_settings::load(root, bot)?;
    if settings.mcp_servers.iter().any(|s| s.name == name) || settings.mcp_servers.len() >= MAX_SERVERS {
        return Err(bad("MCP_NAME_TAKEN", "A tool server with that name already exists, or the list is full."));
    }
    Ok((name, url))
}
