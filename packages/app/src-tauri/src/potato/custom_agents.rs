//! The owner's own agent as a bot's brain ("Through my agents"). The owner pastes a `definition.toml` in Settings, sees a plain
//! summary and approves it. A turn then runs the agent headless INSIDE the sandbox, never on the PC.
//!
//! This file is the data side: checking a definition (with the line of any mistake), the summary the owner approves, the
//! environment and the launch script, and the prompt. `service/agent_flow.rs` runs a turn with it.
//!
//! The definition format is a small subset of the Puppet Master agent definition format, so files stay compatible:
//! `schema_version`, `[agent]` (`id`, `label`, `description`), `[binary]` (`names`), `[headless]` (`argv`, `task_separator`,
//! `task_flag`) and `[env]` (`set`, and `secret`: names whose values the owner types in Settings). Anything else is refused
//! with its line.
//!
//! Rules that hold here, and that the tests prove:
//! - a definition is data. The program is a bare name; no argument has a shell metacharacter, a path climb or a placeholder
//!   other than `{task}`. The launch line quotes every word, and the task is one quoted word;
//! - the agent starts with `env -i`: only HOME, PATH and LANG plus the definition's own `[env] set` names. Names that look like
//!   a secret, and every name of ours, are refused;
//! - the agent runs as its own user in the sandbox (not the bot's user, not root). The script refuses to start when that user
//!   can read the bot's home or the control secrets. The connector hub is a root process the host speaks to over a pipe; it
//!   has no port or file this user can reach (see `sandbox/secret_hub.rs`);
//! - a name under `[env] secret` gets the value the owner typed for that agent (kept in the OS credential store, never in the
//!   definition, the settings file, the thread or a log). It reaches only that agent's process, through the launch script's
//!   standard input (a here-document read by the private launcher, never an argument and never a file);
//! - the agent's files live in its own 1 GB ext4 image (`HOME_IMAGE_DIR`), loop-mounted as its home inside its private mount
//!   namespace each turn; no other agent's image is ever mounted. Removing the agent removes the image;
//! - nothing here reads a key, a token or a connector (the service hands over the agent's own secret values).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::sandbox::secrets::{CONTROL_FILE, SECRETS_DIR, TOKEN_FILE, VIEW_FILE};
use super::sandbox::shell_quote;

/// The user the agent runs as inside the sandbox, and its home.
/// Outside the connector hub's 60000-63999 range (`pm-sbx-secret-hub`), which the hub also refuses.
pub const AGENT_UID: u32 = 59999;
pub const AGENT_HOME: &str = "/var/lib/pm-agent";
/// Where each agent's home image lives in the sandbox (root only), and its size: the cap on what the agent can keep. The file
/// is sparse, so it takes disk only as the agent fills it; it takes no RAM.
pub const HOME_IMAGE_DIR: &str = "/var/lib/pm-agents";
const HOME_IMAGE_MB: &str = "1024";
/// Caps (megabytes of RAM at most) on each of the agent's three private temp folders (tmpfs, gone when the turn ends), and the
/// biggest single file (512-byte blocks: 256 MB).
const TMP_TMPFS_MB: &str = "100";
const FILE_LIMIT_BLOCKS: &str = "524288";
/// The credential-store name of one agent's secret: the bot, the agent and the environment name.
pub const AGENT_KEY_PREFIX: &str = "potato_agent_secret_";
pub const MAX_KEYS_PER_AGENT: usize = 8;
pub const MAX_KEY_CHARS: usize = 4096;
/// Where the agent's program is looked for: system folders only (the agent's user cannot read the bot's home).
pub const AGENT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
pub const MAX_DEFINITION_BYTES: usize = 8 * 1024;
pub const MAX_AGENTS_PER_BOT: usize = 8;
pub const MAX_TASK_CHARS: usize = 12_000;
pub const MAX_REPLY_CHARS: usize = 8_000;
/// One turn's time limit.
pub const TURN_SECONDS: u64 = 300;

pub const OWN_ACCOUNT_NOTE: &str = "This agent uses its own account. Its cost isn't counted here.";
pub const DECIDES_NOTE: &str = "This agent decides for itself inside the sandbox. Approval cards don't apply.";

/// An agent the owner approved: the definition text exactly as approved, and its fingerprint.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomAgent {
    pub id: String,
    pub label: String,
    pub toml: String,
    pub fingerprint: String,
    pub approved_at: i64,
}

/// A mistake in a definition: the line (1-based, when known) and plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefError {
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for DefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "Line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

fn bad(message: impl Into<String>) -> DefError {
    DefError { line: None, message: message.into() }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Def {
    schema_version: u32,
    agent: AgentT,
    binary: BinaryT,
    #[serde(default)]
    headless: HeadlessT,
    #[serde(default)]
    env: EnvT,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentT {
    id: String,
    label: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BinaryT {
    names: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct HeadlessT {
    #[serde(default)]
    argv: Vec<String>,
    task_separator: Option<String>,
    task_flag: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct EnvT {
    #[serde(default)]
    set: BTreeMap<String, String>,
    #[serde(default)]
    secret: Vec<String>,
}

/// A checked definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub id: String,
    pub label: String,
    pub description: String,
    pub program: String,
    pub argv: Vec<String>,
    pub task_separator: Option<String>,
    pub task_flag: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Names whose values the owner types in Settings (see `agent_secret_name`).
    pub secret_names: Vec<String>,
}

/// What the owner reads before approving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub label: String,
    pub description: String,
    pub command: String,
    pub installs: String,
    pub env_names: Vec<String>,
    /// The names it asks the owner for. Each value is typed in Settings after approval and kept in the OS credential store.
    pub secret_names: Vec<String>,
    pub fingerprint: String,
    pub notes: Vec<String>,
}

pub fn fingerprint(toml_text: &str) -> String {
    let digest = Sha256::digest(toml_text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes().iter().take(offset.min(text.len())).filter(|b| **b == b'\n').count() + 1
}

/// Finds the line of a value in the text, for a mistake the parser did not locate.
fn line_with(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

const ARG_FORBIDDEN: &[char] = &[';', '|', '&', '<', '>', '$', '`', '\\', '\n', '\r', '\0', '(', ')', '"', '\'', '*', '?', '~', '!', '#'];

fn check_arg(text: &str, what: &str, allow_task: bool, task_seen: &mut u32) -> Result<(), DefError> {
    let fail = |why: &str| {
        let mut e = bad(format!("{what} {why}"));
        e.line = line_with(text, "");
        e
    };
    if text.chars().count() > 200 {
        return Err(fail("is too long (200 characters at most)."));
    }
    if text.chars().any(|c| c.is_control() || ARG_FORBIDDEN.contains(&c)) {
        return Err(fail("has a character that a command line does not take here (a quote, ; | & < > $ ` \\ or similar)."));
    }
    if text.contains("..") {
        return Err(fail("climbs out of a folder with \"..\"."));
    }
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        let tail = &rest[at..];
        if let Some(inner) = tail.strip_prefix("{task}") {
            if !allow_task {
                return Err(fail("may not hold {task} here."));
            }
            *task_seen += 1;
            rest = inner;
        } else {
            return Err(fail("holds a placeholder. Only {task} is known here."));
        }
    }
    if text.contains('}') && !text.contains("{task}") {
        return Err(fail("has a stray brace."));
    }
    Ok(())
}

/// Names the sandbox sets itself, and names of ours (every prefix, and the exact names of the keys Potato Bot passes to its own
/// brain). Neither `[env] set` nor `[env] secret` may use them.
pub fn env_name_reserved(name: &str) -> Option<&'static str> {
    const FIXED: &[&str] = &[
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "IFS", "ENV", "BASH_ENV", "PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP",
        "NODE_OPTIONS", "NODE_PATH", "PERL5LIB", "PERL5OPT", "RUBYOPT", "RUBYLIB", "CLASSPATH", "JAVA_TOOL_OPTIONS", "GIT_ASKPASS",
        "SSH_ASKPASS", "SSH_AUTH_SOCK", "DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR",
    ];
    const OURS: &[&str] = &["POTATO_", "FISHD_", "PM_", "PUPPET_", "PM", "DEEPSEEK", "CONNECTOR", "MCP_", "SANDBOX", "TUNNEL"];
    const OUR_KEYS: &[&str] = &[super::env::KEY_NAME, super::env::GO_KEY_NAME, super::env::CUSTOM_KEY_NAME];
    if FIXED.contains(&name) || name.starts_with("LD_") || name.starts_with("DYLD_") {
        return Some("is a name the sandbox sets itself");
    }
    if OURS.iter().any(|p| name.starts_with(p)) || OUR_KEYS.contains(&name) {
        return Some("is a name Puppet Master uses for its own secrets and settings");
    }
    None
}

/// Names of ours and names that look like a secret never go into the agent's environment as plain `[env] set` values.
pub fn env_name_refused(name: &str) -> Option<&'static str> {
    if let Some(why) = env_name_reserved(name) {
        return Some(why);
    }
    const SECRETISH: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "PASS", "CREDENTIAL", "AUTH", "COOKIE", "SESSION", "BEARER", "PRIVATE", "CERT", "SIGN"];
    if name.split('_').any(|part| SECRETISH.contains(&part)) || SECRETISH.iter().any(|s| s.len() > 4 && name.contains(s)) {
        return Some("looks like a key, token or password. List it under [env] secret = [...] instead, and type its value in Settings");
    }
    None
}

fn valid_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && text.len() <= 64
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        && !text.contains("..")
}

/// Checks a definition. Errors carry the line.
pub fn parse(text: &str) -> Result<Spec, DefError> {
    if text.len() > MAX_DEFINITION_BYTES {
        return Err(bad("That definition is too long (8 KB at most)."));
    }
    if text.contains('\0') {
        return Err(bad("The definition has a stray character."));
    }
    let def: Def = toml::from_str(text).map_err(|e| {
        let line = e.span().map(|s| line_of(text, s.start));
        let raw = e.message().to_string();
        let message = if raw.contains("unknown field") { format!("This version does not take that setting. {raw}") } else { raw };
        DefError { line, message }
    })?;
    if def.schema_version != 1 {
        return Err(DefError { line: line_with(text, "schema_version"), message: "schema_version must be 1.".into() });
    }
    let id = def.agent.id.trim().to_string();
    let id_ok = id.len() >= 2
        && id.len() <= 32
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !id_ok {
        return Err(DefError { line: line_with(text, "id"), message: "The agent id is 2 to 32 lowercase letters, digits or _, starting with a letter.".into() });
    }
    let label = def.agent.label.trim().to_string();
    if label.is_empty() || label.chars().count() > 40 || label.chars().any(char::is_control) {
        return Err(DefError { line: line_with(text, "label"), message: "The label is 1 to 40 plain characters.".into() });
    }
    let description = def.agent.description.trim().to_string();
    if description.chars().count() > 200 || description.chars().any(|c| c.is_control()) {
        return Err(DefError { line: line_with(text, "description"), message: "The description is 200 plain characters at most.".into() });
    }
    if def.binary.names.is_empty() || def.binary.names.len() > 3 {
        return Err(DefError { line: line_with(text, "names"), message: "[binary] names lists one to three program names.".into() });
    }
    for name in &def.binary.names {
        if !valid_name(name) {
            return Err(DefError {
                line: line_with(text, name),
                message: format!("The program name must be a bare name (letters, digits, . _ + -), not a path or a command. Got: {}.", name.chars().take(40).collect::<String>()),
            });
        }
    }
    let mut task_seen = 0u32;
    for arg in &def.headless.argv {
        check_arg(arg, "A [headless] argv word", true, &mut task_seen).map_err(|mut e| {
            e.line = line_with(text, arg);
            e
        })?;
    }
    for (what, value) in [("task_separator", &def.headless.task_separator), ("task_flag", &def.headless.task_flag)] {
        if let Some(v) = value {
            check_arg(v, what, false, &mut task_seen).map_err(|mut e| {
                e.line = line_with(text, v);
                e
            })?;
            if v.is_empty() {
                return Err(DefError { line: line_with(text, what), message: format!("{what} cannot be empty.") });
            }
        }
    }
    if task_seen > 1 {
        return Err(DefError { line: line_with(text, "{task}"), message: "{task} may appear once.".into() });
    }
    if def.env.set.len() > 16 {
        return Err(bad("[env] set holds 16 names at most."));
    }
    for (name, value) in &def.env.set {
        let shape = !name.is_empty() && name.len() <= 64 && name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !shape {
            return Err(DefError { line: line_with(text, name), message: format!("The environment name {name} is capital letters, digits and _.") });
        }
        if let Some(why) = env_name_refused(name) {
            return Err(DefError { line: line_with(text, name), message: format!("The environment name {name} {why}.") });
        }
        if value.chars().count() > 200 || value.chars().any(char::is_control) {
            return Err(DefError { line: line_with(text, name), message: format!("The value of {name} is 200 plain characters at most.") });
        }
    }
    if def.env.secret.len() > MAX_KEYS_PER_AGENT {
        return Err(bad(format!("[env] secret holds {MAX_KEYS_PER_AGENT} names at most.")));
    }
    let mut seen: Vec<&str> = Vec::new();
    for name in &def.env.secret {
        let shape = !name.is_empty() && name.len() <= 64 && name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !shape {
            return Err(DefError { line: line_with(text, name), message: format!("The secret name {name} is capital letters, digits and _.") });
        }
        if let Some(why) = env_name_reserved(name) {
            return Err(DefError { line: line_with(text, name), message: format!("The secret name {name} {why}.") });
        }
        if def.env.set.contains_key(name) || seen.contains(&name.as_str()) {
            return Err(DefError { line: line_with(text, name), message: format!("The name {name} is listed twice.") });
        }
        seen.push(name);
    }
    Ok(Spec {
        id,
        label,
        description,
        program: def.binary.names[0].clone(),
        argv: def.headless.argv,
        task_separator: def.headless.task_separator,
        task_flag: def.headless.task_flag,
        env: def.env.set,
        secret_names: def.env.secret,
    })
}

/// The command line the owner reads (the task shows as `<your message>`).
pub fn command_text(spec: &Spec) -> String {
    let mut parts = vec![spec.program.clone()];
    parts.extend(spec.argv.iter().cloned());
    let has_inline = spec.argv.iter().any(|a| a.contains("{task}"));
    if !has_inline {
        if let Some(sep) = &spec.task_separator {
            parts.push(sep.clone());
        }
        if let Some(flag) = &spec.task_flag {
            parts.push(flag.clone());
        }
        parts.push("<your message>".to_string());
    }
    parts.join(" ").replace("{task}", "<your message>")
}

pub fn summary(text: &str) -> Result<Summary, DefError> {
    let spec = parse(text)?;
    let mut notes = vec![
        DECIDES_NOTE.to_string(),
        OWN_ACCOUNT_NOTE.to_string(),
        "It runs only in the sandbox, as its own user, never on this PC. It cannot see this bot's files or your connectors.".to_string(),
        "It keeps its own files, and any sign-in it makes, in its own 1 GB space in the sandbox. No other agent can see that space, and it is deleted when you remove the agent.".to_string(),
    ];
    if !spec.secret_names.is_empty() {
        notes.push(format!(
            "It asks you for: {}. You type each value in Settings after you approve. Each is kept in your computer's keychain and given only to this agent, never to the bot's model.",
            spec.secret_names.join(", ")
        ));
    }
    notes.push(format!("Each turn gets the bot's instructions, a short memory excerpt and the recent chat. Time limit: {} minutes.", TURN_SECONDS / 60));
    Ok(Summary {
        id: spec.id.clone(),
        label: spec.label.clone(),
        description: spec.description.clone(),
        command: command_text(&spec),
        installs: format!(
            "Nothing. {} must already be in the sandbox for every user (for example in /usr/local/bin). Puppet Master does not install it.",
            spec.program
        ),
        env_names: spec.env.keys().cloned().collect(),
        secret_names: spec.secret_names.clone(),
        fingerprint: fingerprint(text),
        notes,
    })
}

/// Approves a definition into a list: the text is checked again here, and the fingerprint is made here. One id once.
pub fn approve(list: &mut Vec<CustomAgent>, text: &str, now: i64) -> Result<CustomAgent, DefError> {
    let spec = parse(text)?;
    if list.iter().any(|a| a.id == spec.id) {
        return Err(bad(format!("There is already an agent called {}. Remove it first.", spec.id)));
    }
    if list.len() >= MAX_AGENTS_PER_BOT {
        return Err(bad("This bot has eight agents already. Remove one first."));
    }
    let agent = CustomAgent { id: spec.id, label: spec.label, toml: text.to_string(), fingerprint: fingerprint(text), approved_at: now };
    list.push(agent.clone());
    Ok(agent)
}

/// An agent the model or a stored file may run: it is in the list, it parses, and its text still matches the fingerprint
/// the owner approved. Anything else is refused.
pub fn approved_spec(list: &[CustomAgent], id: &str) -> Result<Spec, DefError> {
    let agent = list.iter().find(|a| a.id == id).ok_or_else(|| bad("This agent is not approved. Add it in Settings first."))?;
    if fingerprint(&agent.toml) != agent.fingerprint {
        return Err(bad("This agent's definition changed since you approved it. Add it again."));
    }
    parse(&agent.toml)
}

/// A part of a name that goes into a file name or a credential-store name: letters, digits, `_` and `-` only.
fn safe_part(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// The credential-store name of one secret of one agent of one bot. The webview may set it and read it masked, never read it back.
pub fn agent_secret_name(bot: &str, agent: &str, env_name: &str) -> String {
    format!("{AGENT_KEY_PREFIX}{}_{}_{}", safe_part(bot), safe_part(agent), safe_part(env_name))
}

/// True for a name of that shape (what the credential-store commands accept for the Settings field).
pub fn is_agent_secret_name(name: &str) -> bool {
    name.strip_prefix(AGENT_KEY_PREFIX).is_some_and(|rest| !rest.is_empty() && rest.len() <= 200 && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

/// The path of one agent's home image in the sandbox. One image per bot and agent: two bots never share a sign-in.
pub fn home_image(bot: &str, agent: &str) -> String {
    format!("{HOME_IMAGE_DIR}/{}--{}.img", safe_part(bot), safe_part(agent))
}

/// A value the owner typed that may be passed: one line, not empty, not huge.
pub fn secret_value_ok(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= MAX_KEY_CHARS && !value.chars().any(|c| c == '\n' || c == '\r' || c == '\0')
}

/// Takes the agent's own secret values out of a text that came back from it (a reply, an error). A secret the agent prints is
/// not shown, saved or logged.
pub fn hide_secrets(text: &str, secrets: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (_, value) in secrets {
        if value.chars().count() >= 4 {
            out = out.replace(value.as_str(), "[hidden]");
        }
    }
    out
}

/// Every variable the agent starts with, and nothing else: the sandbox's three plus the definition's own `[env] set`.
pub fn env_pairs(spec: &Spec) -> Vec<(String, String)> {
    let mut env = vec![("HOME".to_string(), AGENT_HOME.to_string()), ("PATH".to_string(), AGENT_PATH.to_string()), ("LANG".to_string(), "C.UTF-8".to_string())];
    for (name, value) in &spec.env {
        env.push((name.clone(), value.clone()));
    }
    env
}

/// Shell helpers shared by the launch script and the clean-up script: the processes whose real, effective or saved user is
/// the agent's (a zombie does not count), and a loop that stops then kills them until none is left. It reads `/proc`, so a child
/// that called `setsid` or `setpgid` (which `timeout -k` cannot reach) is found by its user, not by its group.
const KILL_FUNCS: &str = r##"agent_pids() { for d in /proc/[0-9]*; do awk -v U=@UID@ '$1=="State:"{z=($2=="Z")} $1=="Uid:"{m=($2==U||$3==U||$4==U)} END{exit !(m&&!z)}' "$d/status" 2>/dev/null && echo "${d#/proc/}"; done; }
clear_agent() { n=0; while [ "$n" -lt 20 ]; do p=$(agent_pids); [ -z "$p" ] && return 0; kill -STOP $p 2>/dev/null; kill -KILL $p 2>/dev/null; n=$((n+1)); sleep 0.2; done; [ -z "$(agent_pids)" ]; }
"##;

/// The guards, run as root before the agent starts. Each one fails closed: the turn does not start and the exit code says which
/// kind of guard it was (70 home, 71 files and screen, 72 network and Windows, 73 leftover processes, 74 private view).
///
/// The agent then starts in its own mount namespace (`unshare -m`, the script `/run/pm-agent-ns.sh` written above): its own
/// small tmpfs over `/run/WSL` (the Windows interop socket), `/run/user` (the bot's session bus and sound sockets), `/tmp`,
/// `/var/tmp` and `/dev/shm` (capped tmpfs, gone when the turn ends) and, over its home, its own ext4 image (the cap is the image
/// size, and it is kept between turns, with the agent's sign-in). Nothing of the bot's is seen there. Past a cap, a write fails
/// with "No space left on device"; one file can be at most `ulimit -f` big. The bot and every other process keep their own view.
///
/// The agent's secrets are the lines of a here-document at the end of the launch command: the private launcher reads them from
/// its standard input into its environment, so they are never an argument and never a file.
const GUARDS: &str = r##"fail() { echo "$2" >&2; exit "$1"; }
resolvers() { cat /etc/resolv.conf /run/systemd/resolve/resolv.conf 2>/dev/null | awk '$1=="nameserver" && NF>=2 {print $2}' | sort -u | grep -E "$1"; }
fw_ipt() {
  c=$1
  if [ "$2" = 6 ]; then re='^[0-9A-Fa-f:.]*:[0-9A-Fa-f:.]*$'; else re='^[0-9]+(\.[0-9]+){3}$'; fi
  "$c" -N PM_SBX_AGENT 2>/dev/null
  "$c" -F PM_SBX_AGENT || return 1
  for ns in $(resolvers "$re"); do
    "$c" -A PM_SBX_AGENT -d "$ns" -p udp --dport 53 -m owner --uid-owner @UID@ -j RETURN || return 1
    "$c" -A PM_SBX_AGENT -d "$ns" -p tcp --dport 53 -m owner --uid-owner @UID@ -j RETURN || return 1
  done
  "$c" -A PM_SBX_AGENT -o lo -m owner --uid-owner @UID@ -j REJECT || return 1
  while "$c" -D OUTPUT -j PM_SBX_AGENT 2>/dev/null; do :; done
  "$c" -I OUTPUT 1 -j PM_SBX_AGENT || return 1
  [ "$("$c" -S OUTPUT | sed -n 2p)" = '-A OUTPUT -j PM_SBX_AGENT' ] || return 1
  [ "$("$c" -S PM_SBX_AGENT | tail -n 1 | grep -c -- '-o lo .*--uid-owner @UID@ .*-j REJECT')" = 1 ]
}
fw_nft() {
  {
    echo "add table inet pm_sbx_agent"
    echo "delete table inet pm_sbx_agent"
    echo "table inet pm_sbx_agent {"
    echo "  chain output {"
    echo "    type filter hook output priority -10; policy accept;"
    for ns in $(resolvers '^[0-9]+(\.[0-9]+){3}$'); do
      echo "    meta skuid @UID@ ip daddr $ns udp dport 53 accept"
      echo "    meta skuid @UID@ ip daddr $ns tcp dport 53 accept"
    done
    for ns in $(resolvers '^[0-9A-Fa-f:.]*:[0-9A-Fa-f:.]*$'); do
      echo "    meta skuid @UID@ ip6 daddr $ns udp dport 53 accept"
      echo "    meta skuid @UID@ ip6 daddr $ns tcp dport 53 accept"
    done
    echo "    meta skuid @UID@ oifname \"lo\" reject"
    echo "  }"
    echo "}"
  } | "$1" -f - || return 1
  "$1" list chain inet pm_sbx_agent output | grep 'skuid @UID@' | grep -q 'oifname "lo" reject'
}
guards() {
  mkdir -p @HOME@ && chown @UID@:@UID@ @HOME@ && chmod 0700 @HOME@ || fail 70 'The sandbox could not make the agent a home.'
  clear_agent || fail 73 'The sandbox could not stop an earlier agent run. Nothing was started.'
  for t in truncate mkfs.ext4 e2fsck mount; do command -v "$t" >/dev/null || fail 74 "The sandbox has no $t, so it cannot keep a private space for this agent. Nothing was started."; done
  mkdir -p -m 0700 @IMGDIR@ && chmod 0700 @IMGDIR@ || fail 74 'The sandbox could not make the folder for the agent spaces. Nothing was started.'
  if [ ! -f @IMG@ ]; then
    rm -f @IMG@.new
    { truncate -s @HOMEMB@M @IMG@.new && mkfs.ext4 -q -F -m 0 -L pmagent -E root_owner=@UID@:@UID@ @IMG@.new >/dev/null 2>&1 && chmod 0600 @IMG@.new && mv @IMG@.new @IMG@; } || { rm -f @IMG@.new; fail 74 'The sandbox could not make a private space for this agent. Nothing was started.'; }
  fi
  e2fsck -p @IMG@ >/dev/null 2>&1
  [ "$?" -le 1 ] || fail 74 'The private space of this agent is damaged and could not be repaired. Nothing was started.'
  chmod 0600 @IMG@
  PMH=$(getent passwd pm | cut -d: -f6)
  PMU=$(id -u pm 2>/dev/null)
  [ -n "$PMH" ] && [ -d "$PMH" ] && [ -n "$PMU" ] || fail 71 'The sandbox has no bot user to check. Nothing was started.'
  chmod o-rwx "$PMH" || fail 71 'The sandbox could not close the bot home. Nothing was started.'
  for d in "$PMH/.pm-sbx" "$PMH/.browser-profiles" "$PMH/.config"; do [ -d "$d" ] && chmod go-rwx "$d" 2>/dev/null; done
  [ -d @SECRETS@ ] && chown root:"$(id -g pm)" @SECRETS@ 2>/dev/null && chmod 0750 @SECRETS@ 2>/dev/null
  for p in "$PMH" @SECRETS@ @TOKEN@ @VIEW@ @CONTROL@; do
    if [ -n "$p" ] && [ -e "$p" ] && @DROP@ sh -c 'test -r "$1" || test -x "$1" || test -w "$1"' sh "$p"; then fail 71 'The sandbox is not isolated enough for this agent (it could read the bot files). Nothing was started.'; fi
  done
  XD=$(command -v xdpyinfo)
  if [ -n "$XD" ]; then
    if @DROP@ env -i HOME=@HOME@ DISPLAY=:99 "$XD" >/dev/null 2>&1; then fail 71 'The sandbox screen is open to other users. Restart the sandbox, then try again. Nothing was started.'; fi
  elif [ -e /tmp/.X11-unix/X99 ]; then
    fail 71 'The sandbox cannot check that its screen is closed to this agent. Nothing was started.'
  fi
  grep -qx 'state=applied' /run/pm-sbx/egress.status 2>/dev/null || fail 72 'The sandbox network wall is not in place. Nothing was started.'
  if awk '$3=="drvfs"||$3=="9p"{f=1} END{exit !f}' /proc/mounts; then fail 72 'The sandbox can see the Windows PC. Nothing was started.'; fi
  command -v unshare >/dev/null || fail 72 'The sandbox cannot give this agent a private view of its files. Nothing was started.'
  mount -o remount,hidepid=2 /proc 2>/dev/null
  FW=0
  if IPT=$(command -v iptables) && "$IPT" -S OUTPUT >/dev/null 2>&1; then
    fw_ipt "$IPT" 4 || fail 72 'The sandbox could not close its local network to this agent. Nothing was started.'
    if IP6=$(command -v ip6tables) && "$IP6" -S OUTPUT >/dev/null 2>&1; then
      fw_ipt "$IP6" 6 || fail 72 'The sandbox could not close its local network (IPv6) to this agent. Nothing was started.'
    elif [ -s /proc/net/if_inet6 ]; then
      fail 72 'The sandbox has IPv6 but no way to close it to this agent. Nothing was started.'
    fi
    FW=1
  elif NFT=$(command -v nft) && "$NFT" list ruleset >/dev/null 2>&1; then
    fw_nft "$NFT" || fail 72 'The sandbox could not close its local network to this agent. Nothing was started.'
    FW=1
  fi
  [ "$FW" = 1 ] || fail 72 'The sandbox has no firewall tools to close its local network to this agent. Nothing was started.'
  (umask 077; cat >/run/pm-agent-ns.sh) <<'PMNS' || fail 70 'The sandbox could not prepare a private view for this agent. Nothing was started.'
set -u
bad() { echo "$2" >&2; exit "$1"; }
ulimit -f @FSIZE@ || bad 74 'The sandbox could not limit file sizes for this agent. Nothing was started.'
mnt() { mount -t tmpfs -o "$2" tmpfs "$1" || bad 74 "The sandbox could not make a private $1 for this agent. Nothing was started."; }
for d in /run/WSL /run/user; do [ -d "$d" ] && mnt "$d" size=64k,mode=0555; done
[ -d /dev/shm ] && mnt /dev/shm size=@TMPMB@m,mode=1777
mnt /tmp size=@TMPMB@m,mode=1777
mnt /var/tmp size=@TMPMB@m,mode=1777
mount -o loop,nosuid,nodev @IMG@ @HOME@ || bad 74 "The sandbox could not open the private space of this agent (loop mounts may not work here). Nothing was started."
chown @UID@:@UID@ @HOME@ && chmod 0700 @HOME@ || bad 74 "The sandbox could not close the private space of this agent. Nothing was started."
for d in /run/WSL /run/user /tmp /var/tmp; do [ -z "$(ls -A "$d" 2>/dev/null)" ] || bad 72 "The sandbox could not hide $d from this agent. Nothing was started."; done
for s in $(find /run /dev/shm -type s -user pm 2>/dev/null); do
  if @DROP@ test -w "$s"; then bad 71 "The agent could reach a bot socket ($s). Nothing was started."; fi
done
while IFS= read -r line; do [ -n "$line" ] && export "$line"; done
cd @HOME@ || bad 70 "The sandbox could not open the home of this agent. Nothing was started."
exec @DROP@ "$@" </dev/null
PMNS
}
"##;

/// Replaces every `@NAME@` in one pass, so a value that holds such a word (the task, an env value) is never read again.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 512);
    let mut rest = template;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        match tail.find('@').and_then(|end| values.iter().find(|(name, _)| *name == &tail[..end]).map(|(_, v)| (end, *v))) {
            Some((end, value)) => {
                out.push_str(value);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('@');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The command that drops to the agent's user: no groups, no new privileges (so a setuid program or sudo gains nothing).
fn drop_privs() -> String {
    format!("setpriv --reuid={AGENT_UID} --regid={AGENT_UID} --clear-groups --no-new-privs")
}

/// Kills every process of the agent's user, as root, and checks none is left (exit 73 when one is). The host runs it after every
/// turn however it ended: done, timed out or stopped.
pub fn kill_script() -> String {
    let uid = AGENT_UID.to_string();
    let mut text = String::from("set -u\n");
    text.push_str(&fill(KILL_FUNCS, &[("UID", &uid)]));
    text.push_str("clear_agent </dev/null || { echo 'AGENT_LEFTOVER' >&2; exit 73; }\n");
    text
}

/// The script that runs one turn (as root, over the shell's standard input). It makes the agent's home, runs the guards (the
/// bot's files and secrets, shared temp files, the screen, the network wall, the Windows PC, the local network) and refuses to go
/// on when one is missing, finds the program in the system folders, and starts it as the agent's user with an empty environment
/// but the allow-list, a time limit, no input and no new privileges. When it ends, however it ended, every process of that user
/// is killed and checked.
pub fn launch_script(spec: &Spec, task: &str, seconds: u64, bot: &str, secrets: &[(String, String)]) -> String {
    let uid = AGENT_UID.to_string();
    let mut words: Vec<String> = Vec::new();
    for arg in &spec.argv {
        words.push(shell_quote(&arg.replace("{task}", task)));
    }
    if !spec.argv.iter().any(|a| a.contains("{task}")) {
        if let Some(sep) = &spec.task_separator {
            words.push(shell_quote(sep));
        }
        if let Some(flag) = &spec.task_flag {
            words.push(shell_quote(flag));
        }
        words.push(shell_quote(task));
    }
    let env: Vec<String> = env_pairs(spec).into_iter().map(|(name, value)| if value.contains(|c: char| !(c.is_ascii_alphanumeric() || "/:._-".contains(c))) { format!("{name}={}", shell_quote(&value)) } else { format!("{name}={value}") }).collect();
    let drop = drop_privs();
    let image = home_image(bot, &spec.id);
    let (prog, env, words, seconds) = (shell_quote(&spec.program), env.join(" "), words.join(" "), seconds.to_string());
    let fixed = [
        ("UID", uid.as_str()),
        ("HOME", AGENT_HOME),
        ("SECRETS", SECRETS_DIR),
        ("TOKEN", TOKEN_FILE),
        ("VIEW", VIEW_FILE),
        ("CONTROL", CONTROL_FILE),
        ("DROP", drop.as_str()),
        ("HOMEMB", HOME_IMAGE_MB),
        ("TMPMB", TMP_TMPFS_MB),
        ("FSIZE", FILE_LIMIT_BLOCKS),
        ("IMGDIR", HOME_IMAGE_DIR),
        ("IMG", image.as_str()),
    ];
    let mut script = String::from("set -u\n");
    script.push_str(&fill(KILL_FUNCS, &fixed));
    script.push_str(&fill(GUARDS, &fixed));
    // The secrets: `NAME=value` lines of a here-document whose end marker is a line that is none of them.
    let lines: Vec<String> = secrets.iter().filter(|(name, value)| spec.secret_names.contains(name) && secret_value_ok(value)).map(|(name, value)| format!("{name}={value}")).collect();
    let mut marker = String::from("PMSEC");
    while lines.iter().any(|l| *l == marker) {
        marker.push('_');
    }
    let input = if lines.is_empty() { "</dev/null".to_string() } else { format!("<<'{marker}'\n{}\n{marker}", lines.join("\n")) };
    // The parts that may hold any text come last, in one pass.
    let tail = "guards </dev/null\n\
                PROG=$(PATH=@PATH@ command -v @PROG@) || fail 127 'AGENT_NOT_INSTALLED'\n\
                timeout -k 5 @SECONDS@ unshare -m --propagation private env -i @ENV@ sh /run/pm-agent-ns.sh \"$PROG\" @WORDS@ @INPUT@\n\
                rc=$?\n\
                clear_agent </dev/null || fail 73 'The sandbox could not stop everything the agent started.'\n\
                exit \"$rc\"\n";
    script.push_str(&fill(
        tail,
        &[("PATH", AGENT_PATH), ("PROG", &prog), ("HOME", AGENT_HOME), ("SECONDS", &seconds), ("DROP", &drop), ("ENV", &env), ("WORDS", &words), ("INPUT", &input)],
    ));
    script
}

/// What a finished run means for the owner, in plain words. `Ok(text)` is the agent's answer.
pub fn reading(out: &super::sandbox::ShellOutput, label: &str) -> Result<String, String> {
    if out.timed_out || out.code == Some(124) || out.code == Some(137) {
        return Err(format!("{label} took too long (over {} minutes) and was stopped.", TURN_SECONDS / 60));
    }
    match out.code {
        Some(0) => {
            let text = out.stdout.trim();
            if text.is_empty() {
                return Err(format!("{label} finished without an answer."));
            }
            Ok(text.chars().take(MAX_REPLY_CHARS).collect())
        }
        Some(127) if out.stderr.contains("AGENT_NOT_INSTALLED") => Err(format!("{label}'s program is not installed in the sandbox for every user. Install it there (for example in /usr/local/bin), then try again.")),
        Some(70..=73) => Err(out.stderr.trim().chars().take(200).collect()),
        code => {
            let tail: String = out.stderr.trim().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
            let code = code.map_or("none".to_string(), |c| c.to_string());
            Err(if tail.is_empty() { format!("{label} stopped with an error (code {code}).") } else { format!("{label} stopped with an error (code {code}): {tail}") })
        }
    }
}

/// The prompt for one turn. Persona and memory are the owner's own text; the recent chat is plain lines. The caller has
/// already taken keys out of every part; `clean` takes them out again, and the whole is cut to the task limit.
pub fn build_prompt(name: &str, persona: &str, memory: &str, history: &[(String, String)], message: &str) -> String {
    let mut out = format!("You are {name}, chatting with your owner. Answer their newest message in plain text.\n");
    if !persona.trim().is_empty() {
        out.push_str(&format!("\nYour instructions from your owner:\n{}\n", persona.trim()));
    }
    if !memory.trim().is_empty() {
        out.push_str(&format!("\nNotes you kept (may be old):\n{}\n", memory.trim()));
    }
    if !history.is_empty() {
        out.push_str("\nThe recent chat:\n");
        for (who, text) in history {
            out.push_str(&format!("{who}: {}\n", text.trim()));
        }
    }
    out.push_str(&format!("\nTheir newest message:\n{}\n", message.trim()));
    let cut: String = out.chars().filter(|c| *c != '\0').collect();
    if cut.chars().count() > MAX_TASK_CHARS {
        // Keep the newest message whole: cut the middle of the older parts.
        let tail = format!("\nTheir newest message:\n{}\n", message.trim());
        let room = MAX_TASK_CHARS.saturating_sub(tail.chars().count());
        let mut head: String = cut.chars().take(room).collect();
        head.push_str("\n[older text cut]");
        return format!("{head}{tail}").chars().take(MAX_TASK_CHARS + 20).collect();
    }
    cut
}
