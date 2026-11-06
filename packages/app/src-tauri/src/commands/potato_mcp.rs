//! Tauri commands of a Potato Bot's tool servers (the MCP list on its Advanced tab). Desktop only, like the other Potato
//! commands. The logic is in `potato::mcp`; these wrappers only find the sandbox and the settings folder, and run the
//! call off the UI thread (a test or a start can take a while).
//!
//! Errors are one string, `CODE: plain sentence`. The env VALUES go in with `potato_mcp_add` and never come back.

use std::sync::Arc;

use crate::potato::live;
use crate::potato::mcp::{self, NewServer, Overview, TestResult};
use crate::potato::sandbox::manager::Manager;
use crate::potato::sandbox::Sandbox;
use crate::potato::sandbox_host;
use crate::potato::store::StoreError;

fn text(error: StoreError) -> String {
    error.to_string()
}

/// Runs one change against the bot's list and the sandbox. Off the UI thread.
async fn run<T: Send + 'static>(call: impl FnOnce(&std::path::Path, &dyn Sandbox) -> Result<T, StoreError> + Send + 'static) -> Result<T, String> {
    let manager: Arc<Manager> = sandbox_host::manager()?;
    tauri::async_runtime::spawn_blocking(move || {
        let root = live::root();
        call(&root, manager.sandbox().as_ref()).map_err(text)
    })
    .await
    .map_err(|e| format!("INTERNAL: {e}"))?
}

/// The built-in computer, the catalog and the bot's servers with their status.
#[tauri::command]
pub async fn potato_mcp_overview(bot_id: String) -> Result<Overview, String> {
    run(move |root, sandbox| mcp::overview(root, &bot_id, sandbox)).await
}

/// Adds one catalog entry (enabled), then sends the list when the computer runs.
#[tauri::command]
pub async fn potato_mcp_catalog_add(bot_id: String, id: String) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::add_catalog(root, &bot_id, &id, sandbox)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Adds the person's own server. Its env values go into the credential store, not the settings.
#[tauri::command]
pub async fn potato_mcp_add(bot_id: String, server: NewServer) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::add_server(root, &bot_id, server, sandbox)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Removes a server and its env values.
#[tauri::command]
pub async fn potato_mcp_remove(bot_id: String, name: String) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::remove_server(root, &bot_id, &name, sandbox)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Switches a server on or off.
#[tauri::command]
pub async fn potato_mcp_set_enabled(bot_id: String, name: String, enabled: bool) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::set_enabled(root, &bot_id, &name, enabled, sandbox)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Sends the list and asks the computer which tools the server offers.
#[tauri::command]
pub async fn potato_mcp_test(bot_id: String, name: String) -> Result<TestResult, String> {
    run(move |root, sandbox| mcp::test_server(root, &bot_id, &name, sandbox)).await
}

/// Connects a catalog connector: installs it on the running computer once and keeps the values in the credential store.
/// The values leave the page here and never come back.
#[tauri::command]
pub async fn potato_connector_connect(bot_id: String, id: String, values: std::collections::BTreeMap<String, String>) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::connect(root, &bot_id, &id, values, sandbox)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Switches "Always allow reads" for one connector.
#[tauri::command]
pub async fn potato_connector_reads(bot_id: String, name: String, on: bool) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::set_always_allow_reads(root, &bot_id, &name, on)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Adds a tool server by link. The token (if any) goes to the credential store and is used only by this app when it calls the
/// server; it never comes back. Local allows an address on this computer or the person's own network.
#[tauri::command]
pub async fn potato_mcp_remote_add(bot_id: String, name: String, url: String, local: bool, token: Option<String>) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::add_remote(root, &bot_id, &name, &url, local, token)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Sets which tools of a server the bot may call (`allow`, empty means all) and which it may never call (`block`).
#[tauri::command]
pub async fn potato_mcp_tools_set(bot_id: String, name: String, allow: Vec<String>, block: Vec<String>) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::set_tool_lists(root, &bot_id, &name, allow, block)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Signs a tool server by link in, in the person's browser. The tokens go to the credential store and never come back.
/// Waits (off the UI thread) until the person finishes or the time runs out.
#[tauri::command]
pub async fn potato_mcp_oauth_sign_in(bot_id: String, name: String, client_id: Option<String>) -> Result<Overview, String> {
    run(move |root, sandbox| {
        let wire = mcp::wire_for(&crate::potato::bot_settings::load(root, &bot_id)?.mcp_servers.into_iter().find(|s| s.name == name).unwrap_or_default());
        let open = |url: &str| crate::app_lifecycle::open_external_url(url.to_string());
        mcp::oauth_sign_in(root, &bot_id, &name, client_id.as_deref(), wire.as_ref(), std::time::Duration::from_secs(crate::potato::mcp_oauth::SIGN_IN_SECS), &open)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}

/// Forgets a server's sign-in.
#[tauri::command]
pub async fn potato_mcp_oauth_sign_out(bot_id: String, name: String) -> Result<Overview, String> {
    run(move |root, sandbox| {
        mcp::oauth_sign_out(root, &bot_id, &name)?;
        mcp::overview(root, &bot_id, sandbox)
    })
    .await
}
