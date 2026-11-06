//! The host's side of the secret hub: the root process in the computer (`pm-sbx-secret-hub`) that owns the tool servers
//! holding a token. It is started by the host with `wsl -u root` and spoken to over that process's stdin and stdout, one
//! JSON line each way. The bot (user pm) has no port, socket, file or command that reaches it, so it cannot call a
//! connector's tools behind the card gate: only this host can, after `sandbox_calls` has decided.
//!
//! The values travel in the `config` request, once per push, and are never in a reply. A closed pipe, a dead process or a
//! timeout is an error here; `WslSandbox` starts a new hub and replays the last config when the next call finds it dead.
//! Stopping the sandbox, or dropping this, closes the hub's stdin: it stops every server and exits.

use std::time::Duration;

use serde_json::{json, Value};

use super::wsl::{Pipe, WslError};
use super::{SandboxError, SbxResult};

/// The program, installed by the base recipe and by the guest refresh.
pub const HUB_PATH: &str = "/usr/local/sbin/pm-sbx-secret-hub";
/// A config can wait for servers that were told to start; the hub answers at once, then each server comes up on its own.
const CONFIG_WAIT: Duration = Duration::from_secs(20);
const TOOLS_WAIT: Duration = Duration::from_secs(20);
/// Longer than the hub's own call limit (60 s), so its answer arrives before ours gives up.
const CALL_WAIT: Duration = Duration::from_secs(75);

fn closed() -> SandboxError {
    SandboxError::new("SANDBOX_MCP_HUB_CLOSED", "The connectors' program in the computer stopped. It will start again with the next use.")
}

pub struct SecretHub {
    pipe: Box<dyn Pipe>,
    next_id: u64,
}

impl SecretHub {
    pub fn new(pipe: Box<dyn Pipe>) -> SecretHub {
        SecretHub { pipe, next_id: 0 }
    }

    pub fn alive(&mut self) -> bool {
        self.pipe.alive()
    }

    pub fn stop(&mut self) {
        self.pipe.stop();
    }

    /// One request, one reply with the same id. A reply with another id is a late answer to an earlier timed-out request
    /// and is skipped.
    fn request(&mut self, body: Value, wait: Duration) -> SbxResult<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let mut request = body;
        request["id"] = json!(id);
        self.pipe.send(&request.to_string()).map_err(|_| closed())?;
        let started = std::time::Instant::now();
        loop {
            let left = wait.checked_sub(started.elapsed()).ok_or_else(|| SandboxError::new("SANDBOX_MCP_TIMEOUT", "The connector did not answer in time."))?;
            let line = match self.pipe.recv(left) {
                Ok(Some(line)) => line,
                Ok(None) => return Err(closed()),
                Err(WslError::Timeout) => return Err(SandboxError::new("SANDBOX_MCP_TIMEOUT", "The connector did not answer in time.")),
                Err(_) => return Err(closed()),
            };
            let Ok(reply) = serde_json::from_str::<Value>(&line) else { continue };
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(reply);
            }
            // The hub's own words never carry a value; they are cut anyway.
            let message: String = reply.get("error").and_then(Value::as_str).unwrap_or("The connector could not do that.").chars().take(300).collect();
            return Err(SandboxError::new("SANDBOX_MCP_REFUSED", message));
        }
    }

    /// Replaces the hub's servers. `servers` is `[{name, command, args, env}]`: the one place a value is sent.
    pub fn config(&mut self, servers: &Value) -> SbxResult<()> {
        self.request(json!({ "op": "config", "servers": servers }), CONFIG_WAIT).map(|_| ())
    }

    /// `{servers: [{name, status, error, tools}]}`, the same shape the control service gives.
    pub fn tools(&mut self) -> SbxResult<Value> {
        let reply = self.request(json!({ "op": "tools" }), TOOLS_WAIT)?;
        Ok(json!({ "servers": reply.get("servers").cloned().unwrap_or_else(|| json!([])) }))
    }

    /// `{content, isError}`, the same shape the control service gives.
    pub fn call(&mut self, server: &str, tool: &str, arguments: &Value) -> SbxResult<Value> {
        let reply = self.request(json!({ "op": "call", "server": server, "tool": tool, "arguments": arguments }), CALL_WAIT)?;
        Ok(json!({ "content": reply.get("content").cloned().unwrap_or_else(|| json!([])), "isError": reply.get("isError").and_then(Value::as_bool).unwrap_or(false) }))
    }
}

impl Drop for SecretHub {
    fn drop(&mut self) {
        self.pipe.stop();
    }
}
