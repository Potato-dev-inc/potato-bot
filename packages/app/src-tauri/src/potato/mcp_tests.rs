//! Tool servers for the bot's computer (`mcp.rs`): names, catalog, the answers, the settings and the guest's list, and the
//! two brain tools' argument checks in `sandbox_calls`. The sandbox here is `FakeSandbox::new()`, which is stopped, so
//! nothing reaches a guest.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::bot_settings::{self, McpServer};
use super::mcp::{self, NewServer};
use super::sandbox::FakeSandbox;
use super::sandbox_calls::{validate, Options, Request, Tool};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn server(name: &str) -> NewServer {
    NewServer { name: name.to_string(), command: "npx".to_string(), args: vec!["-y".to_string(), "pkg".to_string()], env: BTreeMap::new() }
}

#[test]
fn server_names_are_one_to_thirty_two_lowercase_letters_digits_dash_or_underscore() {
    assert!(mcp::valid_server_name("filesystem"));
    assert!(mcp::valid_server_name("my_tool-2"));
    assert!(mcp::valid_server_name(&"a".repeat(32)));
    assert!(!mcp::valid_server_name(""));
    assert!(!mcp::valid_server_name(&"a".repeat(33)));
    assert!(!mcp::valid_server_name("Has Caps"));
    assert!(!mcp::valid_server_name("with space"));
    assert!(!mcp::valid_server_name("dot.name"));
}

#[test]
fn env_names_start_with_a_letter_or_underscore() {
    assert!(mcp::valid_env_key("GITHUB_TOKEN"));
    assert!(mcp::valid_env_key("_x1"));
    assert!(!mcp::valid_env_key("1BAD"));
    assert!(!mcp::valid_env_key("BAD-NAME"));
    assert!(!mcp::valid_env_key(""));
}

#[test]
fn the_catalog_has_the_entries_with_valid_names() {
    let names: Vec<&str> = mcp::catalog().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["filesystem", "memory", "playwright", "github", "notion"]);
    assert!(mcp::catalog().iter().all(|e| mcp::valid_server_name(&e.name)));
    // The playwright entry is pinned in the recipe, never @latest.
    let playwright = mcp::catalog().iter().find(|e| e.name == "playwright").unwrap();
    assert_eq!(playwright.command, "playwright-mcp");
    assert!(!std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../potato-sandbox/recipes/mcp-catalog.json")).unwrap().contains("@latest"));
}

#[test]
fn an_answer_keeps_the_text_and_lifts_the_first_picture_like_a_screenshot() {
    let reply = json!({
        "isError": false,
        "content": [
            { "type": "text", "text": "one" },
            { "type": "image", "data": "AAAA", "mimeType": "image/png" },
            { "type": "image", "data": "BBBB", "mimeType": "image/png" },
            { "type": "text", "text": "two" }
        ]
    });
    let out = mcp::answer(&reply);
    assert_eq!(out["ok"], json!(true));
    assert_eq!(out["text"], json!("one\ntwo"));
    assert_eq!(out["image"], json!({ "data": "AAAA", "mime": "image/png" }));
    assert_eq!(out["note"], json!("1 picture(s) the tool sent were not shown."));
}

#[test]
fn an_error_answer_is_not_ok_and_an_odd_picture_type_is_not_passed_on() {
    let reply = json!({ "isError": true, "content": [ { "type": "image", "data": "AAAA", "mimeType": "image/gif" } ] });
    let out = mcp::answer(&reply);
    assert_eq!(out["ok"], json!(false));
    assert!(out.get("image").is_none());
    assert_eq!(out["note"], json!("1 picture(s) the tool sent were not shown."));
}

#[test]
fn a_new_server_is_checked_before_anything_is_saved() {
    let none: Vec<McpServer> = vec![];
    assert!(mcp::validate_new(&server("ok"), &none).is_ok());
    assert!(mcp::validate_new(&server("Bad Name"), &none).is_err());
    assert!(mcp::validate_new(&server("computer"), &none).is_err(), "the built-in name is taken");
    let mut bad_env = server("ok");
    bad_env.env.insert("1BAD".to_string(), "x".to_string());
    assert!(mcp::validate_new(&bad_env, &none).is_err());
    let mut blank = server("ok");
    blank.command = "   ".to_string();
    assert!(mcp::validate_new(&blank, &none).is_err());
    let mut many = server("ok");
    many.args = vec!["a".to_string(); 33];
    assert!(mcp::validate_new(&many, &none).is_err());
    let taken = vec![McpServer { name: "ok".into(), command: "npx".into(), args: vec![], env_keys: vec![], enabled: true, ..Default::default() }];
    assert!(mcp::validate_new(&server("ok"), &taken).is_err());
}

#[test]
fn the_brain_tool_arguments_are_checked_and_the_list_is_read_only() {
    let opts = Options::default();
    let call = validate(Tool::McpCallTool, &json!({ "server": "memory", "tool": "create_entities", "arguments": { "x": 1 } }), None, &opts);
    match call {
        Ok(Request::McpCall { server, tool, arguments }) => {
            assert_eq!(server, "memory");
            assert_eq!(tool, "create_entities");
            assert_eq!(arguments, json!({ "x": 1 }));
        }
        other => panic!("expected an MCP call, got {other:?}"),
    }
    assert!(validate(Tool::McpCallTool, &json!({ "tool": "x" }), None, &opts).is_err(), "a server is required");
    assert!(validate(Tool::McpCallTool, &json!({ "server": "memory" }), None, &opts).is_err(), "a tool is required");
    assert!(validate(Tool::McpCallTool, &json!({ "server": "memory", "tool": "x", "arguments": [1] }), None, &opts).is_err());
    let list = validate(Tool::McpListTools, &json!({}), None, &opts).unwrap();
    assert_eq!(list, Request::McpList);
    assert!(list.is_read_only());
    assert_eq!(list.cost(), 0);
    let call = validate(Tool::McpCallTool, &json!({ "server": "memory", "tool": "x" }), None, &opts).unwrap();
    assert!(!call.is_read_only(), "a tool call is never repeated");
    assert_eq!(call.cost(), 1);
}

#[test]
fn adding_and_removing_a_server_keeps_only_the_env_names_in_the_settings() {
    let root = temp("add");
    let sandbox = FakeSandbox::new();
    let mut with_env = server("gh");
    with_env.env.insert("GITHUB_TOKEN".to_string(), "secret-value-123".to_string());
    mcp::add_server(&root, "helper-bot", with_env, &sandbox).unwrap();

    let settings = bot_settings::load(&root, "helper-bot").unwrap();
    assert_eq!(settings.mcp_servers.len(), 1);
    assert_eq!(settings.mcp_servers[0].env_keys, vec!["GITHUB_TOKEN".to_string()]);
    assert!(settings.mcp_servers[0].enabled);
    let on_disk = std::fs::read_to_string(bot_settings::path(&root, "helper-bot").unwrap()).unwrap();
    assert!(!on_disk.contains("secret-value-123"), "the value is never in the settings file");

    let view = mcp::overview(&root, "helper-bot", &sandbox).unwrap();
    assert_eq!(view.servers[0].env_keys, vec!["GITHUB_TOKEN".to_string()]);
    assert_eq!(view.servers[0].status, "unknown", "a stopped computer is not asked");
    assert!(!view.builtin.ready);
    assert_eq!(view.catalog.iter().filter(|c| c.installed).count(), 0);
    assert!(!serde_json::to_string(&view).unwrap().contains("secret-value-123"));

    mcp::remove_server(&root, "helper-bot", "gh", &sandbox).unwrap();
    assert!(bot_settings::load(&root, "helper-bot").unwrap().mcp_servers.is_empty());
    assert!(mcp::remove_server(&root, "helper-bot", "gh", &sandbox).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_guest_list_has_enabled_servers_of_every_bot_once_and_by_name() {
    let root = temp("guest");
    let sandbox = FakeSandbox::new();
    mcp::add_catalog(&root, "bot-a", "memory", &sandbox).unwrap();
    mcp::add_catalog(&root, "bot-a", "filesystem", &sandbox).unwrap();
    mcp::add_catalog(&root, "bot-b", "memory", &sandbox).unwrap();
    mcp::set_enabled(&root, "bot-a", "filesystem", false, &sandbox).unwrap();

    let list = mcp::guest_config(&root).unwrap();
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["memory"], "a disabled server is left out and a name is sent once");
    assert_eq!(list[0]["env"], json!({}));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn testing_a_server_with_the_computer_stopped_starts_nothing() {
    let root = temp("test");
    let sandbox = FakeSandbox::new();
    mcp::add_catalog(&root, "bot-a", "memory", &sandbox).unwrap();
    let result = mcp::test_server(&root, "bot-a", "memory", &sandbox).unwrap();
    assert!(!result.ok);
    assert!(result.tools.is_empty());
    assert!(result.error.is_some());
    let _ = std::fs::remove_dir_all(&root);
}
