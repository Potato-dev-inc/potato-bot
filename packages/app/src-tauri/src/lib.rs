#![recursion_limit = "512"]

mod acp;
mod actors;
mod agent_authority;
mod agent_run_mode;
mod agent_contexts;
mod agent_pack;
mod ambient;
mod audit;
mod loop_guard;
mod mcp_install;
mod notifications;
mod toolbelt;
mod wsl;
mod agent_adapters;
pub mod agent_runs;
mod app_lifecycle;
mod autostart;
pub mod app_paths;
mod bridge;
mod budgets;
mod commands;
mod crash;
mod event_log;
mod git_exclude;
mod handoff;
mod events;
pub mod mcp_sessions;
mod mobile_pairing;
mod one_brain;
mod companion;
mod potato;
mod opencode;
pub mod operations;
#[cfg(test)]
mod operations_contract_tests;
mod pane_activity;
mod pane_jobs;
mod pane_preview;
mod panic;
mod attention;
pub mod pane_outbox;
mod pane_prompt;
mod prompt_risk;
pub mod questions;
mod push;
pub mod redact;
mod flight;
mod secure_file;
mod app_windows;
mod cockpit;
mod ui_mode;
mod pane_windows;
mod tray;
mod pane_wait;
mod pane_wait_notify;
mod platform;
pub mod proc_util;
mod path_limit;
mod project_path;
mod projections;
mod pty;
#[cfg(feature = "dev-fake-tui")]
#[doc(hidden)]
pub mod fake_tui;
mod session_context;
mod chat_index;
mod session_chat;
mod secrets;
mod settings_store;
mod settings_patch;
mod shell_env;
mod startup_timing;
pub mod shell_exec;
mod standby;
mod terminal_chat;
pub mod tool_registry;
mod tui;
pub mod turn_summary;
pub mod worktrees;
pub mod env_probe;

use commands::AppState;
use std::path::{PathBuf};
use tauri::{Listener, Manager};

/// Command-line modes of the app executable that run without a window (`acp-bridge --agent <id>`: a chat
/// adapter for an agent that has no ACP). `Some(exit code)` when one ran; `main` exits with it. `None`:
/// start the app as usual.
pub fn run_cli_subcommand() -> Option<i32> {
    acp::bridge::run_cli(&std::env::args().skip(1).collect::<Vec<_>>())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    shell_env::apply_to_process();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,puppet_master_app_lib=debug")
            }),
        )
        .with_target(false)
        // Console as before, plus the last lines kept in memory for the diagnostic file.
        .with_writer(flight::applog::Tee)
        .init();

    // `PUPPET_MASTER_PROFILE=<name>`: a separate app (own identifier, so its own settings store,
    // webview data and single-instance lock). The default profile keeps the configured identifier.
    let mut context = tauri::generate_context!();
    if app_paths::profile().is_some() {
        context.config_mut().identifier = app_paths::app_id();
    }

    tauri::Builder::default()
        // A second launch opens the running app's cockpit and exits, so it never starts
        // another bridge or rewrites the bridge port file.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second launch opens the primary window (the cockpit); `--new-pane <agent>` opens only
            // a pop-out pane (no Settings) through the same code path as the tray's "New pane" menu.
            match tray::second_launch_action(args.as_slice()) {
                tray::SecondLaunch::Settings => cockpit::open_primary_window(app),
                // A sign-in start while the app runs: no window, no focus.
                tray::SecondLaunch::Quiet => tracing::info!("sign-in start while running: nothing shown"),
                tray::SecondLaunch::NewPane(agent) => {
                    let app = app.clone();
                    std::thread::spawn(move || match tray::new_pane(&app, &agent) {
                        Ok(id) => tracing::info!(%agent, pane = %id, "second launch: opened new pane"),
                        Err(e) => tracing::warn!(%agent, error = %e, "second launch: new pane failed"),
                    });
                }
            }
        }))
        .plugin(autostart::plugin())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::new())
        .manage(commands::SessionChatState::default())
        .invoke_handler(tauri::generate_handler![
            commands::acp_addon_install,
            commands::acp_addon_remove,
            commands::acp_addon_status,
            commands::acp_back_to_chat,
            commands::chat_export_save,
            commands::pane_set_title,
            commands::acp_chat_cancel,
            commands::acp_chat_keep_waiting,
            commands::acp_chat_remove,
            commands::acp_chat_restart,
            commands::acp_chat_retry_sign_in,
            commands::acp_chat_set_agent_answering,
            commands::acp_chat_set_config,
            commands::acp_chat_start,
            commands::acp_chat_state,
            commands::acp_conversation_create,
            commands::acp_conversation_notices_seen,
            commands::acp_conversation_of_terminal,
            commands::acp_conversations_list,
            commands::acp_conversations_restore,
            commands::acp_open_in_terminal,
            commands::acp_overlay_detect,
            commands::acp_overlay_install,
            commands::acp_overlay_remove,
            commands::acp_overlay_resolve,
            commands::acp_overlay_set,
            commands::agent_lane_get,
            commands::agent_lane_set,
            commands::agents_list,
            commands::answer_pane_prompt,
            commands::chat_search,
            commands::chat_search_facets,
            commands::companion_agents,
            commands::companion_archive,
            commands::companion_checkins_set,
            commands::companion_create,
            commands::companion_delete,
            commands::companion_feed,
            commands::companion_get,
            commands::companion_list,
            commands::companion_memory_get,
            commands::companion_memory_save,
            commands::companion_open_folder,
            commands::companion_outside_change_decide,
            commands::companion_pending_decide,
            commands::companion_rehome,
            commands::companion_restart,
            commands::companion_skill_approve,
            commands::companion_skill_get,
            commands::companion_skill_remove,
            commands::companion_skill_save,
            commands::companion_sleep,
            commands::companion_tidy_up,
            commands::companion_update,
            commands::companion_wake,
            commands::confirm_pane_window_close,
            commands::dev_labs_override_active,
            commands::get_project_path_cmd,
            commands::get_ui_mode,
            commands::interrupt_pane,
            commands::kill_all_panes,
            commands::kill_pane_cmd,
            commands::list_panes,
            commands::open_cockpit,
            commands::open_settings,
            commands::pane_message_cancel,
            commands::pane_message_send,
            commands::pane_messages_list,
            commands::pane_prompt,
            commands::potato_answer_card,
            commands::potato_answer_card_always,
            commands::potato_submit_login,
            commands::potato_login_origin,
            commands::potato_confirm_login,
            commands::potato_forget_site,
            commands::potato_artifact_delete,
            commands::potato_artifact_export,
            commands::potato_artifact_list,
            commands::potato_artifact_open,
            commands::potato_artifact_reveal,
            commands::potato_artifact_save_message,
            commands::potato_files_download,
            commands::potato_files_list,
            commands::potato_files_preview,
            commands::potato_files_upload,
            commands::potato_bot_add,
            commands::potato_bot_brain_set,
            commands::potato_bot_delete,
            commands::potato_bot_export_markdown,
            commands::potato_bot_instructions_restore,
            commands::potato_bot_instructions_save,
            commands::potato_bot_options_get,
            commands::potato_bot_provider_set,
            commands::potato_bot_rename,
            commands::potato_bot_start,
            commands::potato_bot_stop,
            commands::potato_bot_tools_set,
            commands::potato_bot_level_set,
            commands::potato_bot_sites_set,
            commands::potato_bot_always_remove,
            commands::potato_check_key,
            commands::potato_forget_talk_memory,
            commands::potato_helper_decline,
            commands::potato_helper_folders,
            commands::potato_helper_start,
            commands::potato_hold_state,
            commands::potato_install_node,
            commands::potato_sandbox_limits_get,
            commands::potato_sandbox_limits_set,
            commands::potato_sandbox_memory_return_get,
            commands::potato_sandbox_memory_return_enable,
            commands::app_update_check,
            commands::app_update_busy,
            commands::app_update_install,
            commands::potato_log,
            commands::potato_bot_archive,
            commands::potato_mark_read,
            commands::potato_toggle_reaction,
            commands::potato_memory_delete,
            commands::potato_memory_list,
            commands::potato_memory_read,
            commands::potato_messages,
            commands::potato_open_cards,
            commands::potato_job_file_restore,
            commands::potato_job_files,
            commands::potato_job_get,
            commands::potato_notify_get,
            commands::potato_notify_set,
            commands::potato_open_work_add,
            commands::potato_open_work_list,
            commands::potato_open_work_remove,
            commands::potato_open_work_set_done,
            commands::potato_pause_all,
            commands::potato_resume_all,
            commands::potato_routine_create,
            commands::potato_routine_cron_preview,
            commands::potato_routine_delete,
            commands::potato_routine_pause,
            commands::potato_routine_run_now,
            commands::potato_routine_runs,
            commands::potato_routine_update,
            commands::potato_routines_list,
            commands::potato_wait_cancel,
            commands::potato_waits_list,
            commands::potato_rooms_list,
            commands::potato_room_create,
            commands::potato_room_delete,
            commands::potato_room_lines,
            commands::potato_room_mark_read,
            commands::potato_room_rename,
            commands::potato_room_send,
            commands::potato_room_set_bots,
            commands::potato_room_set_settings,
            commands::potato_mcp_add,
            commands::potato_mcp_remote_add,
            commands::potato_mcp_tools_set,
            commands::potato_mcp_oauth_sign_in,
            commands::potato_mcp_oauth_sign_out,
            commands::potato_mcp_catalog_add,
            commands::potato_mcp_overview,
            commands::potato_mcp_remove,
            commands::potato_mcp_set_enabled,
            commands::potato_mcp_test,
            commands::potato_connector_connect,
            commands::potato_connector_reads,
            commands::potato_sandbox_add_os,
            commands::potato_sandbox_cancel_install,
            commands::potato_sandbox_continue_install,
            commands::potato_sandbox_detect,
            commands::potato_sandbox_dismiss_install,
            commands::potato_sandbox_install,
            commands::potato_sandbox_linux_ask,
            commands::potato_sandbox_linux_enable,
            commands::potato_sandbox_menu,
            commands::potato_sandbox_remove,
            commands::potato_sandbox_reset,
            commands::potato_sandbox_set_idle,
            commands::potato_sandbox_start,
            commands::potato_sandbox_status,
            commands::potato_sandbox_stop,
            commands::potato_sandbox_switch_os,
            commands::potato_sandbox_vercel_install,
            commands::potato_sandbox_vercel_status,
            commands::potato_screen_hand_back,
            commands::potato_screen_keep,
            commands::potato_screen_link,
            commands::potato_screen_skip,
            commands::potato_screen_state,
            commands::potato_screen_take_over,
            commands::potato_send,
            commands::potato_send_files,
            commands::potato_set_persona,
            commands::potato_setting_undo,
            commands::potato_settings_get,
            commands::potato_settings_set,
            commands::potato_state,
            commands::potato_search_chats,
            commands::potato_skill_delete,
            commands::potato_skill_list,
            commands::potato_skill_save,
            commands::potato_trash_list,
            commands::potato_trash_restore,
            commands::potato_usage,
            commands::profile_bridge_url,
            commands::question_answer,
            commands::question_dismiss,
            commands::questions_list,
            commands::read_pane_buffer,
            commands::read_pane_raw_buffer,
            commands::read_pane_screen_replay,
            commands::read_pane_snapshot,
            commands::reclaim_pane_size,
            commands::resize_pane,
            commands::secret_delete,
            commands::secret_get_for_llm,
            commands::secret_get_masked,
            commands::secret_set,
            commands::secret_storage_info,
            commands::session_chat_close,
            commands::session_chat_open,
            commands::set_project_path,
            commands::set_ui_mode,
            commands::spawn_pane,
            commands::terminal_chat_answer,
            commands::terminal_chat_prompt,
            commands::write_pane_input,
            app_lifecycle::get_app_install_info,
            app_lifecycle::open_external_url,
            app_lifecycle::launch_uninstall,
            app_lifecycle::autostart_get,
            app_lifecycle::autostart_set,
            settings_patch::settings_patch,
        ])
        .setup(|app| {
            // First, so a panic anywhere below leaves a file behind (the console and the in-memory
            // log tail die with the process).
            crash::install(app_paths::app_data_dir());
            // Register approved declarative agent definitions before anything resolves a profile.
            tui::declarative::loader::reload();
            bridge::start_definitions_watcher(app.handle().clone());
            let port_file = app_paths::bridge_port_file();
            let pairing_file = app_paths::pairing_file();
            if let Err(err) = event_log::init_global_event_log(event_log::event_log_path()) {
                tracing::warn!(%err, "event log not initialized");
            }
            let initial_project_path = {
                let state = app.state::<AppState>();
                pty::registry_get_project_path(&state.registry)
            };
            event_log::set_active_project_path(Some(PathBuf::from(initial_project_path)));

            let registry = app.state::<AppState>().registry.clone();
            let bridge_url = match bridge::start_embedded_bridge(
                registry,
                app.handle().clone(),
                port_file,
                pairing_file,
            ) {
                Ok(handle) => {
                    tracing::info!(url = %handle.url, "embedded bridge started");
                    Some(handle.url)
                }
                Err(err) => {
                    tracing::warn!(%err, "embedded bridge not started — external MCP unavailable");
                    None
                }
            };

            push::init(app.state::<AppState>().registry.clone());

            // Chat conversations that were live come back, lazily: no adapter starts until one is used.
            {
                let state = app.state::<AppState>();
                let report = commands::acp_restore_at_start(app.handle(), &state);
                tracing::info!(restored = report.restored.len(), in_terminal = report.in_terminal.len(), failed = report.failed.len(), "acp: conversations restored");
            }

            let _ = &bridge_url;


            app.handle().listen("terminal-data", move |event| {
                let payload = event.payload();
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                    let pane_id = value.get("pane_id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = value.get("data").and_then(|v| v.as_array());
                    if !pane_id.is_empty() {
                        if let Some(bytes) = data {
                            let raw: Vec<u8> = bytes
                                .iter()
                                .filter_map(|v| v.as_u64().map(|n| n as u8))
                                .collect();
                            let offset = value.get("offset").and_then(|v| v.as_u64());
                            bridge::push_terminal_sse(pane_id, &raw, offset);
                        }
                    }
                }
            });

            let registry_for_panes = app.state::<AppState>().registry.clone();
            pane_windows::init(app.handle(), registry_for_panes.clone());
            secrets::migrate_on_startup();
            // What broke it: remove test copies an earlier run left behind (SPEC n33 section 3.4).
            // The sandbox host builds its manager (and freezes where the guest files are) as soon as the saved settings
            // are applied, so it must know the app handle first. 1.0.1 set it later and found no files on a clean PC.
            potato::sandbox_host::set_app(app.handle().clone());
            #[cfg(not(test))]
            potato::sidecar::set_bundled_node(potato::resource_dirs::installed(app.handle(), "node", "node.exe").map(|dir| dir.join("node.exe")));
            potato::sandbox_host::startup_diag();
            commands::startup_probe(app.handle().clone());
            #[cfg(not(test))]
            settings_store::load_saved_public_settings(app.handle());
            // With the Labs switch on (known now), each conversation that came back is a chat pane in the pane list.
            // Nothing starts yet: the adapter starts when the chat is read or a message is sent.
            if acp::chat_enabled() {
                let state = app.state::<AppState>();
                let deps = commands::acp_chat::live_deps(app.handle(), &state.registry);
                let panes = acp::chat::restore_panes(&deps);
                tracing::info!(panes = panes.len(), "acp: chat panes restored");
            }
            ui_mode::load_saved();
            if let Err(e) = tray::init(app.handle()) {
                tracing::warn!(error = %e, "tray icon failed to start");
            }
            // A normal start opens the primary window (same as a tray click: always the cockpit, in
            // either UI mode; Individual mode still puts Settings in its own window). A sign-in start,
            // a worker host and a headless start (`--no-window` / `PUPPET_MASTER_NO_WINDOW`) leave only
            // the tray.
            let args = std::env::args().collect::<Vec<_>>();
            let no_window_env = std::env::var(cockpit::NO_WINDOW_ENV).ok();
            if cockpit::startup_window(&args, false, no_window_env.as_deref()) {
                cockpit::open_primary_window(app.handle());
            } else {
                tracing::info!("started without a window: tray only");
            }
            standby::init(registry_for_panes.clone(), app.handle().clone());
            // The Agent Wizard's panes (guided capture, visible test) go through the app's pane registry. The
            // host is installed here, not referenced from the wizard module, so the Tauri window code stays out
            // of the unit-test executables.
            tui::declarative::wizard::visible::host::install(std::sync::Arc::new(
                tui::declarative::wizard::visible::registry_host::RegistryHost::new(registry_for_panes.clone(), app.handle().clone()),
            ));
            pane_activity::init(registry_for_panes.clone(), app.handle().clone());
            loop_guard::init(app.handle().clone());
            panic::init_shortcut(app.handle());
            turn_summary::start_presence();
            attention::init();
            // Focus: load the saved state (resume or end it), then start the tick thread. Once, here.
            attention::focus::live::start(app.handle());

            companion::live::start(app.handle().clone(), registry_for_panes.clone());
            // Potato Bot: the engine and its supervisor thread. No brain starts here; one starts on its first message.
            potato::live::start(Some(app.handle().clone()), Some(registry_for_panes.clone()));
            // The Computer panel: the screen hub, its 1 s keeper and the `potato://screen` event.
            potato::sandbox::screen_live::start(Some(app.handle().clone()));
            // A room a crash left on the relay is ended now (its own thread: it may wait for the network).
            app.handle().listen("pty://panes-changed", move |_event| {
                bridge::push_panes_sse(&registry_for_panes);
            });

            app.handle().listen("pty://status", |event| {
                let payload = event.payload();
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                    let pane_id = value.get("pane_id").and_then(|v| v.as_str()).unwrap_or("");
                    let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    if !pane_id.is_empty() && !status.is_empty() {
                        bridge::push_pane_status_sse(pane_id, status);
                    }
                }
            });

            Ok(())
        })
        .build(context)
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Closing the last window (cockpit, a pane, Settings) must not end the app: it stays in the tray.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                crash::record_exit(&app_paths::app_data_dir(), *code, "ExitRequested");
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            if let tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::CloseRequested { api, .. }, .. } = &event {
                if app_windows::on_close_requested(app_handle, label) {
                    api.prevent_close();
                }
                if pane_windows::on_close_requested(app_handle, label) {
                    api.prevent_close();
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                crash::record_exit(&app_paths::app_data_dir(), None, "Exit");
                // The link ends with the app: best effort, the relay's time limit ends it otherwise.
                standby::shutdown();
                // The keyboard hook of My computers goes with the app.
                // Potato Bot's brains leave with the app: asked first, then their whole tree is stopped.
                potato::live::stop_all(std::time::Duration::from_secs(2));
                // The computer may sleep again: a Night Shift hold must not outlive the app.
                if let Some(chat) = app_handle.try_state::<commands::SessionChatState>() {
                    chat.shutdown();
                }
                bridge::port_file::release_owned(&app_paths::bridge_port_file());
                if let Some(state) = app_handle.try_state::<AppState>() {
                    // The process ends right after this: wait for every tree to be stopped.
                    pty::registry_kill_all_and_wait(&state.registry);
                }
            }
        });
}
