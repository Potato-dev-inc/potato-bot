//! The owner's own agent: the definition check, the approval, the launch script and its environment. Fakes only. The security
//! tests here are never deleted: an unapproved or changed definition must not run, a path or a shell word must not get in, and
//! no secret of ours may reach the agent's environment or the connector hub.

use super::custom_agents::*;
use super::sandbox::ShellOutput;

pub(super) const GOOD: &str = r#"schema_version = 1

[agent]
id = "scribe"
label = "Scribe"
description = "A small writing agent."

[binary]
names = ["scribe"]

[headless]
argv = ["run", "--plain"]
task_separator = "--"

[env]
set = { SCRIBE_MODE = "plain" }
"#;

fn err_of(text: &str) -> DefError {
    parse(text).expect_err("the definition is refused")
}

#[test]
fn a_good_definition_gets_a_plain_summary_with_a_fingerprint() {
    let s = summary(GOOD).unwrap();
    assert_eq!((s.id.as_str(), s.label.as_str()), ("scribe", "Scribe"));
    assert_eq!(s.command, "scribe run --plain -- <your message>");
    assert!(s.installs.contains("Nothing") && s.installs.contains("already be in the sandbox"));
    assert_eq!(s.env_names, ["SCRIBE_MODE"]);
    assert_eq!(s.fingerprint, fingerprint(GOOD));
    assert!(s.notes.iter().any(|n| n == DECIDES_NOTE) && s.notes.iter().any(|n| n == OWN_ACCOUNT_NOTE));
}

#[test]
fn a_mistake_shows_its_line_and_an_unknown_field_is_refused() {
    let e = err_of(&GOOD.replace("task_separator = \"--\"", "task_separator = \"--\"\nstdin = \"x\""));
    assert!(e.message.contains("does not take"), "{e}");
    assert_eq!(e.line, Some(14), "{e}");
    let e = err_of(&GOOD.replace("[binary]", "[binary]\nversion_args = [\"-v\"]"));
    assert!(e.line.is_some(), "{e}");
    let e = err_of(&GOOD.replace("schema_version = 1", "schema_version = 2"));
    assert_eq!(e.line, Some(1));
    let e = err_of("schema_version = 1\n[agent\nid = 1");
    assert!(e.line.is_some());
    assert!(e.to_string().starts_with("Line "));
}

#[test]
fn a_definition_is_data_no_path_no_shell_word_no_unknown_placeholder() {
    for bad in ["../bin/evil", "/usr/bin/evil", "evil;ls", "a b", "evil|x", "$(x)", "-rf", "evil\\x", ".."] {
        let e = err_of(&GOOD.replace("names = [\"scribe\"]", &format!("names = [{bad:?}]")));
        assert!(e.message.contains("bare name"), "{bad}: {e}");
    }
    for bad in ["--x=$HOME", "a;b", "`x`", "a|b", "a&b", "a>b", "../x", "x\\y", "it's", "{model}", "a{task}b{task}"] {
        let text = GOOD.replace("argv = [\"run\", \"--plain\"]", &format!("argv = [\"{}\"]", bad.replace('\\', "\\\\").replace('"', "\\\"")));
        assert!(parse(&text).is_err(), "{bad} was accepted");
    }
    assert!(parse(&GOOD.replace("argv = [\"run\", \"--plain\"]", "argv = [\"run\", \"{task}\"]")).is_ok());
}

#[test]
fn secret_looking_and_our_own_names_never_get_into_the_environment() {
    for name in ["OPENAI_API_KEY", "GITHUB_TOKEN", "DB_PASSWORD", "POTATO_BRAIN_KEY", "FISHD_PROVIDER", "PM_SBX_TOKEN", "DEEPSEEK_API_KEY", "PATH", "HOME", "LD_PRELOAD", "NODE_OPTIONS", "MY_SESSION", "CONNECTOR_URL"] {
        let e = err_of(&GOOD.replace("SCRIBE_MODE", name));
        assert!(e.message.contains(name), "{name}: {e}");
    }
    assert!(parse(&GOOD.replace("SCRIBE_MODE", "SCRIBE_THEME")).is_ok());
}

#[test]
fn the_owner_approves_a_fingerprint_and_a_changed_text_is_not_run() {
    let mut list = Vec::new();
    let agent = approve(&mut list, GOOD, 5).unwrap();
    assert_eq!(agent.fingerprint, fingerprint(GOOD));
    assert!(approved_spec(&list, "scribe").is_ok());
    assert!(approved_spec(&list, "other").unwrap_err().message.contains("not approved"));
    list[0].toml = GOOD.replace("--plain", "--evil");
    assert!(approved_spec(&list, "scribe").unwrap_err().message.contains("changed"));
    let mut full = Vec::new();
    approve(&mut full, GOOD, 1).unwrap();
    assert!(approve(&mut full, GOOD, 1).is_err(), "the same id twice");
    for i in 0..MAX_AGENTS_PER_BOT + 2 {
        let text = GOOD.replace("id = \"scribe\"", &format!("id = \"scribe{}\"", (b'a' + i as u8) as char));
        let _ = approve(&mut full, &text, 1);
    }
    assert_eq!(full.len(), MAX_AGENTS_PER_BOT);
}

fn spec() -> Spec {
    parse(GOOD).unwrap()
}

#[test]
fn the_launch_script_quotes_the_task_drops_to_its_own_user_and_names_no_secret() {
    let evil = "hi'; rm -rf / #\n$(cat /run/pm-sbx/control.token) `x`";
    let script = launch_script(&spec(), evil, 300, "nimbus", &[]);
    assert!(script.contains("'hi'\"'\"'; rm -rf / #\n$(cat /run/pm-sbx/control.token) `x`'"), "{script}");
    assert!(script.contains("timeout -k 5 300 unshare -m --propagation private env -i "));
    assert!(script.contains("exec setpriv --reuid=59999 --regid=59999 --clear-groups --no-new-privs \"$@\" </dev/null"));
    assert!(!script.contains("exec timeout"), "the shell stays, so it can kill what the agent left");
    assert!(script.contains("</dev/null"));
    assert!(script.contains("SCRIBE_MODE=plain"));
    for path in ["/run/pm-sbx/control.token", "/run/pm-sbx/vnc-view.pass", "/run/pm-sbx/vnc-control.pass", "$PMH"] {
        assert!(script.contains(path), "{path}");
    }
    assert!(script.contains("fail 71"));
    assert_ne!(AGENT_UID, 0);
    assert!(!(60000..64000).contains(&AGENT_UID), "outside the connector hub's range");
    assert!(script.contains("PATH=/usr/local/bin:/usr/bin:/bin command -v 'scribe'"));
}

#[test]
fn only_the_allow_list_is_in_the_agents_environment_even_when_the_app_has_secrets() {
    let pairs = env_pairs(&spec());
    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["HOME", "PATH", "LANG", "SCRIBE_MODE"]);
    // run `env -i <the pairs> env` with secrets in the parent's own environment: none comes out
    let path = std::env::var("PATH").unwrap_or_default();
    let mut cmd = std::process::Command::new("env");
    cmd.env("DEEPSEEK_API_KEY", "sekret-1").env("POTATO_TOKEN", "sekret-2").env("PM_SBX_TOKEN", "sekret-3").env("CONNECTOR_TOKEN", "sekret-4").arg("-i");
    for (n, v) in &pairs {
        cmd.arg(format!("{n}={}", if n == "PATH" { path.as_str() } else { v.as_str() }));
    }
    cmd.arg("env");
    let Ok(out) = cmd.output() else { return };
    if !out.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("SCRIBE_MODE=plain"), "{text}");
    assert!(!text.contains("sekret") && !text.contains("DEEPSEEK") && !text.contains("POTATO_") && !text.contains("PM_SBX") && !text.contains("CONNECTOR"), "{text}");
}

#[test]
fn the_connector_hub_is_not_in_reach_of_the_agent_code() {
    // The hub is a root process the host talks to over a pipe (no port, no socket, no file). The agent path never names it, its
    // pipe, the sign-in flow or the credential store, and never uses the bot's shell or the connector calls.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("potato");
    for file in ["custom_agents.rs", "service/agent_flow.rs"] {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap();
        let code: String = code.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        // The one read of the credential store the agent path makes: the agent's own values, under its own names.
        let code = code.replace("self.host.secret(&agents::agent_secret_name(", "");
        for word in ["secret_hub", "HUB_PATH", "pm-sbx-secret-hub", "secret_config", "mcp_call", "mcp_config", "root_shell", ".shell(", "login", "host.secret(", "SECRET_"] {
            assert!(!code.contains(word), "{file} names {word}");
        }
    }
    let script = launch_script(&spec(), "x", 10, "nimbus", &[]);
    assert!(!script.contains("secret-hub") && !script.contains("mcp-hub"));
}

#[test]
fn a_finished_run_reads_in_plain_words() {
    let run = |code: Option<i32>, out: &str, err: &str, timed_out: bool| ShellOutput { code, stdout: out.into(), stderr: err.into(), timed_out };
    assert_eq!(reading(&run(Some(0), "  hello \n", "", false), "Scribe").unwrap(), "hello");
    assert!(reading(&run(Some(0), "  ", "", false), "Scribe").unwrap_err().contains("without an answer"));
    assert!(reading(&run(None, "", "", true), "Scribe").unwrap_err().contains("took too long"));
    assert!(reading(&run(Some(124), "", "", false), "Scribe").unwrap_err().contains("took too long"));
    assert!(reading(&run(Some(127), "", "AGENT_NOT_INSTALLED\n", false), "Scribe").unwrap_err().contains("not installed in the sandbox"));
    assert!(reading(&run(Some(71), "", "The sandbox is not isolated enough", false), "Scribe").unwrap_err().contains("not isolated"));
    for code in [72, 73] {
        assert!(reading(&run(Some(code), "partial words", "The sandbox network wall is not in place. Nothing was started.", false), "Scribe").unwrap_err().contains("Nothing was started"), "{code}");
    }
    assert!(reading(&run(Some(2), "", "boom", false), "Scribe").unwrap_err().contains("code 2): boom"));
    let long = "x".repeat(MAX_REPLY_CHARS + 500);
    assert_eq!(reading(&run(Some(0), &long, "", false), "Scribe").unwrap().chars().count(), MAX_REPLY_CHARS);
}

#[test]
fn the_prompt_has_persona_notes_and_chat_and_keeps_the_newest_message_whole() {
    let p = build_prompt("Nimbus", "Be brief.", "- a.md: likes tea", &[("Owner".into(), "hi".into()), ("Nimbus".into(), "hello".into())], "what now?");
    assert!(p.contains("Be brief.") && p.contains("likes tea") && p.contains("Owner: hi") && p.contains("Nimbus: hello") && p.ends_with("what now?\n"));
    let big = build_prompt("N", &"p".repeat(20_000), "", &[], "the last words");
    assert!(big.chars().count() < MAX_TASK_CHARS + 100 && big.ends_with("the last words\n"));
}

// ---- isolation guards -----------------------------------------------------------------------------------------------

/// The text of one shell function in the script (up to its closing line).
fn function_of<'a>(script: &'a str, name: &str) -> &'a str {
    let start = script.find(&format!("{name}() {{")).unwrap_or_else(|| panic!("no function {name}"));
    let rest = &script[start..];
    let end = rest.find("\n}\n").map(|i| i + 3).unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn the_agents_user_is_outside_the_hubs_range_and_the_hub_refuses_it() {
    let hub = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../potato-sandbox/guest/control/pm-sbx-secret-hub")).unwrap();
    let number = |name: &str| -> u32 { hub.lines().find_map(|l| l.strip_prefix(name).and_then(|r| r.trim().strip_prefix('=')).map(|r| r.trim().parse().unwrap())).unwrap_or_else(|| panic!("hub has no {name}")) };
    let (base, span, agent) = (number("UID_BASE"), number("UID_SPAN"), number("AGENT_UID"));
    assert_eq!(agent, AGENT_UID, "the hub and the app name the same user");
    assert!(!(base..base + span).contains(&AGENT_UID), "no connector server can be given the agent's user");
    assert!(hub.contains("if uid == AGENT_UID:") && hub.contains("that server name is not allowed"), "and the hub refuses it if it ever came up");
}

#[test]
fn the_script_holds_every_guard_and_starts_the_agent_only_after_them() {
    let script = launch_script(&spec(), "x", 10, "nimbus", &[]);
    let guards_at = script.find("\nguards </dev/null\n").expect("guards are called");
    let start_at = script.find("timeout -k 5 10 ").expect("the agent is started");
    assert!(guards_at < start_at, "guards first");
    // loopback: a rule by owner uid for IPv4 and IPv6, checked in place, and a refusal without firewall tools
    for needle in [
        "-A PM_SBX_AGENT -o lo -m owner --uid-owner 59999 -j REJECT",
        "fw_ipt \"$IPT\" 4 ||",
        "fw_ipt \"$IP6\" 6 ||",
        "-d \"$ns\" -p udp --dport 53 -m owner --uid-owner 59999 -j RETURN",
        "-d \"$ns\" -p tcp --dport 53 -m owner --uid-owner 59999 -j RETURN",
        "has IPv6 but no way",
        "meta skuid 59999 oifname",
        "meta skuid 59999 ip daddr $ns udp dport 53 accept",
        "no firewall tools",
        "-I OUTPUT 1 -j PM_SBX_AGENT",
        "'-A OUTPUT -j PM_SBX_AGENT'",
    ] {
        assert!(script.contains(needle), "{needle}");
    }
    // the screen: the agent must be refused by the X server
    assert!(script.contains("DISPLAY=:99 \"$XD\"") && script.contains("The sandbox screen is open to other users"));
    // the bot home and secrets: closed, then tried for read, traverse and write as the agent
    assert!(script.contains("chmod o-rwx \"$PMH\"") && script.contains("test -r \"$1\" || test -x \"$1\" || test -w \"$1\""));
    for file in ["/run/pm-sbx/control.token", "/run/pm-sbx/vnc-view.pass", "/run/pm-sbx/vnc-control.pass"] {
        assert!(script.contains(file), "{file}");
    }
    // the secrets folder is closed (older sandboxes too) before it is checked
    assert!(script.contains("chown root:\"$(id -g pm)\" /run/pm-sbx") && script.contains("chmod 0750 /run/pm-sbx"));
    assert!(script.find("chmod 0750 /run/pm-sbx").unwrap() < script.find("for p in \"$PMH\" /run/pm-sbx").unwrap());
    // a private view: its own mounts over everything the bot or Windows leaves reachable, a size cap and a file-size limit
    assert!(script.contains("unshare -m --propagation private env -i ") && script.contains(" sh /run/pm-agent-ns.sh \"$PROG\" "));
    for needle in [
        "for d in /run/WSL /run/user; do",
        "mnt /dev/shm size=100m,mode=1777",
        "mnt /tmp size=100m,mode=1777",
        "mnt /var/tmp size=100m,mode=1777",
        "mount -o loop,nosuid,nodev /var/lib/pm-agents/nimbus--scribe.img /var/lib/pm-agent",
        "truncate -s 1024M /var/lib/pm-agents/nimbus--scribe.img.new",
        "mkfs.ext4 -q -F -m 0 -L pmagent -E root_owner=59999:59999",
        "e2fsck -p /var/lib/pm-agents/nimbus--scribe.img",
        "loop mounts may not work here",
        "ulimit -f 524288",
        "find /run /dev/shm -type s -user pm",
        "The sandbox could not hide $d from this agent",
        "command -v unshare",
        "exec setpriv --reuid=59999",
    ] {
        assert!(script.contains(needle), "{needle}");
    }
    // what is left: killed by user, before and after the run, and a leftover is a failure
    assert!(script.contains("agent_pids()") && script.contains("kill -KILL $p") && script.contains("clear_agent || fail 73"));
    assert!(script.contains("rc=$?\nclear_agent </dev/null || fail 73") && script.trim_end().ends_with("exit \"$rc\""));
    // the Windows PC and other reach-outs
    for needle in ["state=applied", "drvfs", "hidepid=2", "--no-new-privs"] {
        assert!(script.contains(needle), "{needle}");
    }
    assert!(!script.contains("WSLInterop"), "the VM-wide kernel entry is not a reason to refuse; the interop socket is hidden instead");
    // every guard failure is a refusal with its own code
    for code in ["fail 70", "fail 71", "fail 72", "fail 73", "bad 74"] {
        assert!(script.contains(code), "{code}");
    }
    assert!(!script.contains("size=512m"), "the home is an image now, not RAM");
    for left in ["@UID@", "@DROP@", "@HOME@", "@FSIZE@", "@TMPMB@", "@HOMEMB@", "@SECRETS@", "@IMG@", "@IMGDIR@", "@INPUT@"] {
        assert!(!script.contains(left), "{left} was not filled");
    }
}

#[test]
fn a_task_that_looks_like_a_placeholder_is_not_filled_in() {
    let script = launch_script(&spec(), "@UID@ @HOME@ @WORDS@ @DROP@", 10, "nimbus", &[]);
    assert!(script.contains("'@UID@ @HOME@ @WORDS@ @DROP@'"), "{script}");
}

#[test]
fn the_clean_up_script_kills_by_user_and_fails_when_one_is_left() {
    let kill = kill_script();
    assert!(kill.contains("awk -v U=59999") && kill.contains("kill -STOP $p") && kill.contains("kill -KILL $p"));
    assert!(kill.contains("exit 73") && kill.contains("AGENT_LEFTOVER"));
    assert!(kill.contains("/proc/[0-9]*"), "found by user, not by process group, so a child that called setsid is found too");
}

/// A POSIX shell, on unix only. On Windows `bash` is the WSL launcher (`System32\bash.exe`), which a test must never start.
fn bash() -> Option<String> {
    if !cfg!(unix) {
        return None;
    }
    ["bash", "sh"].iter().find(|b| std::process::Command::new(b).arg("-c").arg("exit 0").output().is_ok_and(|o| o.status.success())).map(|b| b.to_string())
}

/// Runs the `fw_ipt` function from the script against a stand-in `iptables` that answers like the real one; `listing` is the
/// lines `iptables -S OUTPUT` prints after the policy line, `rule_ok` is whether the append works.
fn run_fw_ipt(tag: &str, listing: &str, rule_ok: bool) -> Option<bool> {
    let shell = bash()?;
    let dir = std::env::temp_dir().join(format!("pm-fw-test-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let stub = dir.join("iptables");
    let body = format!(
        "#!/bin/sh\ncase \"$1\" in\n -N) exit 0;;\n -F) exit 0;;\n -D) exit 1;;\n -I) exit 0;;\n -A) {append};;\n -S) if [ \"$2\" = OUTPUT ]; then printf '%s\\n' '-P OUTPUT ACCEPT' {listing}; else echo '-N PM_SBX_AGENT'; echo '-A PM_SBX_AGENT -o lo -m owner --uid-owner 59999 -j REJECT --reject-with icmp-port-unreachable'; fi;;\nesac\n",
        append = if rule_ok { "exit 0" } else { "exit 1" },
    );
    std::fs::write(&stub, body).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    let script = launch_script(&spec(), "x", 10, "nimbus", &[]);
    let func = function_of(&script, "fw_ipt");
    let stub_path = stub.to_string_lossy().replace('\\', "/");
    let resolvers = function_of(&script, "resolvers");
    let program = format!("{resolvers}\n{func}\nfw_ipt '{stub_path}' 4\n");
    let status = std::process::Command::new(shell).arg("-c").arg(program).status().ok()?;
    let _ = std::fs::remove_file(&stub);
    let _ = std::fs::remove_dir(&dir);
    Some(status.success())
}

#[test]
fn the_firewall_step_passes_only_when_the_rule_is_first_and_in_place() {
    // the rule is first in OUTPUT and in its own chain: passes
    if let Some(ok) = run_fw_ipt("ok", "'-A OUTPUT -j PM_SBX_AGENT' '-A OUTPUT -j PM_SBX_EGRESS'", true) {
        assert!(ok, "a correct firewall passes");
        // the egress chain came first (its loopback accept would win): refused
        assert!(!run_fw_ipt("late", "'-A OUTPUT -j PM_SBX_EGRESS' '-A OUTPUT -j PM_SBX_AGENT'", true).unwrap(), "a rule that is not first is not in place");
        // the rule could not be added (no owner match in this kernel): refused
        assert!(!run_fw_ipt("noadd", "'-A OUTPUT -j PM_SBX_AGENT'", false).unwrap(), "a rule that failed to add is a refusal");
    }
}

#[test]
fn the_scripts_are_valid_shell() {
    use std::io::Write;
    let Some(shell) = bash() else { return };
    for script in [launch_script(&spec(), "it's \"quoted\" $(x)", 10, "nimbus", &[]), kill_script()] {
        let mut child = std::process::Command::new(&shell).arg("-n").stdin(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn the_private_view_hides_the_interop_socket_the_bot_sockets_and_temp_files_and_the_resolver_stays_open() {
    let script = launch_script(&spec(), "x", 10, "nimbus", &[]);
    // the namespace script is a quoted heredoc, so nothing in it is expanded by the outer shell
    assert!(script.contains("<<'PMNS'"));
    let inner = script.split("<<'PMNS'").nth(1).unwrap().split("\nPMNS\n").next().unwrap();
    // every mount fails closed, and the checks run after them, before the agent starts
    assert!(inner.find("mnt /tmp").unwrap() < inner.find("ls -A").unwrap() && inner.find("ls -A").unwrap() < inner.find("exec setpriv").unwrap());
    assert!(inner.contains("|| bad 74") && inner.contains("[ -z \"$(ls -A \"$d\" 2>/dev/null)\" ] || bad 72"));
    // the resolver is let through for the agent before the loopback is closed (the sandbox's resolver is an address on lo)
    let chain = function_of(&script, "fw_ipt");
    assert!(chain.find("-j RETURN").unwrap() < chain.find("-j REJECT").unwrap());
    // and the rule names the whole loopback device, so the guest's own addresses on lo are closed too
    assert!(chain.contains("-o lo -m owner --uid-owner 59999 -j REJECT"));
}

// ---- the agent's own secrets and its saved space --------------------------------------------------------------------

const WITH_SECRET: &str = r#"schema_version = 1

[agent]
id = "scribe"
label = "Scribe"

[binary]
names = ["scribe"]

[headless]
argv = ["run"]
task_separator = "--"

[env]
set = { SCRIBE_MODE = "plain" }
secret = ["SCRIBE_TOKEN", "OTHER_LOGIN"]
"#;

const SECRET_VALUE: &str = "tok-0123456789-abcdef-SECRET";

fn secret_spec() -> Spec {
    parse(WITH_SECRET).unwrap()
}

#[test]
fn an_agent_can_list_the_names_it_wants_typed_in_settings_and_the_summary_shows_them() {
    let s = summary(WITH_SECRET).unwrap();
    assert_eq!(s.secret_names, ["SCRIBE_TOKEN", "OTHER_LOGIN"]);
    assert!(s.notes.iter().any(|n| n.contains("It asks you for: SCRIBE_TOKEN, OTHER_LOGIN") && n.contains("keychain")), "{:?}", s.notes);
    assert!(s.notes.iter().any(|n| n.contains("its own 1 GB space") && !n.contains("gone when the turn ends")), "{:?}", s.notes);
    assert!(summary(GOOD).unwrap().secret_names.is_empty());
}

#[test]
fn a_secret_name_may_look_like_a_key_but_never_be_one_of_ours_or_the_sandboxs() {
    // looks like a key: fine under `secret`, refused under `set`
    assert!(parse(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", "secret = [\"OPENAI_API_KEY\", \"GITHUB_TOKEN\"]")).is_ok());
    assert!(parse(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", "set = { GITHUB_TOKEN = \"x\" }")).is_err(), "the rule for `set` stays");
    for name in ["DEEPSEEK_API_KEY", "OPENCODE_API_KEY", "POTATO_PROVIDER_API_KEY", "POTATO_TOKEN", "PM_POTATO_TOKEN", "FISHD_PROVIDER", "PM_SBX_TOKEN", "PATH", "HOME", "LD_PRELOAD", "DISPLAY", "CONNECTOR_URL"] {
        let e = err_of(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", &format!("secret = [\"{name}\"]")));
        assert!(e.message.contains(name) && (e.message.contains("Puppet Master uses") || e.message.contains("sandbox sets")), "{name}: {e}");
    }
    for bad in ["lower", "WITH SPACE", "9X", "A;B", ""] {
        assert!(parse(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", &format!("secret = [\"{bad}\"]"))).is_err(), "{bad:?}");
    }
    // twice, or also under `set`
    assert!(parse(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", "secret = [\"A_B\", \"A_B\"]")).is_err());
    assert!(parse(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", "set = { A_B = \"1\" }\nsecret = [\"A_B\"]")).is_err());
    let many: Vec<String> = (0..9).map(|i| format!("\"NAME_{i}\"")).collect();
    assert!(parse(&GOOD.replace("set = { SCRIBE_MODE = \"plain\" }", &format!("secret = [{}]", many.join(", ")))).is_err());
}

#[test]
fn a_secret_name_in_the_credential_store_has_one_shape_and_the_page_can_set_it_but_not_read_it_back() {
    let name = agent_secret_name("nimbus", "scribe", "SCRIBE_TOKEN");
    assert_eq!(name, "potato_agent_secret_nimbus_scribe_SCRIBE_TOKEN");
    assert!(is_agent_secret_name(&name));
    assert!(crate::secrets::is_ui_secret(&name) && crate::secrets::is_sidecar_secret(&name) && !crate::secrets::is_provider_secret(&name));
    for bad in ["potato_agent_secret_", "potato_agent_secret_../x", "potato_agent_secret_a b", "deepseek_api_key_x"] {
        assert!(!is_agent_secret_name(bad), "{bad}");
    }
    assert_ne!(agent_secret_name("nimbus", "scribe", "A"), agent_secret_name("mochi", "scribe", "A"), "two bots never share a value");
}

#[test]
fn a_secret_reaches_the_agent_only_through_the_here_document_never_an_argument() {
    let script = launch_script(&secret_spec(), "do the thing", 60, "nimbus", &[("SCRIBE_TOKEN".into(), SECRET_VALUE.into()), ("OTHER_LOGIN".into(), "pw-two-1234".into())]);
    assert_eq!(script.matches(SECRET_VALUE).count(), 1, "once, in the here-document");
    let (before, rest) = script.split_once("<<'PMSEC'\n").expect("a quoted here-document");
    let (body, after) = rest.split_once("\nPMSEC\n").expect("its end");
    assert_eq!(body, format!("SCRIBE_TOKEN={SECRET_VALUE}\nOTHER_LOGIN=pw-two-1234"));
    // nothing before it (the guards, the command line, the arguments) and nothing after it holds a value
    for part in [before, after] {
        assert!(!part.contains(SECRET_VALUE) && !part.contains("pw-two-1234"));
    }
    // the command line that starts the agent holds the name of nothing secret; the launcher reads and exports the lines
    let command = before.lines().last().unwrap();
    assert!(command.starts_with("timeout -k 5 60 unshare -m --propagation private env -i ") && !command.contains("SCRIBE_TOKEN") && !command.contains("OTHER_LOGIN"), "{command}");
    assert!(script.contains("while IFS= read -r line; do [ -n \"$line\" ] && export \"$line\"; done"));
    // a value with a line break, an empty one and a name the definition does not list are never passed
    let odd = launch_script(&secret_spec(), "x", 60, "nimbus", &[("SCRIBE_TOKEN".into(), "a\nPATH=/evil".into()), ("OTHER_LOGIN".into(), "".into()), ("UNLISTED".into(), "u-secret-value".into())]);
    assert!(!odd.contains("PATH=/evil") && !odd.contains("u-secret-value") && odd.contains("</dev/null") && !odd.contains("<<'PMSEC'"), "{odd}");
    // a value that is the end marker does not end the here-document early
    let tricky = launch_script(&secret_spec(), "x", 60, "nimbus", &[("SCRIBE_TOKEN".into(), "PMSEC".into())]);
    assert!(tricky.contains("<<'PMSEC'\nSCRIBE_TOKEN=PMSEC\nPMSEC\n"), "a line is NAME=value, never the bare marker");
    let marker_value = launch_script(&secret_spec(), "x", 60, "nimbus", &[("SCRIBE_TOKEN".into(), "v".into())]);
    assert!(marker_value.contains("<<'PMSEC'"));
}

#[test]
fn a_secret_the_agent_prints_is_taken_out_of_its_answer() {
    let secrets = vec![("SCRIBE_TOKEN".to_string(), SECRET_VALUE.to_string()), ("S".to_string(), "abc".to_string())];
    let shown = hide_secrets(&format!("my key is {SECRET_VALUE}, and abc"), &secrets);
    assert_eq!(shown, "my key is [hidden], and abc", "a value under four characters is left (it would eat ordinary words)");
}

#[test]
fn each_bot_and_agent_has_its_own_home_image_and_only_that_one_is_mounted() {
    assert_eq!(home_image("nimbus", "scribe"), "/var/lib/pm-agents/nimbus--scribe.img");
    assert_ne!(home_image("nimbus", "scribe"), home_image("mochi", "scribe"));
    assert_ne!(home_image("nimbus", "scribe"), home_image("nimbus", "other"));
    assert_eq!(home_image("../x", "a b"), "/var/lib/pm-agents/___x--a_b.img", "no path climb and no space in a file name");
    let script = launch_script(&spec(), "x", 10, "mochi", &[]);
    assert!(script.contains("/var/lib/pm-agents/mochi--scribe.img") && !script.contains("nimbus--scribe.img"));
    assert_eq!(script.matches("mount -o loop").count(), 1);
}
