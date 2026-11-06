use serde_json::json;

use super::approvals::{
    explicit_manual_approval_required, native_status_unavailable, startup_manual_approval_required,
};
use super::errors::operation_http_error;
use super::http::query_param;
use super::locks::parse_operation_lock;
use super::tool_names::bridge_tool_name;

// Helpers the removed production code used to own; only these tests still need them.

fn pane_buffer_view(requested: Option<&str>, agent_type: &str) -> Result<&'static str, &'static str> {
    let default = if crate::pty::status::is_shell_agent(agent_type) { "scrollback" } else { "screen" };
    match requested {
        None => Ok(default),
        Some("screen") => Ok("screen"),
        Some("scrollback") => Ok("scrollback"),
        Some(_) => Err("view must be screen or scrollback"),
    }
}

fn register_spawned_pane(session: Option<&str>, agent_type: &str, pane_id: &str) -> Result<(), crate::operations::OperationError> {
    let Some(session) = session else {
        return Ok(());
    };
    if crate::pty::status::is_shell_agent(agent_type) {
        crate::mcp_sessions::register_shell_pane(session, pane_id)
    } else {
        crate::mcp_sessions::register_owned_pane(session, pane_id)
    }
}

fn parse_json<T: for<'de> serde::Deserialize<'de>>(body: &[u8]) -> Result<T, (u16, serde_json::Value)> {
    serde_json::from_slice(body).map_err(|err| operation_http_error(crate::operations::OperationError::invalid_body(&err)))
}

fn requested_project_path(body_path: Option<&str>, query: &str) -> Option<String> {
    body_path
        .filter(|path| !path.trim().is_empty())
        .map(str::to_string)
        .or_else(|| query_param(query, "project_path"))
}

#[test]
fn a_trust_dialog_on_screen_blocks_startup_dispatch_whatever_the_status_says() {
    // Real captures of a visible cursor_agent pane. Right after the dialog is drawn the pane
    // status still reads `running`; typing the task then hits the dialog's hotkeys.
    let pending = include_str!("../tui/cursor/fixtures/workspace_trust_visible_pending.txt");
    let resolved = include_str!("../tui/cursor/fixtures/workspace_trust_visible_resolved.txt");
    assert!(startup_manual_approval_required(
        "cursor_agent",
        Some("running"),
        pending,
        &[]
    ));
    assert!(!startup_manual_approval_required(
        "cursor_agent",
        Some("running"),
        resolved,
        &[]
    ));
    assert!(!startup_manual_approval_required(
        "cursor_agent",
        Some("idle"),
        "Cursor Agent
→ Plan, search, build anything
",
        &[]
    ));
}

#[test]
fn pane_buffer_default_follows_the_pane_type_and_explicit_views_override_it() {
    assert_eq!(pane_buffer_view(None, "codex"), Ok("screen"));
    assert_eq!(pane_buffer_view(None, "bash"), Ok("scrollback"));
    assert_eq!(
        pane_buffer_view(Some("scrollback"), "codex"),
        Ok("scrollback")
    );
    assert_eq!(pane_buffer_view(Some("screen"), "powershell"), Ok("screen"));
    assert!(pane_buffer_view(Some("raw"), "codex").is_err());
}

#[test]
fn spawned_shells_register_as_shell_panes_for_the_request_session() {
    let session = uuid::Uuid::new_v4().to_string();
    let pane_id = uuid::Uuid::new_v4().to_string();
    register_spawned_pane(Some(&session), "powershell", &pane_id).unwrap();
    crate::mcp_sessions::set_mode(&session, "shell").unwrap();
    assert!(crate::mcp_sessions::authorize(
        Some(&session),
        "POST",
        &format!("/panes/{pane_id}/input"),
        &json!({"text":"Get-Location"})
    )
    .is_ok());
}

#[test]
fn spawned_agents_register_as_agent_panes_for_the_request_session() {
    let session = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let pane_id = uuid::Uuid::new_v4().to_string();
    register_spawned_pane(Some(&session), "codex", &pane_id).unwrap();
    crate::mcp_sessions::set_mode(&session, "shell").unwrap();
    crate::mcp_sessions::set_mode(&other, "shell").unwrap();
    let write = |who: &str| {
        crate::mcp_sessions::authorize(
            Some(who),
            "POST",
            &format!("/panes/{pane_id}/input"),
            &json!({"text":"y"}),
        )
    };
    // The pane the connection's own run opened is its own: typing needs no take_over...
    assert!(write(&session).is_ok());
    // ...a shell command into it still does, and another connection is refused.
    assert!(crate::mcp_sessions::authorize(
        Some(&session),
        "POST",
        "/shell/exec",
        &json!({"command":"dir","pane_id":pane_id})
    )
    .is_err());
    assert!(write(&other).is_err());
}

#[test]
fn bridge_tool_names_include_session_context_routes() {
    assert_eq!(
        bridge_tool_name("GET", &["session", "context"]).as_deref(),
        Some("read_session_context")
    );
    assert_eq!(
        bridge_tool_name("PATCH", &["session", "context"]).as_deref(),
        Some("update_session_context")
    );
    assert_eq!(
        bridge_tool_name("POST", &["panes", "pane-1", "role"]).as_deref(),
        Some("set_pane_role")
    );
    assert_eq!(
        bridge_tool_name("POST", &["delegate-task"]).as_deref(),
        Some("delegate_task")
    );
    assert_eq!(
        bridge_tool_name("POST", &["operations", "delegate"]).as_deref(),
        Some("delegate_work")
    );
    assert_eq!(
        bridge_tool_name("GET", &["operations", "op-1"]).as_deref(),
        Some("get_operation")
    );
    assert_eq!(
        bridge_tool_name("POST", &["operations", "op-1", "wait"]).as_deref(),
        Some("wait_for_operation")
    );
    assert_eq!(
        bridge_tool_name("POST", &["operations", "op-1", "cancel"]).as_deref(),
        Some("cancel_operation")
    );
}

#[test]
fn operation_lock_contract_requires_typed_nonempty_resources() {
    assert_eq!(
        parse_operation_lock("file:src/main.rs").unwrap(),
        ("file".into(), "src/main.rs".into())
    );
    assert_eq!(
        parse_operation_lock(" :name ").unwrap_err().code,
        "INVALID_LOCK"
    );
    assert_eq!(
        parse_operation_lock("missing-type").unwrap_err().code,
        "INVALID_LOCK"
    );
}

#[test]
fn readiness_distinguishes_normal_agent_prompt_from_real_permission() {
    assert!(!explicit_manual_approval_required(
        "codex",
        "How can I help? >",
        &[]
    ));
    assert!(explicit_manual_approval_required(
        "codex",
        "Allow this command to run? (y/n)",
        &[],
    ));
    assert!(explicit_manual_approval_required(
        "opencode_native",
        "Waiting for input",
        &["permission-123".into()],
    ));
}

#[test]
fn native_pending_permission_blocks_startup_even_when_pane_looks_idle() {
    assert!(startup_manual_approval_required(
        "opencode_native",
        Some("idle"),
        "",
        &["permission-123".into()],
    ));
    assert!(!startup_manual_approval_required(
        "codex",
        Some("idle"),
        "Allow this command?",
        &[],
    ));
    assert!(startup_manual_approval_required(
        "codex",
        Some("waiting_input"),
        "Allow this command?",
        &[],
    ));
}

#[test]
fn operation_errors_use_the_structured_retry_contract() {
    let mut error = crate::operations::OperationError::new("RESOURCE_LOCKED", "busy", true);
    error.retry_after_ms = Some(750);
    error.context = json!({"resource_id":"file:src/main.rs"});
    let (status, body) = operation_http_error(error);
    assert_eq!(status, 409);
    assert_eq!(body["code"], "RESOURCE_LOCKED");
    assert_eq!(body["message"], "busy");
    assert_eq!(body["error"], "busy");
    assert_eq!(body["recoverable"], true);
    assert_eq!(body["retry_after_ms"], 750);
    assert_eq!(body["context"]["resource_id"], "file:src/main.rs");
    for (code, expected_status) in [
        ("IDEMPOTENCY_KEY_CONFLICT", 409),
        ("PANE_UNAVAILABLE", 409),
        ("INVALID_AGENT_TYPE", 400),
        ("UNKNOWN_AGENT_TYPE", 400),
        ("AGENT_DEF_APPROVAL_STALE", 409),
        ("AGENT_DEF_BINARY_CHANGED", 409),
        ("AGENT_DEF_UNAPPROVED", 409),
        ("AGENT_NOT_INSTALLED", 400),
    ] {
        let (status, body) = operation_http_error(crate::operations::OperationError::new(
            code, "invalid", false,
        ));
        assert_eq!(status, expected_status, "wrong mapping for {code}");
        assert_eq!(body["code"], code);
    }
}

/// R8 verify finding: a body the caller got wrong answered 503 `INVALID_JSON`. Every code that
/// `OperationError::invalid_body` can produce is a client error, and the bridge's own
/// `parse_json` (used by the plain routes) now speaks the same typed codes.
#[test]
fn a_malformed_request_body_is_a_typed_4xx_never_a_503() {
    for (code, field) in [
        ("INVALID_JSON", None),
        ("MISSING_FIELD", Some("prompt_id")),
        ("INVALID_FIELD", None),
    ] {
        let mut error = crate::operations::OperationError::new(code, "bad body", false);
        if let Some(field) = field {
            error.context = json!({ "field": field });
        }
        let (status, body) = operation_http_error(error);
        assert_eq!(status, 400, "{code} must be a 4xx");
        assert_eq!(body["code"], code);
    }
    #[derive(serde::Deserialize, Debug)]
    struct Probe {
        #[allow(dead_code)]
        name: String,
    }
    let (status, body) = parse_json::<Probe>(b"{}").expect_err("name is required");
    assert_eq!((status, body["code"].as_str()), (400, Some("MISSING_FIELD")));
    assert_eq!(body["context"]["field"], "name");
    let (status, body) = parse_json::<Probe>(b"{").expect_err("truncated");
    assert_eq!((status, body["code"].as_str()), (400, Some("INVALID_JSON")));
}

#[test]
fn native_permission_status_failure_is_recoverable_and_fails_closed() {
    let error = native_status_unavailable("worker API is unreachable");
    assert_eq!(error.code, "NATIVE_STATUS_UNAVAILABLE");
    assert!(error.recoverable);
    let (status, body) = operation_http_error(error);
    assert_eq!(status, 503);
    assert_eq!(body["code"], "NATIVE_STATUS_UNAVAILABLE");
    assert_eq!(body["recoverable"], true);
}

#[test]
fn query_parameters_decode_url_encoded_paths_and_reject_malformed_values() {
    assert_eq!(
        query_param(
            "project_path=C%3A%2FUsers%2FDev%20Folder%2Fapp",
            "project_path"
        ),
        Some("C:/Users/Dev Folder/app".into())
    );
    assert_eq!(query_param("project_path=%ZZ", "project_path"), None);
    assert_eq!(query_param("other=value", "project_path"), None);
}

#[test]
fn wait_project_path_prefers_body_then_query() {
    assert_eq!(
        requested_project_path(Some("C:/body"), "project_path=C%3A%2Fquery"),
        Some("C:/body".into())
    );
    assert_eq!(
        requested_project_path(None, "project_path=C%3A%2Fquery"),
        Some("C:/query".into())
    );
    assert_eq!(requested_project_path(Some("  "), ""), None);
}

fn assert_unknown_agent_type_response(status: u16, body: &serde_json::Value) {
    assert_eq!(status, 400);
    assert_eq!(body["code"], "UNKNOWN_AGENT_TYPE");
    assert_eq!(body["recoverable"], false);
    assert_eq!(body["context"]["hint"], "GET /agents/types");
    let valid: Vec<String> = body["context"]["valid"]
        .as_array()
        .expect("valid ids")
        .iter()
        .map(|value| value.as_str().expect("id").to_string())
        .collect();
    assert_eq!(valid.len(), 9);
    assert_eq!(valid, crate::tui::registered_agent_ids());
}

#[test]
fn run_agent_unknown_type_returns_unknown_agent_type() {
    let _serial = crate::tui::registry::test_serial();
    let snapshot = crate::operations::OperationSnapshot {
        operation_id: uuid::Uuid::new_v4().to_string(),
        runtime_id: String::new(),
        request_fingerprint: String::new(),
        project_path: "C:/proj".into(),
        task: "work".into(),
        agent_type: "not-a-real-agent".into(),
        pane_id: None,
        pane_created: false,
        idempotency_key: "key".into(),
        acceptance_criteria: None,
        task_id: None,
        exclusive: false,
        locks: vec![],
        status: crate::operations::OperationStatus::Queued,
        revision: 1,
        created_at_ms: 0,
        updated_at_ms: 0,
        observed_at_ms: None,
        pane_state: None,
        required_action: None,
        source: crate::operations::StateSource::Native,
        stage: None,
        progress_pct: None,
        result: None,
        error: None,
        timeout_ms: 60_000,
        read_only: false,
        keep_pane: false,
        placement: None,
        keep_pane_requested: None,
        owner_session_id: None,
        parent_pane_id: None,
        worker_has_mcp_tools: false,
        agent_run_id: "run".into(),
        turn_index: 0,
        verified: false,
        started_at_ms: None,
        finished_at_ms: None,
        message_baseline_ids: vec![],
        output_baseline: None,
        result_capture: None,
        acceptance_status: crate::operations::AcceptanceStatus::NotChecked,
        worker: Default::default(),
        checks: Vec::new(),
        check_results: Vec::new(),
        warnings: Vec::new(),
    };
    let error =
        super::dispatch::check_agent_dispatchable(&snapshot, "not-a-real-agent").unwrap_err();
    let (status, body) = operation_http_error(error);
    assert_unknown_agent_type_response(status, &body);
}

#[test]
fn spawn_unknown_type_returns_unknown_agent_type() {
    let _serial = crate::tui::registry::test_serial();
    let unknown = match crate::tui::require_registered("bogus-agent") {
        Err(err) => err,
        Ok(_) => panic!("expected unknown agent type"),
    };
    let (status, body) = operation_http_error(
        crate::operations::OperationError::unknown_agent_type(unknown.id, unknown.valid),
    );
    assert_unknown_agent_type_response(status, &body);
}

#[test]
fn delegate_work_unknown_type_keeps_invalid_agent_type() {
    let dir = std::env::temp_dir().join(format!("pm-delegate-unknown-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let req = crate::operations::DelegateWorkRequest {
        project_path: dir.to_string_lossy().into_owned(),
        task: "do work".into(),
        agent_type: "not-a-real-agent".into(),
        pane_id: None,
        idempotency_key: "delegate-unknown".into(),
        acceptance_criteria: None,
        task_id: None,
        exclusive: false,
        locks: vec![],
        timeout_ms: 900_000,
        read_only: false,
        keep_pane: false,
        placement: None,
        owner_session_id: None,
        parent_pane_id: None,
        worker_has_mcp_tools: false,
        agent_run_id: None,
        turn_index: 0,
        worker: Default::default(),
        context_policy: None,
        checks: Vec::new(),
    };
    let error = crate::operations::create_operation(req).unwrap_err();
    assert_eq!(error.code, "INVALID_AGENT_TYPE");
    assert_eq!(error.message, "agent_type is not supported");
    let (status, body) = operation_http_error(error);
    assert_eq!(status, 400);
    assert_eq!(body["code"], "INVALID_AGENT_TYPE");
    assert_eq!(body["message"], "agent_type is not supported");
    let _ = std::fs::remove_dir_all(dir);
}

// ---- /agents/definitions (T19) -------------------------------------------------------------

mod definitions {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{json, Value};

    use crate::mcp_sessions;
    use crate::tui::declarative::loader::{reload_global, LoadInputs};
    use crate::tui::declarative::service;

    /// A unique temp tree; cleanup removes only recorded entries, one at a time.
    struct Tree {
        root: PathBuf,
        files: Vec<PathBuf>,
    }

    impl Tree {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir()
                .join(format!("pm-defroutes-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(root.join("agents")).unwrap();
            Self { root, files: Vec::new() }
        }
        fn agents(&self) -> PathBuf {
            self.root.join("agents")
        }
        fn approvals(&mut self) -> PathBuf {
            let path = self.root.join("approvals.json");
            self.files.push(path.clone());
            path
        }
        fn file(&mut self, name: &str, text: &str) -> PathBuf {
            let path = self.agents().join(name);
            std::fs::write(&path, text).unwrap();
            self.files.push(path.clone());
            path
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            for file in &self.files {
                let _ = std::fs::remove_file(file);
            }
            let _ = std::fs::remove_dir(self.root.join("agents"));
            let _ = std::fs::remove_dir(&self.root);
        }
    }

    /// `cargo` is on PATH while tests run, so the binary resolves without any agent installed.
    fn def(id: &str, extra: &str) -> String {
        format!(
            "schema_version = 1\n[agent]\nid = \"{id}\"\nlabel = \"Test {id}\"\n[binary]\nnames = [\"cargo\"]\ninstall_hint = \"get cargo\"\n[headless]\nargv = [\"run\"]\ntask_separator = \"--\"\n[headless.output]\nformat = \"plain_text\"\n{extra}"
        )
    }

    fn inputs<'a>(dir: &'a Path, code_ids: &'a [&'a str]) -> LoadInputs<'a> {
        LoadInputs { bundled: &[], user_dir: Some(dir), code_ids, projects: &[], extra: &[] }
    }

    #[test]
    fn agents_definitions_lists_state_and_errors() {
        let mut t = Tree::new();
        let good = t.file("a.toml", &def("defa_x", ""));
        t.file("b.toml", "schema_version = 1\n[agent]\nid = \"defb_x\"\nbogus = 1\n");
        let approvals = t.approvals();
        let code = ["claude"];
        let body = service::listing(&inputs(&t.agents(), &code), &approvals);
        assert_eq!(body["sources"][1]["name"], "user");
        assert_eq!(body["sources"][1]["enabled"], true);
        let defs = body["definitions"].as_array().unwrap();
        assert_eq!(defs.len(), 2);
        let a = defs.iter().find(|d| d["id"] == "defa_x").unwrap();
        assert_eq!(a["state"], "needs_approval");
        assert_eq!(a["source"], "user");
        assert!(a["fingerprint"].as_str().unwrap().starts_with("sha256:"));
        assert_eq!(a["file"], good.to_string_lossy().as_ref());
        let bad = defs.iter().find(|d| d["state"] == "invalid").unwrap();
        let errors = bad["errors"].as_array().unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0]["code"].is_string() && errors[0]["message"].is_string());
        assert!(bad["file"].as_str().unwrap().ends_with("b.toml"));
    }

    #[test]
    fn approve_stale_fingerprint_and_revoke() {
        let mut t = Tree::new();
        let file = t.file("a.toml", &def("defc_x", ""));
        let approvals = t.approvals();
        let code = ["claude"];
        let agents = t.agents();
        let inp = inputs(&agents, &code);
        let listed = service::listing(&inp, &approvals);
        let fp = listed["definitions"][0]["fingerprint"].as_str().unwrap().to_string();

        let stale = service::approve(&inp, &approvals, &file.to_string_lossy(), "sha256:nope").unwrap_err();
        assert_eq!((stale.status, stale.code), (409, "AGENT_DEF_APPROVAL_STALE"));
        let missing = service::approve(&inp, &approvals, "C:/no/such.toml", &fp).unwrap_err();
        assert_eq!(missing.code, "AGENT_DEF_NOT_FOUND");

        let ok = service::approve(&inp, &approvals, &file.to_string_lossy(), &fp).unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["id"], "defc_x");
        assert_eq!(ok["state"], "active");
        let record = crate::tui::declarative::approvals::Approvals::load(&approvals).records()[0].clone();
        assert!(!record.binary.is_empty(), "binary is pinned at approval");

        assert_eq!(service::listing(&inp, &approvals)["definitions"][0]["state"], "active");
        service::revoke(&approvals, "defc_x").unwrap();
        assert_eq!(service::listing(&inp, &approvals)["definitions"][0]["state"], "needs_approval");
        assert_eq!(service::revoke(&approvals, "defc_x").unwrap_err().code, "AGENT_DEF_NOT_FOUND");
    }

    #[test]
    fn validate_renders_argv_and_registers_nothing() {
        let good = service::validate_text(&def("defd_x", ""), "user", &["claude"]).unwrap();
        assert_eq!(good["ok"], true, "{good}");
        assert_eq!(good["rendered"]["headless_argv"], json!(["run", "--", "<task>"]));
        let clash = service::validate_text(&def("claude", ""), "user", &["claude"]).unwrap();
        assert_eq!(clash["ok"], false);
        assert!(!clash["errors"].as_array().unwrap().is_empty());
        assert_eq!(service::validate_text("x", "nowhere", &[]).unwrap_err().status, 400);
    }

}

#[test]
fn stop_all_route_is_not_exposed_to_mcp_sessions() {
    // Agents must not be able to stop their siblings through the bridge: the route is not a tool.
    let session = uuid::Uuid::new_v4().to_string();
    let err = crate::mcp_sessions::authorize(Some(&session), "POST", "/panes/stop-all", &json!({}))
        .expect_err("MCP session refused");
    assert_eq!(err.code, "AUTHORIZATION_DENIED");
    assert!(crate::mcp_sessions::authorize(None, "POST", "/panes/stop-all", &json!({})).is_ok());
}

#[test]
fn stop_report_wire_shape() {
    let report = crate::panic::StopReport {
        stopped: vec![crate::panic::StoppedPane { pane_id: "p1".into(), agent_type: "claude".into(), cwd: "/w".into(), checkpoint_id: None }],
        failed: vec![],
    };
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        json!({"stopped":[{"pane_id":"p1","agent_type":"claude","cwd":"/w"}],"failed":[]})
    );
    let with_checkpoint = crate::panic::StopReport {
        stopped: vec![crate::panic::StoppedPane { pane_id: "p1".into(), agent_type: "claude".into(), cwd: "/w".into(), checkpoint_id: Some("4".into()) }],
        failed: vec![],
    };
    assert_eq!(
        serde_json::to_value(with_checkpoint).unwrap(),
        json!({"stopped":[{"pane_id":"p1","agent_type":"claude","cwd":"/w","checkpoint_id":"4"}],"failed":[]})
    );
}

#[test]
fn worktree_refusals_map_to_http_statuses_with_their_context() {
    let mut conflict = crate::operations::OperationError::new("MERGE_CONFLICT", "2 files conflict", false);
    conflict.context = json!({"files": ["a.txt", "b.txt"]});
    let (status, body) = operation_http_error(conflict);
    assert_eq!(status, 409);
    assert_eq!(body["code"], "MERGE_CONFLICT");
    assert_eq!(body["context"]["files"], json!(["a.txt", "b.txt"]));
    for (code, expected) in [
        ("NOT_A_GIT_REPO", 400),
        ("NO_COMMITS", 400),
        ("WORKTREE_NOT_FOUND", 404),
        ("ORIGIN_DIRTY_OVERLAP", 409),
        ("RUN_ACTIVE", 409),
    ] {
        let (status, _) = operation_http_error(crate::operations::OperationError::new(code, "x", false));
        assert_eq!(status, expected, "{code}");
    }
}

mod pane_parent {
    use crate::bridge::http::request_session;
    use crate::bridge::pane_util::note_pane_parent;
    use crate::pty::registry::PaneRegistry;
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// A registry with a parent pane (whose process is this test process) and a fresh child pane.
    fn registry(parent_pid: u32) -> (Arc<Mutex<PaneRegistry>>, String, String) {
        let (parent, child) = (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string());
        let registry = Arc::new(Mutex::new(PaneRegistry::new()));
        {
            let mut reg = registry.lock();
            let mut stub = PaneRegistry::test_pane_stub(&parent);
            stub.info.pid = parent_pid;
            reg.panes.insert(parent.clone(), stub);
            reg.panes.insert(child.clone(), PaneRegistry::test_pane_stub(&child));
        }
        (registry, parent, child)
    }

    fn parent_of(registry: &Arc<Mutex<PaneRegistry>>, pane: &str) -> Option<String> {
        registry.lock().panes[pane].info().parent_pane_id
    }

    #[test]
    fn the_headers_the_shim_sends_reach_the_session_table() {
        let session = uuid::Uuid::new_v4().to_string();
        let headers = format!(
            "POST /agents/run HTTP/1.1\r\nX-Puppet-Master-Session: {session}\r\nX-Puppet-Master-Pid: 4321\r\nX-Puppet-Master-Pane: pane-xyz\r\n"
        );
        assert_eq!(request_session(&headers), Some(session.as_str()));
        assert_eq!(crate::mcp_sessions::claimed_session_pane(&session).as_deref(), Some("pane-xyz"));
        assert_eq!(crate::mcp_sessions::session_pid(&session), Some(4321));
    }

    #[test]
    fn a_run_started_from_a_pane_is_nested_under_it() {
        let (registry, parent, child) = registry(std::process::id());
        let session = uuid::Uuid::new_v4().to_string();
        crate::mcp_sessions::note_session_pane(&session, &parent);
        crate::mcp_sessions::note_session_pid(&session, std::process::id());
        note_pane_parent(&registry, Some(&session), None, &child);
        assert_eq!(parent_of(&registry, &child).as_deref(), Some(parent.as_str()));
        assert_eq!(parent_of(&registry, &parent), None, "the parent itself is untouched");
    }

    #[test]
    fn a_helper_started_by_plain_http_from_a_pane_is_nested_under_the_pane_the_os_placed_it_in() {
        // No MCP session, no header: the caller pane came from the connection's own process tree.
        let (registry, parent, child) = registry(1);
        note_pane_parent(&registry, None, Some(&parent), &child);
        assert_eq!(parent_of(&registry, &child).as_deref(), Some(parent.as_str()));
        assert_eq!(parent_of(&registry, &parent), None, "the parent itself is untouched");
    }

    #[test]
    fn the_caller_pane_wins_over_a_forged_hint_and_is_kept_for_the_panes_life() {
        let (registry, parent, child) = registry(1);
        let session = uuid::Uuid::new_v4().to_string();
        crate::mcp_sessions::note_session_pane(&session, "some-other-pane");
        note_pane_parent(&registry, Some(&session), Some(&parent), &child);
        assert_eq!(parent_of(&registry, &child).as_deref(), Some(parent.as_str()));
        // Set once: a later call (a second run on the pane) does not move it.
        note_pane_parent(&registry, None, Some("another"), &child);
        assert_eq!(parent_of(&registry, &child).as_deref(), Some(parent.as_str()));
    }

    #[test]
    fn a_caller_pane_that_is_gone_or_is_the_child_itself_earns_nothing() {
        let (registry, parent, child) = registry(1);
        *registry.lock().panes[&parent].exited.lock() = true;
        note_pane_parent(&registry, None, Some(&parent), &child);
        note_pane_parent(&registry, None, Some(&child), &child);
        note_pane_parent(&registry, None, Some("no-such-pane"), &child);
        assert_eq!(parent_of(&registry, &child), None);
    }

    #[test]
    fn a_helper_started_from_a_companions_pane_is_nested_under_the_companion() {
        // The companion's pane is `companion-<id>`; its helper gets it as `parent_pane_id` like any parent pane.
        let (registry, _parent, child) = registry(std::process::id());
        let companion = "companion-mochi".to_string();
        {
            let mut reg = registry.lock();
            let mut stub = PaneRegistry::test_pane_stub(&companion);
            stub.info.pid = std::process::id();
            reg.panes.insert(companion.clone(), stub);
        }
        let session = uuid::Uuid::new_v4().to_string();
        crate::mcp_sessions::note_session_pane(&session, &companion);
        crate::mcp_sessions::note_session_pid(&session, std::process::id());
        note_pane_parent(&registry, Some(&session), None, &child);
        assert_eq!(parent_of(&registry, &child).as_deref(), Some(companion.as_str()));
    }

    #[test]
    fn a_connection_that_runs_outside_the_named_pane_is_not_nested() {
        // The named pane's process is not an ancestor of the caller: someone else's pane id.
        let (registry, parent, child) = registry(u32::MAX - 7);
        let session = uuid::Uuid::new_v4().to_string();
        crate::mcp_sessions::note_session_pane(&session, &parent);
        crate::mcp_sessions::note_session_pid(&session, std::process::id());
        note_pane_parent(&registry, Some(&session), None, &child);
        if crate::pty::agent_proc::snapshot().is_some() {
            assert_eq!(parent_of(&registry, &child), None);
        }
    }

    #[test]
    fn a_connection_with_no_hint_or_no_session_is_not_nested() {
        let (registry, parent, child) = registry(std::process::id());
        let silent = uuid::Uuid::new_v4().to_string();
        crate::mcp_sessions::note_session_pid(&silent, std::process::id());
        note_pane_parent(&registry, Some(&silent), None, &child);
        note_pane_parent(&registry, None, None, &child);
        assert_eq!(parent_of(&registry, &child), None);
        // A hint for a pane that does not exist is dropped too.
        let ghost = uuid::Uuid::new_v4().to_string();
        crate::mcp_sessions::note_session_pane(&ghost, "no-such-pane");
        crate::mcp_sessions::note_session_pid(&ghost, std::process::id());
        note_pane_parent(&registry, Some(&ghost), None, &child);
        assert_eq!(parent_of(&registry, &child), None);
        let _ = parent;
    }
}

// ---- Restored after the split: the auth decisions whose production path (`authorize_caller`,
// `mcp_sessions::authorize`) is still live. The route handlers they once drove are gone.

#[test]
fn stop_all_route_is_authorised_like_other_remote_control_routes() {
    use crate::mobile_pairing::authorize_caller;
    let segs = ["panes", "stop-all"];
    // Remote peer without a pairing token: refused.
    let (status, body) = authorize_caller("", false, &segs, "POST").map(|_| ()).unwrap_err();
    assert_eq!((status, body["error"].as_str()), (401, Some("pairing_required")));
    // The phone PWA arrives through the loopback proxy: the proxy marker still demands a token.
    let (status, _) = authorize_caller("X-PM-Proxied: 1\r\n", true, &segs, "POST").map(|_| ()).unwrap_err();
    assert_eq!(status, 401);
    // A wrong token is refused too.
    let (status, _) = authorize_caller("Authorization: Bearer nope\r\n", false, &segs, "POST").map(|_| ()).unwrap_err();
    assert!(status == 401 || status == 503, "{status}");
    // The desktop app itself (loopback, no proxy marker) is allowed.
    assert!(authorize_caller("", true, &segs, "POST").map(|_| ()).is_ok());
}

#[test]
fn agent_definition_routes_are_app_session_only_for_mcp_sessions() {
    let body = json!({"file": "C:/x.toml", "fingerprint": "sha256:x", "removed_id": "zz-none"});
    // an MCP session (header present) can reach none of the definition routes
    for (method, path) in [
        ("POST", "/agents/definitions/approve"),
        ("POST", "/agents/definitions/revoke"),
        ("POST", "/agents/definitions/consent"),
        ("POST", "/agents/definitions/reload"),
        ("POST", "/agents/definitions/remove"),
        ("POST", "/agents/definitions/restore"),
        ("POST", "/agents/definitions/removed/purge"),
        ("POST", "/agents/definitions/removed/purge-all"),
        ("GET", "/agents/definitions/removed"),
        ("GET", "/agents/definitions"),
    ] {
        let denied = crate::mcp_sessions::authorize(Some("mcp-session-1"), method, path, &body).unwrap_err();
        assert_eq!(denied.code, "AUTHORIZATION_DENIED", "{method} {path}");
        // the app session (no MCP header) is let through the gate
        assert!(crate::mcp_sessions::authorize(None, method, path, &body).is_ok(), "{method} {path}");
    }
    // Policy: the only non-definition agent route open without a session is the read-only
    // `GET /agents/types` listing (the phone's "new agent" sheet). It stays GET-only,
    // and every other agent API still demands an MCP session.
    assert!(crate::mcp_sessions::authorize(None, "GET", "/agents/types", &json!({})).is_ok());
    for (method, path) in [
        ("POST", "/agents/types"),
        ("GET", "/agents"),
        ("GET", "/agents/types/extra"),
        ("POST", "/agents/take-over"),
        ("POST", "/agents/release"),
    ] {
        let err = crate::mcp_sessions::authorize(None, method, path, &json!({})).unwrap_err();
        assert_eq!(err.code, "SESSION_REQUIRED", "{method} {path}");
    }
}

#[test]
fn mcp_sessions_cannot_reach_the_toolbelt_through_the_session_gate() {
    let err = crate::mcp_sessions::authorize(Some("mcp-session-1"), "POST", "/toolbelt/sync/apply", &json!({}));
    assert!(err.is_err(), "route is not exposed to MCP sessions");
}
