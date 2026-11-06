//! Tauri command surface for Potato Bot: panes, chat, companions, ACP chats and the bot itself.

pub(crate) mod acp_chat;
mod acp_conversations;
mod acp_install;
mod acp_lane;
mod acp_overlay;
mod acp_remove;
mod agents;
mod app;
mod cockpit;
mod chat_export;
mod companion;
mod pane_close;
mod pane_outbox;
mod panes;
mod potato;
mod potato_artifacts;
mod potato_attach;
mod potato_bot_settings;
mod potato_files;
mod potato_hold;
mod potato_mcp;
mod potato_panel;
mod potato_rooms;
mod potato_routines;
mod potato_sandbox;
mod potato_screen;
mod potato_search;
mod potato_skills;
mod potato_trash;
mod potato_usage;
mod questions;
mod secrets;
mod session_chat;
mod terminal_chat_prompt;

pub use acp_chat::*;
pub use acp_conversations::*;
pub use acp_install::*;
pub use acp_lane::*;
pub use acp_overlay::*;
pub use acp_remove::*;
pub use agents::*;
pub use app::*;
pub use cockpit::*;
pub use chat_export::*;
pub use companion::*;
pub use pane_close::*;
pub use pane_outbox::*;
pub use panes::*;
pub use potato::*;
pub use potato_artifacts::*;
pub use potato_attach::*;
pub use potato_bot_settings::*;
pub use potato_files::*;
pub use potato_hold::*;
pub use potato_mcp::*;
pub use potato_panel::*;
pub use potato_rooms::*;
pub use potato_routines::*;
pub use potato_sandbox::*;
pub use potato_screen::*;
pub use potato_search::*;
pub use potato_skills::*;
pub use potato_trash::*;
pub use potato_usage::*;
pub use questions::*;
pub use secrets::*;
pub use session_chat::*;
pub use terminal_chat_prompt::*;

use parking_lot::Mutex;
use serde::{Deserialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::pty::{
    registry_get_project_path, registry_kill_all, registry_kill_pane, registry_read_buffer,
    registry_read_raw_buffer, registry_read_screen_replay_at, registry_read_snapshot, registry_resize,
    registry_set_project_path, registry_spawn_pane, registry_write_input, PaneInfo, PaneRegistry,
    SpawnPaneArgs,
};
use crate::settings_store;

#[derive(Default)]
pub struct AppState {
    pub registry: Arc<Mutex<PaneRegistry>>,
    pub public_settings: Mutex<Value>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(PaneRegistry::new())),
            public_settings: Mutex::new(settings_store::default_public_settings()),
        }
    }
}

