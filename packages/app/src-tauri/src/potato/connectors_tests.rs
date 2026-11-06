//! Connectors (`mcp.rs`): the catalog is checked, a token goes only to the credential store and the guest's server
//! process, and every other place (settings file, views, notes, answers, shell line, errors) is shown to be free of it.
//! The gate tests (cards for reads and changes, rooms, other bots) are in `sandbox_calls_tests.rs`.
//! Tokens here are fake strings. The credential store is the in-memory one the tests use.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::json;

use super::bot_settings;
use super::mcp::{self, Install};
use super::sandbox::{FakeSandbox, Recorded, Sandbox};

const TOKEN: &str = "fake-token-8f3a91c2d7";

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-conn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn running() -> FakeSandbox {
    let sandbox = FakeSandbox::new();
    sandbox.start().unwrap();
    sandbox
}

fn github_token() -> BTreeMap<String, String> {
    BTreeMap::from([("GITHUB_PERSONAL_ACCESS_TOKEN".to_string(), format!("  {TOKEN}  "))])
}

fn entry(id: &str) -> &'static mcp::CatalogEntry {
    mcp::catalog().iter().find(|e| e.name == id).unwrap()
}

// ---- the catalog --------------------------------------------------------------------------------------

#[test]
fn every_connector_comes_from_a_pinned_source_the_makers_own_docs_name() {
    let connectors: Vec<&mcp::CatalogEntry> = mcp::catalog().iter().filter(|e| e.connector).collect();
    let names: Vec<&str> = connectors.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["github", "notion"]);
    for e in connectors {
        assert!(mcp::valid_server_name(&e.name));
        assert!(e.install.is_some() && e.install_fields_ok(), "{} installs from a checked source", e.name);
        assert!(e.docs.as_deref().is_some_and(|d| d.starts_with("https://github.com/")), "{} names the maker's page", e.name);
        assert!(e.maker.is_some() && !e.description.is_empty());
        assert!(!e.env.is_empty() && e.env.iter().all(|f| mcp::valid_env_key(&f.key) && !f.label.is_empty()));
        assert!(e.command_path().starts_with(mcp::CONNECTORS_DIR));
        let line = e.install_command().unwrap();
        assert!(!line.contains("latest") && !line.contains(TOKEN));
    }
    match &entry("github").install {
        Some(Install::Release { url, sha256, .. }) => {
            assert!(url.contains("/v2.1.0/") && sha256.len() == 64);
        }
        other => panic!("github installs from a release, got {other:?}"),
    }
}

#[test]
fn an_install_entry_with_odd_characters_is_refused() {
    let mut bent = entry("notion").clone();
    bent.install = Some(Install::Npm { spec: "pkg; rm -rf ~@1.0.0".into(), bin: "x".into() });
    assert!(!bent.install_fields_ok());
    bent.install = Some(Install::Npm { spec: "@scope/pkg@latest".into(), bin: "x".into() });
    assert!(!bent.install_fields_ok(), "a version, never latest");
    bent.install = Some(Install::Release { url: "https://evil.example/x.tgz".into(), sha256: "a".repeat(64), member: "x".into(), dir: "d".into() });
    assert!(!bent.install_fields_ok(), "github.com only");
    bent.install = Some(Install::Release { url: "https://github.com/a/b/x.tgz".into(), sha256: "zz".into(), member: "x".into(), dir: "d".into() });
    assert!(!bent.install_fields_ok(), "a real sha256");
}

#[test]
fn a_connector_is_not_added_as_a_plain_tool_server() {
    let root = temp("plain");
    let err = mcp::add_catalog(&root, "cn2a", "github", &running()).unwrap_err();
    assert_eq!(err.code, "MCP_USE_CONNECTORS");
    let _ = std::fs::remove_dir_all(&root);
}

// ---- connecting ---------------------------------------------------------------------------------------

#[test]
fn connecting_keeps_the_token_in_the_store_and_in_the_guest_list_only() {
    let root = temp("connect");
    let sandbox = running();
    mcp::connect(&root, "cn3a", "github", github_token(), &sandbox).unwrap();

    // The install line ran once on the computer and holds no token.
    let shells: Vec<String> = sandbox.calls().into_iter().filter_map(|c| if let Recorded::RootShell(line) = c { Some(line) } else { None }).collect();
    assert_eq!(shells.len(), 1, "the install runs as root, once");
    assert!(sandbox.calls().iter().all(|c| !matches!(c, Recorded::Shell(_))), "never as the bot's own user");
    assert!(shells[0].contains("sha256sum -c") && !shells[0].contains(TOKEN));

    // Settings: names only, marked as a connector, reads asking by default.
    let settings = bot_settings::load(&root, "cn3a").unwrap();
    let server = &settings.mcp_servers[0];
    assert!(server.connector && server.enabled && !server.always_allow_reads);
    assert_eq!(server.env_keys, vec!["GITHUB_PERSONAL_ACCESS_TOKEN".to_string()]);
    assert_eq!(server.command, "/opt/pm-connectors/github/github-mcp-server");
    let on_disk = std::fs::read_to_string(bot_settings::path(&root, "cn3a").unwrap()).unwrap();
    assert!(!on_disk.contains(TOKEN), "not in the settings file");

    // The page's view: names only.
    let view = serde_json::to_string(&mcp::overview(&root, "cn3a", &sandbox).unwrap()).unwrap();
    assert!(!view.contains(TOKEN), "not in what the page is sent");
    assert!(view.contains("\"connector\":true"));

    // The control service (the bot can reach it) is sent no connector at all, and no value.
    let sent = sandbox.last_mcp_config().unwrap();
    assert!(sent.as_array().unwrap().iter().all(|s| s["name"] != "github"), "the connector is not in the pm-side list");
    assert!(!sent.to_string().contains(TOKEN), "not in anything the control service is sent");
    // The value goes only to the root process the host talks to over its own pipe (trimmed).
    let secret = sandbox.last_secret_config().unwrap();
    assert_eq!(secret[0]["name"], json!("github"));
    assert_eq!(secret[0]["env"]["GITHUB_PERSONAL_ACCESS_TOKEN"], json!(TOKEN));
    assert_eq!(secret[0]["command"], json!("/opt/pm-connectors/github/github-mcp-server"));
    assert!(sandbox.calls().contains(&Recorded::SecretConfig(vec!["github".to_string()])));

    // Disconnecting removes the stored value too.
    mcp::remove_server(&root, "cn3a", "github", &sandbox).unwrap();
    assert!(crate::secrets::backend().get(&mcp::env_secret("cn3a", "github")).unwrap().is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failed_install_saves_nothing_and_the_error_has_no_token() {
    let root = temp("fail");
    let sandbox = running();
    sandbox.fail_next(super::sandbox::SandboxError::new("SANDBOX_SHELL", format!("curl failed with {TOKEN}")));
    let err = mcp::connect(&root, "cn4a", "github", github_token(), &sandbox).unwrap_err();
    assert_eq!(err.code, "CONNECTOR_INSTALL_FAILED");
    assert!(!err.message.contains(TOKEN) && !err.to_string().contains(TOKEN));
    assert!(bot_settings::load(&root, "cn4a").unwrap().mcp_servers.is_empty());
    assert!(crate::secrets::backend().get(&mcp::env_secret("cn4a", "github")).unwrap().is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn connecting_needs_the_right_fields_a_running_computer_and_one_bot_per_connector() {
    let root = temp("rules");
    let sandbox = running();
    let stopped = FakeSandbox::new();
    assert_eq!(mcp::connect(&root, "cn5a", "github", github_token(), &stopped).unwrap_err().code, "CONNECTOR_NEEDS_COMPUTER");
    assert_eq!(mcp::connect(&root, "cn5a", "nope", github_token(), &sandbox).unwrap_err().code, "MCP_UNKNOWN_CATALOG");
    assert_eq!(mcp::connect(&root, "cn5a", "memory", BTreeMap::new(), &sandbox).unwrap_err().code, "MCP_UNKNOWN_CATALOG", "a plain entry is not a connector");
    assert_eq!(mcp::connect(&root, "cn5a", "github", BTreeMap::new(), &sandbox).unwrap_err().code, "CONNECTOR_BAD_FIELDS");
    let mut blank = github_token();
    blank.insert("GITHUB_PERSONAL_ACCESS_TOKEN".into(), "   ".into());
    assert_eq!(mcp::connect(&root, "cn5a", "github", blank, &sandbox).unwrap_err().code, "CONNECTOR_BAD_FIELDS");
    let mut two_lines = github_token();
    two_lines.insert("GITHUB_PERSONAL_ACCESS_TOKEN".into(), "a\nb".into());
    assert_eq!(mcp::connect(&root, "cn5a", "github", two_lines, &sandbox).unwrap_err().code, "CONNECTOR_BAD_FIELDS");
    let mut extra = github_token();
    extra.insert("OTHER".into(), "x".into());
    assert_eq!(mcp::connect(&root, "cn5a", "github", extra, &sandbox).unwrap_err().code, "CONNECTOR_BAD_FIELDS");

    mcp::connect(&root, "cn5a", "github", github_token(), &sandbox).unwrap();
    assert_eq!(mcp::connect(&root, "cn5a", "github", github_token(), &sandbox).unwrap_err().code, "CONNECTOR_EXISTS");
    // A second bot cannot take the same connector while the first has it on: the guest runs one server per name.
    assert_eq!(mcp::connect(&root, "cn5b", "github", github_token(), &sandbox).unwrap_err().code, "CONNECTOR_IN_USE");
    mcp::set_enabled(&root, "cn5a", "github", false, &sandbox).unwrap();
    mcp::connect(&root, "cn5b", "github", github_token(), &sandbox).unwrap();
    assert_eq!(mcp::set_enabled(&root, "cn5a", "github", true, &sandbox).unwrap_err().code, "CONNECTOR_IN_USE");
    // The hub holds one github, with bot-b's token only.
    let held = sandbox.last_secret_config().unwrap();
    assert_eq!(held.as_array().unwrap().len(), 1);
    assert_eq!(mcp::guest_config(&root).unwrap().as_array().unwrap().len(), 0, "and the control service none");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn always_allow_reads_is_per_connector_and_only_for_connectors() {
    let root = temp("reads");
    let sandbox = running();
    mcp::connect(&root, "cn6a", "github", github_token(), &sandbox).unwrap();
    mcp::add_catalog(&root, "cn6a", "memory", &sandbox).unwrap();
    mcp::set_always_allow_reads(&root, "cn6a", "github", true).unwrap();
    assert!(bot_settings::load(&root, "cn6a").unwrap().mcp_servers[0].always_allow_reads);
    assert_eq!(mcp::set_always_allow_reads(&root, "cn6a", "memory", true).unwrap_err().code, "MCP_NOT_FOUND");
    mcp::set_always_allow_reads(&root, "cn6a", "github", false).unwrap();
    assert!(!bot_settings::load(&root, "cn6a").unwrap().mcp_servers[0].always_allow_reads);
    let _ = std::fs::remove_dir_all(&root);
}

// ---- nothing carries the token out --------------------------------------------------------------------

#[test]
fn a_token_is_scrubbed_from_text_notes_and_answers() {
    let values = BTreeMap::from([("K".to_string(), TOKEN.to_string())]);
    assert_eq!(mcp::scrub(&format!("Bearer {TOKEN} and {TOKEN}"), &values), "Bearer [hidden] and [hidden]");
    assert_eq!(mcp::scrub("no secret here", &values), "no secret here");

    let root = temp("scrub");
    let sandbox = running();
    mcp::connect(&root, "cn7a", "github", github_token(), &sandbox).unwrap();
    let settings = bot_settings::load(&root, "cn7a").unwrap();
    let mut answer = json!({ "ok": true, "text": format!("env: GITHUB_PERSONAL_ACCESS_TOKEN={TOKEN}"), "note": format!("see {TOKEN}") });
    mcp::scrub_answer("cn7a", &settings, "github", &mut answer);
    assert!(!answer.to_string().contains(TOKEN));
    assert!(answer["text"].as_str().unwrap().contains("[hidden]"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_request_connector_card_names_a_listed_service_by_its_title() {
    assert_eq!(mcp::connector_label("github"), ("GitHub".to_string(), true));
    assert_eq!(mcp::connector_label(" Notion "), ("Notion".to_string(), true));
    let (label, listed) = mcp::connector_label("Gmail\n\u{7}");
    assert_eq!((label.as_str(), listed), ("Gmail", false));
    assert_eq!(mcp::connector_label(&"x".repeat(100)).0.len(), 40);
}

#[test]
fn a_server_with_keys_goes_to_the_secret_hub_a_plain_one_to_the_control_service_and_a_removed_one_leaves() {
    let root = temp("secretfiles");
    let sandbox = running();
    let mut env = BTreeMap::new();
    env.insert("K".to_string(), TOKEN.to_string());
    mcp::add_server(&root, "cn9a", mcp::NewServer { name: "mine".into(), command: "node".into(), args: vec![], env }, &sandbox).unwrap();
    mcp::add_catalog(&root, "cn9a", "memory", &sandbox).unwrap();
    let plain = sandbox.last_mcp_config().unwrap();
    let names = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(&plain), vec!["memory".to_string()]);
    assert_eq!(names(&sandbox.last_secret_config().unwrap()), vec!["mine".to_string()]);
    mcp::remove_server(&root, "cn9a", "mine", &sandbox).unwrap();
    assert!(sandbox.last_secret_config().unwrap().as_array().unwrap().is_empty(), "the hub is told to drop it");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn encoded_echoes_of_a_token_are_scrubbed_too() {
    use base64::Engine;
    let values = BTreeMap::from([("K".to_string(), TOKEN.to_string())]);
    let b64 = base64::engine::general_purpose::STANDARD.encode(TOKEN);
    let b64_url = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TOKEN);
    for echo in [b64.as_str(), b64_url.as_str(), TOKEN] {
        let out = mcp::scrub(&format!("header: {echo} end"), &values);
        assert!(!out.contains(echo), "{echo} -> {out}");
    }
    let odd = BTreeMap::from([("K".to_string(), "ab/c+d=e 1234".to_string())]);
    assert_eq!(mcp::scrub("q=ab%2Fc%2Bd%3De%201234&x", &odd), "q=[hidden]&x", "a percent-encoded echo");
    assert_eq!(mcp::scrub(&format!("a {b64} b"), &values), "a [hidden] b");
}
