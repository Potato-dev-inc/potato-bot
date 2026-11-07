//! Source guards for Spaces. They read source files, so a change that breaks a rule fails here.
use std::path::{Path, PathBuf};

fn tauri_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn packages() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn files(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
        return out;
    }
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "node_modules" || name == "target" || name == "dist" {
                continue;
            }
            out.extend(files(&p, exts));
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.contains(&e)) {
            out.push(p);
        }
    }
    out
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default().replace("\r\n", "\n")
}

/// Comments dropped, so the words of a rule written in a comment do not trip it.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}


fn spaces_dir() -> PathBuf {
    tauri_src().join("spaces")
}

/// Every .rs file in spaces/ with its file name.
fn spaces_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in files(&spaces_dir(), &["rs"]) {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        out.push((name, code_only(&read(&p))));
    }
    out
}

const MAY_NAME_P2P: [&str; 4] = ["net.rs", "net_tests.rs", "local_net_tests.rs", "spike.rs"];
const NOT_SHIPPED: [&str; 5] = ["guard_tests.rs", "spike.rs", "net_tests.rs", "local_net_tests.rs", "memnet.rs"];
const PURE: [&str; 9] = [
    "ids.rs", "wire.rs", "words.rs", "identity.rs", "roster.rs", "invite.rs", "log.rs", "sync.rs", "limits.rs",
];

#[test]
fn spaces_is_not_reachable_from_the_bridge_mcp_or_the_phone() {
    let src = tauri_src();
    let pk = packages();
    let roots: Vec<PathBuf> = vec![
        src.join("bridge"),
        src.join("tool_registry"),
        src.join("mcp_shim"),
        src.join("companion"),
        src.join("mobile_pairing"),
        src.join("channels"),
        src.join("bin"),
        pk.join("mcp-server/src"),
        pk.join("cli/src"),
        pk.join("app/src/pwa"),
        pk.join("app/src/PwaApp.tsx"),
        pk.join("app/src/PwaLegacyApp.tsx"),
    ];
    let words = ["spaces::", "space_send", "spaces_view", "space_join", "SpaceNet", "fish_bridge", "bot_bridge", "lib/spaces"];
    for root in roots {
        for f in files(&root, &["rs", "ts", "tsx"]) {
            let code = code_only(&read(&f));
            for w in words {
                assert!(!code.contains(w), "{} mentions {w}", f.display());
            }
        }
    }
}

#[test]
fn only_net_rs_names_the_p2p_module() {
    for (name, code) in spaces_files() {
        if MAY_NAME_P2P.contains(&name.as_str()) || name == "guard_tests.rs" {
            continue;
        }
        // Written as pieces so this file does not itself match the p2p guard's pattern.
        let (path_sep, crate_path) = (concat!("p2p", "::"), concat!("crate::", "p2p"));
        assert!(!code.contains(path_sep) && !code.contains(crate_path), "{name} names p2p");
    }
}

#[test]
fn spaces_never_uses_pairing_trust_the_bridge_or_team_transports() {
    let words = [
        "PairManager",
        "TrustList",
        "pairing",
        "bridge::",
        "mobile_pairing",
        "RelayTransport",
        "FolderTransport",
        "TeamTransport",
        "pair_answer",
        "trust_set_permission",
    ];
    for (name, code) in spaces_files() {
        if name == "guard_tests.rs" {
            continue;
        }
        for w in words {
            assert!(!code.contains(w), "{name} uses {w}");
        }
    }
}

#[test]
fn spaces_code_prints_nothing_and_reads_no_env() {
    let words = ["println!", "eprintln!", "dbg!", "env::var", "env::args"];
    for (name, code) in spaces_files() {
        if NOT_SHIPPED.contains(&name.as_str()) {
            continue;
        }
        for w in words {
            assert!(!code.contains(w), "{name} uses {w}");
        }
    }
}

#[test]
fn pure_spaces_files_open_no_files_or_sockets() {
    let words = ["std::fs", "tokio::fs", "File::", "tokio::net", "std::net::Tcp", "UdpSocket", "reqwest", "ureq"];
    for (name, code) in spaces_files() {
        if !PURE.contains(&name.as_str()) {
            continue;
        }
        for w in words {
            assert!(!code.contains(w), "{name} uses {w}");
        }
    }
}

/// The bot bridge (file `fish_bridge.rs` in the first plan, `bot_bridge.rs` after the public/private change) has no
/// route to tools. A turn from anyone but the bot's owner gets memory-read tools only (enforced by the host).
#[test]
fn bot_bridge_has_no_route_to_tools() {
    for file in ["fish_bridge.rs", "bot_bridge.rs"] {
        let p = spaces_dir().join(file);
        if !p.exists() {
            continue;
        }
        let code = code_only(&read(&p));
        for w in ["tool_registry", "Command::new", "std::fs", "mcp", "shell", "computer", "spawn_agent"] {
            assert!(!code.contains(w), "{file} uses {w}");
        }
    }
}

#[test]
fn limits_match_the_founder_defaults() {
    use super::limits::*;
    assert_eq!(MAX_MEMBERS, 50);
    assert_eq!(MAX_CHANNELS, 20);
    assert_eq!(KEEP_MS, 7_776_000_000);
    assert_eq!(KEEP_BYTES, 209_715_200);
    assert_eq!(INVITE_TTL_MS, 86_400_000);
    assert_eq!(INVITE_MAX_STRIKES, 5);
    assert_eq!(BOT_DAY_CAP, 30);
    assert_eq!(BOT_PERSON_HOUR_CAP, 5);
    assert_eq!(BOT_UNPROMPTED_CAP, 4);
}

/// Cowork M2: nothing a member writes can reach merge, approve or action code. Messages offer Reply, React, Delete.
#[test]
fn spaces_has_no_action_route_from_member_content() {
    let words = ["merge_green", "team::actions", "quote_untrusted", "auto_approve"];
    for (name, code) in spaces_files() {
        if name == "guard_tests.rs" {
            continue;
        }
        for w in words {
            assert!(!code.contains(w), "{name} uses {w}");
        }
    }
}

/// Rule 17: the file that decides which bot gets a line names no tool and no file. Which tools a turn may use is decided
/// where the turn runs (the brain's allow-list and the engine's refusal), never here.
#[test]
fn bot_bridge_names_no_tool_and_no_file() {
    let code = code_only(&read(&spaces_dir().join("bot_bridge.rs")));
    assert!(!code.is_empty(), "bot_bridge.rs is missing");
    let words = [
        "memory_search", "memory_open", "memory_timeline", "memory_diary", "space_diary", "read_history", "search_history",
        "skill_read", "sandbox", "helper", "hand_off", "use_tool", "run_tool", "persona_note", "web_fetch", "run_terminal",
        "vault", "journal", ".jsonl", "OpenOptions", "std::fs", "tokio::fs", "File::", "read_to_string", "PathBuf::from",
        "tool_registry", "Command::new", "mcp", "shell", "computer", "spawn_agent", "connector", "schedule",
    ];
    for w in words {
        assert!(!code.contains(w), "bot_bridge.rs uses {w}");
    }
}
