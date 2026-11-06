//! The host's client of the secret hub on a scripted pipe: the line protocol, ids, a closed pipe, a timeout, a late answer, a
//! hub error, and that a value is sent only in `config` and the pipe is stopped when the client goes.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::secret_hub::SecretHub;
use super::wsl::{Pipe, WslError};

const TOKEN: &str = "fake-token-1d77e0a9c3";

#[derive(Default)]
struct Script {
    sent: Vec<String>,
    /// What `recv` gives next: a line, `None` for a closed pipe, or a timeout.
    replies: VecDeque<Result<Option<String>, WslError>>,
    stopped: u32,
    alive: bool,
}

struct ScriptedPipe(Arc<Mutex<Script>>);

impl Pipe for ScriptedPipe {
    fn send(&mut self, line: &str) -> Result<(), WslError> {
        let mut s = self.0.lock();
        if !s.alive {
            return Err(WslError::Start("closed".to_string()));
        }
        s.sent.push(line.to_string());
        Ok(())
    }
    fn recv(&mut self, _timeout: Duration) -> Result<Option<String>, WslError> {
        self.0.lock().replies.pop_front().unwrap_or(Err(WslError::Timeout))
    }
    fn alive(&mut self) -> bool {
        self.0.lock().alive
    }
    fn stop(&mut self) {
        self.0.lock().stopped += 1;
    }
}

fn hub(replies: Vec<Result<Option<String>, WslError>>) -> (SecretHub, Arc<Mutex<Script>>) {
    let script = Arc::new(Mutex::new(Script { replies: replies.into(), alive: true, ..Script::default() }));
    (SecretHub::new(Box::new(ScriptedPipe(script.clone()))), script)
}

fn line(v: Value) -> Result<Option<String>, WslError> {
    Ok(Some(v.to_string()))
}

#[test]
fn a_value_is_sent_in_config_only_and_each_request_has_its_own_id() {
    let (mut h, script) = hub(vec![
        line(json!({ "id": 1, "ok": true, "servers": [] })),
        line(json!({ "id": 2, "ok": true, "servers": [{ "name": "github", "status": "ready", "error": "", "tools": [{ "name": "get_issue" }] }] })),
        line(json!({ "id": 3, "ok": true, "content": [{ "type": "text", "text": "hi" }], "isError": false })),
    ]);
    h.config(&json!([{ "name": "github", "command": "/opt/x", "args": [], "env": { "K": TOKEN } }])).unwrap();
    let tools = h.tools().unwrap();
    assert_eq!(tools["servers"][0]["name"], json!("github"));
    let answer = h.call("github", "get_issue", &json!({ "n": 1 })).unwrap();
    assert_eq!(answer, json!({ "content": [{ "type": "text", "text": "hi" }], "isError": false }));
    let sent = script.lock().sent.clone();
    assert_eq!(sent.len(), 3);
    assert!(sent[0].contains(TOKEN) && sent[0].contains("\"op\":\"config\""));
    assert!(!sent[1].contains(TOKEN) && !sent[2].contains(TOKEN), "a tools or call request never holds a value");
    assert!(sent[2].contains("\"id\":3"));
}

#[test]
fn a_closed_pipe_a_dead_process_and_a_timeout_are_plain_errors() {
    let (mut h, _) = hub(vec![Ok(None)]);
    assert_eq!(h.tools().unwrap_err().code, "SANDBOX_MCP_HUB_CLOSED");
    let (mut h, script) = hub(vec![]);
    script.lock().alive = false;
    assert_eq!(h.tools().unwrap_err().code, "SANDBOX_MCP_HUB_CLOSED", "sending to a dead process");
    assert!(!h.alive());
    let (mut h, _) = hub(vec![Err(WslError::Timeout)]);
    assert_eq!(h.call("github", "x", &json!({})).unwrap_err().code, "SANDBOX_MCP_TIMEOUT");
}

#[test]
fn a_late_answer_to_an_earlier_request_is_skipped_and_not_mistaken_for_this_one() {
    let (mut h, _) = hub(vec![Err(WslError::Timeout), line(json!({ "id": 1, "ok": true, "content": [{ "type": "text", "text": "late" }] })), line(json!({ "id": 2, "ok": true, "content": [{ "type": "text", "text": "right" }], "isError": false }))]);
    assert!(h.call("github", "slow", &json!({})).is_err());
    let second = h.call("github", "fast", &json!({})).unwrap();
    assert_eq!(second["content"][0]["text"], json!("right"));
}

#[test]
fn a_hub_error_is_a_cut_plain_sentence_and_junk_lines_are_ignored() {
    let long = "x".repeat(900);
    let (mut h, _) = hub(vec![Ok(Some("not json".to_string())), line(json!({ "id": 1, "ok": false, "status": 400, "error": long }))]);
    let error = h.call("github", "x", &json!({})).unwrap_err();
    assert_eq!(error.code, "SANDBOX_MCP_REFUSED");
    assert_eq!(error.message.chars().count(), 300);
}

#[test]
fn dropping_the_client_stops_the_pipe_so_the_hub_stops_its_servers() {
    let (h, script) = hub(vec![]);
    drop(h);
    assert_eq!(script.lock().stopped, 1);
}
