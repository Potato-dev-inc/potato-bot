//! The hard risk gate's eye (Professional mode, P5): reads the text of a shell command and says which entries of the
//! risky list it touches. The list, from the plan:
//!
//! - leaving the computer: `git push`, `npm publish`, `ssh`, `scp`, `gh pr merge`, `gh release`, deploy tools;
//! - destroying: `rm -rf` outside the work folder, `git push --force`, `git reset --hard`, dropping a database.
//!
//! "Anything using a real account through a connected tool" is not here: a connected tool is not a shell command.
//!
//! It sees through the simple ways to wrap a command: `bash -c "..."`, `sh -c`, `eval`, `&&`, `||`, `;`, `|`, newlines,
//! `$(...)`, backticks, `<(...)`, env prefixes (`FOO=1 git push`, `env`, `sudo`, `nohup`, `timeout`, `xargs`, ...), full
//! paths (`/usr/bin/git`), quoted words (`'git' "push"`), `find -exec`, and `cd dir && rm -rf x`. A command is only ever
//! read, never run, and unknown commands are not risky.
//!
//! This is a net, not a wall. A script file, `python -c`, a string built at run time or an alias can hide a command from
//! it. The bot's computer stays the real boundary; the gate only stops the plain, honest ways to do the risky things.
//! The decision lives in `sandbox_calls.rs`, in the host. Nothing the model writes can turn it off.

/// The folder a bot may delete inside without asking: the sandbox user's home, below its first level, and not a hidden
/// folder (`.ssh`, `.config`, `.pm-sbx` ...).
pub const WORK_ROOT: &str = "/home/pm";
/// Where a command runs when the call names no folder.
pub const DEFAULT_CWD: &str = "/home/pm";
const MAX_DEPTH: u32 = 6;

/// How much the bot asks for. The owner's setting for the bot (`bot_settings.rs`, field `level`); written only by the
/// app command, never by a tool the model can call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Only read-only commands run without a card; anything unclassified asks, and so do file writes and sends.
    AskFirst,
    /// The risky list asks. The default, and what a bot that never had the setting reads as.
    #[default]
    AskWhenRisky,
    /// The risk gate is off inside the bot's computer. Questions and logins still make cards.
    Full,
}

/// The bot's level. A missing setting reads as the default. A settings file that cannot be read reads as the strictest
/// level, so a damaged file never loosens the gate.
pub fn level_for(root: &std::path::Path, bot: &str) -> Level {
    match super::bot_settings::load(root, bot) {
        Ok(settings) => settings.level,
        Err(_) => Level::AskFirst,
    }
}

/// One entry of the risky list a command touches. `class` is the name a "for this job" approval is kept under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Risk {
    pub class: &'static str,
    pub label: &'static str,
}

const fn risk(class: &'static str, label: &'static str) -> Risk {
    Risk { class, label }
}

pub const GIT_PUSH: Risk = risk("git push", "pushes to a remote");
pub const GIT_FORCE: Risk = risk("git push --force", "force-pushes to a remote");
pub const GIT_RESET: Risk = risk("git reset --hard", "throws away uncommitted work");
pub const PUBLISH: Risk = risk("publish", "publishes a package");
pub const REMOTE: Risk = risk("remote shell", "reaches another computer (ssh, scp, sftp, rsync)");
pub const GH_MERGE: Risk = risk("gh pr merge", "merges a pull request");
pub const GH_RELEASE: Risk = risk("gh release", "makes or changes a release");
pub const GH_DELETE: Risk = risk("gh repo delete", "deletes a repository");
pub const DEPLOY: Risk = risk("deploy", "deploys to the outside world");
pub const RM: Risk = risk("rm -rf", "deletes files outside the work folder");
pub const DROP_DB: Risk = risk("drop database", "drops or empties a database");
pub const PIPED_SHELL: Risk = risk("run piped script", "runs text it was piped as a script");
pub const CHANGES: Risk = risk("any change", "changes something in its computer");
pub const TOO_NESTED: Risk = risk("nested command", "hides a command inside many layers");

/// Every class the gate can name, for a plain description of a saved rule.
const ALL_RISKS: &[Risk] = &[GIT_PUSH, GIT_FORCE, GIT_RESET, PUBLISH, REMOTE, GH_MERGE, GH_RELEASE, GH_DELETE, DEPLOY, RM, DROP_DB, PIPED_SHELL, CHANGES, TOO_NESTED];

/// The classes an owner may say "Always allow" for. A short list on purpose: things that leave the computer but can be
/// undone or are the point of the job. Never the destroying ones (`rm -rf`, force-push, `git reset --hard`, deleting a
/// repository, dropping a database), and never the open-ended ones (a hidden or piped script, a nested command, "any
/// change", a connected tool): those are unreadable or too wide to promise ahead of time. Also checked when a rule is
/// used, so a hand-edited settings file cannot widen it.
pub const ALWAYS_ALLOWABLE: &[&str] = &["git push", "publish", "gh pr merge", "gh release", "deploy", "send file"];

pub fn always_allowable(class: &str) -> bool {
    ALWAYS_ALLOWABLE.contains(&class)
}

/// A class in plain words for the Advanced tab ("pushes to a remote"). A class the gate does not know reads as itself.
pub fn class_description(class: &str) -> String {
    if class == "send file" {
        return "puts a file from its computer into the chat".to_string();
    }
    ALL_RISKS.iter().find(|r| r.class == class).map_or_else(|| class.to_string(), |r| r.label.to_string())
}

/// Every entry of the risky list the command touches, each once, in the order they appear. Empty: not risky.
/// `cwd` is the folder the command runs in (a guest path); `None` means the default.
pub fn classify(command: &str, cwd: Option<&str>) -> Vec<Risk> {
    let mut walk = Walk { risks: Vec::new(), db_client: false };
    walk.scan(command, 0, Some(cwd.unwrap_or(DEFAULT_CWD).to_string()));
    if walk.db_client && drops_data(command) {
        walk.add(DROP_DB);
    }
    walk.risks
}

const READ_ONLY_PROGRAMS: &[&str] = &[
    "ls", "cat", "pwd", "echo", "printf", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "which", "whoami", "date", "uname", "id", "hostname", "stat",
    "file", "du", "df", "tree", "sort", "diff", "cmp", "basename", "dirname", "realpath", "readlink", "true", "false", "test", "[", "env", "find", "git",
];
const READ_ONLY_GIT: &[&str] = &["status", "log", "diff", "show", "rev-parse", "ls-files", "blame", "describe", "shortlog"];

/// Whether a shell command only looks: every command in it is on a short list of programs that cannot change anything,
/// and it writes nowhere (no `>`, no `$(...)`, no backticks). Anything else, or anything it cannot read, is not
/// read-only. Used by the Ask first level: a command that is not read-only asks.
pub fn read_only(command: &str) -> bool {
    if command.contains('>') || command.contains('`') || command.contains("$(") || command.contains("<(") {
        return false;
    }
    let mut any = false;
    for (segment, _) in split_segments(command) {
        let words = tokenize(&segment);
        let Some(first) = words.first() else { continue };
        any = true;
        if first.contains('/') || first.contains('\\') || !READ_ONLY_PROGRAMS.contains(&first.as_str()) {
            return false;
        }
        let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        let has = |flag: &dyn Fn(&str) -> bool| args.iter().any(|a| flag(a));
        let ok = match first.as_str() {
            "env" => args.is_empty(),
            "find" => !has(&|a| matches!(a, "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete" | "-fls") || a.starts_with("-fprint")),
            "git" => args.first().is_some_and(|sub| READ_ONLY_GIT.contains(sub)) && !has(&|a| a.starts_with("--output") || a == "--ext-diff"),
            "sort" | "tree" => !has(&|a| a == "-o" || a.starts_with("--output") || (a.starts_with("-o") && !a.starts_with("--"))),
            "date" => !has(&|a| a == "-s" || a.starts_with("--set")),
            "rg" => !has(&|a| a.starts_with("--pre")),
            "file" => !has(&|a| a == "-C" || a.starts_with("--compile")),
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    any
}

struct Walk {
    risks: Vec<Risk>,
    /// A database client program was seen: the whole text is then read for a DROP.
    db_client: bool,
}

impl Walk {
    fn add(&mut self, risk: Risk) {
        if !self.risks.contains(&risk) {
            self.risks.push(risk);
        }
    }

    /// Reads a piece of shell text: first what is inside `$(...)`, backticks and `<(...)`, then its commands in order.
    fn scan(&mut self, text: &str, depth: u32, mut cwd: Option<String>) {
        if depth > MAX_DEPTH {
            self.add(TOO_NESTED);
            return;
        }
        for body in subshell_bodies(text) {
            self.scan(&body, depth + 1, cwd.clone());
        }
        for (segment, piped) in split_segments(text) {
            let words = tokenize(&segment);
            self.command(&words, depth, &mut cwd, piped);
        }
    }

    /// One command: the words of a single simple command, wrappers and all.
    fn command(&mut self, words: &[String], depth: u32, cwd: &mut Option<String>, piped: bool) {
        let mut i = 0;
        loop {
            // Keywords of compound commands and `VAR=value` prefixes come first.
            while i < words.len() && (is_keyword(&words[i]) || is_assignment(&words[i])) {
                i += 1;
            }
            let Some(first) = words.get(i) else { return };
            let program = base_name(first);
            let rest = &words[i + 1..];
            match program.as_str() {
                "env" => i += 1 + skip_env(rest),
                "sudo" | "doas" => i += 1 + skip_sudo(rest),
                "command" | "exec" | "builtin" | "nohup" | "setsid" | "unbuffer" | "time" | "chronic" | "stdbuf" => i += 1 + skip_flags(rest),
                "nice" => i += 1 + skip_valued(rest, &["-n", "--adjustment"]),
                "ionice" => i += 1 + skip_valued(rest, &["-c", "-n", "-p", "--class", "--classdata"]),
                "timeout" => {
                    let flags = skip_valued(rest, &["-s", "-k", "--signal", "--kill-after"]);
                    // the duration is the first plain word after the flags
                    i += 1 + flags + usize::from(rest.len() > flags);
                }
                "xargs" => i += 1 + skip_valued(rest, &["-I", "-i", "-n", "-P", "-d", "-E", "-L", "-s", "-a"]),
                "cd" | "pushd" => {
                    *cwd = rest.iter().find(|w| !w.starts_with('-')).map_or_else(|| Some(WORK_ROOT.to_string()), |target| resolve(target, cwd.as_deref()));
                    return;
                }
                "eval" => {
                    self.scan(&rest.join(" "), depth + 1, cwd.clone());
                    return;
                }
                "bash" | "sh" | "zsh" | "dash" | "ksh" | "ash" | "fish" | "su" => {
                    self.shell(&program, rest, depth, cwd, piped);
                    return;
                }
                "find" => {
                    self.find(rest, depth, cwd);
                    return;
                }
                _ => {
                    self.program(&program, rest, cwd.as_deref());
                    return;
                }
            }
        }
    }

    /// `bash -c "..."` and its cousins. The script text is read as shell again, in a copy of the folder.
    fn shell(&mut self, program: &str, args: &[String], depth: u32, cwd: &Option<String>, piped: bool) {
        let mut k = 0;
        while k < args.len() {
            let arg = &args[k];
            if matches!(arg.as_str(), "-o" | "+o" | "-O" | "+O" | "--rcfile" | "--init-file") {
                k += 2;
                continue;
            }
            let short_flags = arg.len() > 1 && (arg.starts_with('-') || arg.starts_with('+')) && !arg.starts_with("--");
            if short_flags && arg.contains('c') {
                if let Some(script) = args.get(k + 1) {
                    self.scan(script, depth + 1, cwd.clone());
                }
                return;
            }
            if arg.starts_with('-') || arg.starts_with('+') {
                k += 1;
                continue;
            }
            // a script file: its text is not seen here
            return;
        }
        // No -c and no file: the shell reads its commands from what is piped in.
        if piped && program != "su" {
            self.add(PIPED_SHELL);
        }
    }

    /// `find ... -exec cmd ... ;` runs `cmd`.
    fn find(&mut self, args: &[String], depth: u32, cwd: &mut Option<String>) {
        let mut k = 0;
        while k < args.len() {
            if matches!(args[k].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                let end = args[k + 1..].iter().position(|a| a == ";" || a == "+").map_or(args.len(), |p| k + 1 + p);
                let mut inner = cwd.clone();
                self.command(&args[k + 1..end], depth + 1, &mut inner, false);
                k = end;
            }
            k += 1;
        }
    }

    /// The program and its arguments, after all wrappers are gone.
    fn program(&mut self, program: &str, args: &[String], cwd: Option<&str>) {
        let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
        match program {
            "git" => self.git(&args_ref),
            "npm" | "pnpm" | "yarn" | "bun" | "lerna" | "changeset" | "changesets" => {
                if positionals(&args_ref).iter().take(2).any(|p| *p == "publish") {
                    self.add(PUBLISH);
                }
            }
            "cargo" => {
                if positionals(&args_ref).iter().filter(|p| !p.starts_with('+')).take(1).any(|p| *p == "publish") {
                    self.add(PUBLISH);
                }
            }
            "twine" => {
                if positionals(&args_ref).first() == Some(&"upload") {
                    self.add(PUBLISH);
                }
            }
            "gem" => {
                if positionals(&args_ref).first() == Some(&"push") {
                    self.add(PUBLISH);
                }
            }
            "docker" | "podman" | "nerdctl" => {
                if positionals(&args_ref).first() == Some(&"push") {
                    self.add(PUBLISH);
                }
            }
            "ssh" | "scp" | "sftp" | "mosh" | "sshpass" | "ssh-copy-id" => self.add(REMOTE),
            "rsync" => {
                if args.iter().filter(|a| !a.starts_with('-')).any(|a| looks_remote(a)) {
                    self.add(REMOTE);
                }
            }
            "gh" => self.gh(&args_ref),
            "vercel" | "netlify" | "flyctl" | "fly" | "heroku" | "wrangler" | "firebase" | "serverless" | "sls" | "railway" | "surge" | "supabase" | "amplify" | "cdk" | "pulumi" => {
                let words = positionals(&args_ref);
                let deploys = args.iter().any(|a| {
                    matches!(a.as_str(), "deploy" | "publish" | "release" | "push" | "promote" | "up" | "destroy" | "--prod" | "--production")
                });
                let bare = matches!(program, "vercel" | "surge") && words.is_empty() && !args.iter().any(|a| matches!(a.as_str(), "--version" | "-v" | "--help" | "-h"));
                if deploys || bare {
                    self.add(DEPLOY);
                }
            }
            "terraform" | "tofu" => {
                if positionals(&args_ref).first().is_some_and(|p| matches!(*p, "apply" | "destroy")) {
                    self.add(DEPLOY);
                }
            }
            "kubectl" | "oc" => {
                if positionals(&args_ref).first().is_some_and(|p| matches!(*p, "apply" | "delete" | "replace" | "patch" | "create")) {
                    self.add(DEPLOY);
                }
            }
            "helm" => {
                if positionals(&args_ref).first().is_some_and(|p| matches!(*p, "install" | "upgrade" | "uninstall" | "rollback" | "delete")) {
                    self.add(DEPLOY);
                }
            }
            "rm" => self.rm(args, cwd),
            "dropdb" | "dropuser" | "dropdatabase" => self.add(DROP_DB),
            "psql" | "mysql" | "mariadb" | "sqlite3" | "mongosh" | "mongo" | "sqlcmd" | "clickhouse-client" | "cockroach" | "usql" | "mysqladmin" | "redis-cli" | "pg_restore" => {
                self.db_client = true;
            }
            _ => {}
        }
    }

    fn git(&mut self, args: &[&str]) {
        // Global options come before the subcommand; a few of them take a value.
        let mut k = 0;
        while k < args.len() && args[k].starts_with('-') {
            let takes_value = matches!(args[k], "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" | "--super-prefix");
            k += if takes_value { 2 } else { 1 };
        }
        let Some(&sub) = args.get(k) else { return };
        let rest = &args[k + 1..];
        match sub {
            "push" => {
                if rest.iter().any(|a| matches!(*a, "-n" | "--dry-run")) {
                    return;
                }
                let forced = rest.iter().any(|a| {
                    matches!(*a, "-f" | "--force" | "--mirror" | "--delete" | "-d") || a.starts_with("--force-with-lease") || a.starts_with("--force-if-includes") || (a.starts_with('+') && a.len() > 1)
                });
                self.add(if forced { GIT_FORCE } else { GIT_PUSH });
            }
            "reset" => {
                if rest.contains(&"--hard") {
                    self.add(GIT_RESET);
                }
            }
            _ => {}
        }
    }

    fn gh(&mut self, args: &[&str]) {
        let mut plain = Vec::new();
        let mut k = 0;
        while k < args.len() {
            if matches!(args[k], "-R" | "--repo" | "--hostname") {
                k += 2;
                continue;
            }
            if !args[k].starts_with('-') {
                plain.push(args[k]);
            }
            k += 1;
        }
        match (plain.first().copied(), plain.get(1).copied()) {
            (Some("pr"), Some("merge")) => self.add(GH_MERGE),
            (Some("release"), _) => self.add(GH_RELEASE),
            (Some("repo"), Some("delete")) => self.add(GH_DELETE),
            _ => {}
        }
    }

    fn rm(&mut self, args: &[String], cwd: Option<&str>) {
        let mut recursive = false;
        let mut targets = Vec::new();
        let mut flags_over = false;
        for arg in args {
            if flags_over || !arg.starts_with('-') || arg == "-" {
                targets.push(arg.as_str());
            } else if arg == "--" {
                flags_over = true;
            } else if let Some(long) = arg.strip_prefix("--") {
                if long == "recursive" {
                    recursive = true;
                }
            } else if arg[1..].chars().any(|c| c == 'r' || c == 'R') {
                recursive = true;
            }
        }
        if recursive && targets.iter().any(|t| !inside_work(t, cwd)) {
            self.add(RM);
        }
    }
}

// ---- reading shell text ---------------------------------------------------------------------------------------------

fn is_keyword(word: &str) -> bool {
    matches!(word, "if" | "then" | "else" | "elif" | "fi" | "do" | "done" | "while" | "until" | "{" | "}" | "!" | "for" | "case" | "esac")
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else { return false };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `/usr/bin/git`, `C:\Git\bin\git.exe`, `./git` -> `git`.
fn base_name(word: &str) -> String {
    let last = word.rsplit(['/', '\\']).next().unwrap_or(word);
    last.strip_suffix(".exe").unwrap_or(last).to_string()
}

fn skip_flags(args: &[String]) -> usize {
    args.iter().take_while(|a| a.starts_with('-') && a.len() > 1).count()
}

/// Flags, some of which take the next word.
fn skip_valued(args: &[String], valued: &[&str]) -> usize {
    let mut k = 0;
    while k < args.len() && args[k].starts_with('-') && args[k].len() > 1 {
        k += if valued.contains(&args[k].as_str()) { 2 } else { 1 };
    }
    k.min(args.len())
}

fn skip_env(args: &[String]) -> usize {
    let mut k = 0;
    while k < args.len() {
        let a = args[k].as_str();
        if matches!(a, "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string") {
            k += 2;
        } else if a.starts_with('-') && a.len() > 1 || is_assignment(a) {
            k += 1;
        } else {
            break;
        }
    }
    k.min(args.len())
}

fn skip_sudo(args: &[String]) -> usize {
    let mut k = 0;
    while k < args.len() {
        let a = args[k].as_str();
        if matches!(a, "-u" | "-g" | "-h" | "-p" | "-C" | "-r" | "-t" | "-T" | "-U" | "-D" | "--user" | "--group" | "--host" | "--prompt") {
            k += 2;
        } else if a.starts_with('-') && a.len() > 1 || is_assignment(a) {
            k += 1;
        } else {
            break;
        }
    }
    k.min(args.len())
}

/// The words that are not flags.
fn positionals<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().take_while(|a| *a != "--").filter(|a| !a.starts_with('-')).collect()
}

fn looks_remote(arg: &str) -> bool {
    if arg.starts_with("rsync://") || arg.starts_with("ssh://") {
        return true;
    }
    match arg.split_once(':') {
        Some((host, _)) => !host.is_empty() && !host.contains('/') && host.len() > 1,
        None => false,
    }
}

/// The text of every `$(...)`, backtick pair and `<(...)` / `>(...)` that is not inside single quotes.
fn subshell_bodies(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut in_single, mut in_double) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && !in_single {
            i += 2;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if !in_single {
            let opens = chars.get(i + 1) == Some(&'(') && (c == '$' || (!in_double && (c == '<' || c == '>')));
            if opens {
                let (body, end) = matching_paren(&chars, i + 2);
                out.push(body);
                i = end;
                continue;
            }
            if c == '`' {
                let mut j = i + 1;
                let mut body = String::new();
                while j < chars.len() && chars[j] != '`' {
                    if chars[j] == '\\' && j + 1 < chars.len() {
                        j += 1;
                    }
                    body.push(chars[j]);
                    j += 1;
                }
                out.push(body);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// From just after an opening parenthesis: the text up to its match, and the index after it.
fn matching_paren(chars: &[char], start: usize) -> (String, usize) {
    let mut depth = 1;
    let mut in_single = false;
    let mut j = start;
    let mut body = String::new();
    while j < chars.len() {
        let c = chars[j];
        if c == '\\' && !in_single && j + 1 < chars.len() {
            body.push(c);
            body.push(chars[j + 1]);
            j += 2;
            continue;
        }
        if c == '\'' {
            in_single = !in_single;
        } else if !in_single {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    return (body, j + 1);
                }
            }
        }
        body.push(c);
        j += 1;
    }
    (body, chars.len())
}

/// Splits at `;`, `&`, `&&`, `|`, `||`, newlines and parentheses that are not quoted. `true`: the segment follows a `|`.
fn split_segments(text: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut piped = false;
    let (mut in_single, mut in_double) = (false, false);
    let mut i = 0;
    let flush = |cur: &mut String, piped: &mut bool, next_piped: bool, out: &mut Vec<(String, bool)>| {
        if !cur.trim().is_empty() {
            out.push((std::mem::take(cur), *piped));
        } else {
            cur.clear();
        }
        *piped = next_piped;
    };
    while i < chars.len() {
        let c = chars[i];
        if in_single {
            cur.push(c);
            if c == '\'' {
                in_single = false;
            }
        } else if c == '\\' && i + 1 < chars.len() {
            if chars[i + 1] == '\n' {
                cur.push(' ');
            } else {
                cur.push(c);
                cur.push(chars[i + 1]);
            }
            i += 1;
        } else if in_double {
            cur.push(c);
            if c == '"' {
                in_double = false;
            }
        } else {
            match c {
                '\'' => {
                    in_single = true;
                    cur.push(c);
                }
                '"' => {
                    in_double = true;
                    cur.push(c);
                }
                ';' | '&' | '\n' | '(' | ')' => flush(&mut cur, &mut piped, false, &mut out),
                '|' => {
                    if chars.get(i + 1) == Some(&'|') {
                        i += 1;
                        flush(&mut cur, &mut piped, false, &mut out);
                    } else {
                        if chars.get(i + 1) == Some(&'&') {
                            i += 1;
                        }
                        flush(&mut cur, &mut piped, true, &mut out);
                    }
                }
                _ => cur.push(c),
            }
        }
        i += 1;
    }
    flush(&mut cur, &mut piped, false, &mut out);
    out
}

/// Splits one simple command into words, taking the quotes out (`'git' "push"` -> `git`, `push`).
fn tokenize(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut chars = segment.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                has = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                has = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.peek().copied() {
                            Some(n) if matches!(n, '"' | '\\' | '$' | '`') => {
                                cur.push(n);
                                chars.next();
                            }
                            _ => cur.push('\\'),
                        },
                        _ => cur.push(c),
                    }
                }
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    has = true;
                    cur.push(n);
                }
            }
            '#' if !has => break,
            c if c.is_whitespace() => {
                if has {
                    words.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            _ => {
                has = true;
                cur.push(c);
            }
        }
    }
    if has {
        words.push(cur);
    }
    words
}

// ---- paths ----------------------------------------------------------------------------------------------------------

/// An absolute, normalised path for a word, or `None` when it cannot be known (a variable, a command, `cd -`).
fn resolve(word: &str, cwd: Option<&str>) -> Option<String> {
    if word.contains('$') || word.contains('`') || word == "-" {
        return None;
    }
    let absolute = if word == "~" {
        WORK_ROOT.to_string()
    } else if let Some(rest) = word.strip_prefix("~/") {
        format!("{WORK_ROOT}/{rest}")
    } else if word.starts_with('~') {
        return None;
    } else if word.starts_with('/') {
        word.to_string()
    } else {
        format!("{}/{}", cwd?, word)
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in absolute.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

/// The scratch folder of the sandbox: deleting inside it is as harmless as deleting in the work folder.
pub const TEMP_ROOT: &str = "/tmp";

/// Is this target safely inside the work folder (or the temp folder)? Below the first level of the home folder or of
/// `/tmp`, not a hidden folder, and no wildcard at that first level. A path under `/tmp` may not use `..` at all.
/// Anything that cannot be worked out counts as outside.
pub fn inside_work(target: &str, cwd: Option<&str>) -> bool {
    let Some(path) = resolve(target, cwd) else { return false };
    let under = |root: &str| path.strip_prefix(root).and_then(|r| r.strip_prefix('/'));
    let Some(below) = under(WORK_ROOT).or_else(|| under(TEMP_ROOT).filter(|_| !target.split('/').any(|p| p == ".."))) else { return false };
    let first = below.split('/').next().unwrap_or("");
    !first.is_empty() && !first.starts_with('.') && !first.contains(['*', '?', '[', '{'])
}

/// Does the text drop or empty a database (SQL or a client's own command)?
fn drops_data(command: &str) -> bool {
    let lower = command.to_ascii_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    ["drop database", "drop schema", "drop table", "truncate table", "dropdatabase", "flushall", "flushdb", "drop keyspace"].iter().any(|needle| lower.contains(needle))
}
