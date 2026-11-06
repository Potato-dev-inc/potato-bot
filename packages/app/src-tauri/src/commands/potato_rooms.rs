//! Tauri commands of bot groups (plan 12, slice G8). Desktop only, like the other Potato commands. Every command
//! refuses unless the Labs switch `potato_groups_enabled` is on (and Potato Bot is on too).
//!
//! Errors are one string, `CODE: plain sentence` (the words match `RoomError::to_wire`). No error text carries a
//! path, a secret or model output. A change is announced on `potato://room-changed` (payload `{ roomId }`).

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;

use crate::potato::live;
use crate::potato::rooms::store::{self as rstore, Room, RoomError, RoomLine};
use crate::potato::service::Service;
use crate::potato::store;
use crate::settings_store;

const GROUPS_OFF: &str = "POTATO_GROUPS_OFF: Turn on Bot groups in Labs first.";
const TOO_LONG: &str = "POTATO_TOO_LONG: This message is too long.";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotChip {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    pub id: String,
    pub name: String,
    pub bots: Vec<BotChip>,
    pub last_ms: i64,
    pub unread: u32,
    /// What the owner wrote for the bots of this group.
    pub instructions: String,
    /// The shared folder's name as chosen (empty: the default, the group's id) and its full path inside the bots' computer.
    pub folder: String,
    pub folder_path: String,
    /// `listen` or `mentions`.
    pub mode: String,
    /// Bots switched off in this group only.
    pub off_bots: Vec<String>,
    /// The names of the bots now typing in this group.
    pub typing: Vec<String>,
    /// The live step list of each bot working in this group.
    pub work: Vec<RoomWork>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomWork {
    pub bot: String,
    pub run: crate::potato::work::WorkRun,
}

/// Refuses unless the group switch is on.
pub fn groups_gate(on: bool) -> Result<(), String> {
    if on {
        Ok(())
    } else {
        Err(GROUPS_OFF.to_string())
    }
}

fn wire(e: RoomError) -> String {
    e.to_wire()
}

fn io_error() -> String {
    wire(RoomError::Io(String::new()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn service() -> Result<Arc<Service>, String> {
    live::service().ok_or_else(|| "POTATO_NOT_READY: Potato Bot is not ready yet. Try again in a moment.".to_string())
}

/// The gate, then one call off the UI thread.
async fn run<T: Send + 'static>(app: &AppHandle, call: impl FnOnce(Arc<Service>) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    groups_gate(live::groups_labs_on(&settings_store::read_public_settings(app)))?;
    let service = service()?;
    tauri::async_runtime::spawn_blocking(move || call(service)).await.map_err(|_| "INTERNAL: A group task stopped.".to_string())?
}

fn known_bots(root: &Path) -> Result<Vec<String>, String> {
    store::load_bots(root).map(|bots| bots.into_iter().map(|b| b.id).collect()).map_err(|_| io_error())
}

fn index_of(rooms: &[Room], room_id: &str) -> Result<usize, String> {
    if !store::valid_room_id(room_id) {
        return Err(wire(RoomError::BadId));
    }
    rooms.iter().position(|r| r.id == room_id).ok_or_else(|| wire(RoomError::NotFound))
}

fn view(service: &Service, room: &Room) -> RoomView {
    RoomView {
        id: room.id.clone(),
        name: room.name.clone(),
        bots: room.bot_ids.iter().map(|b| BotChip { id: b.clone(), name: service.bot_name(b) }).collect(),
        last_ms: room.last_ms,
        unread: room.unread,
        instructions: room.instructions.clone(),
        folder: room.folder.clone(),
        folder_path: room.folder_path(),
        mode: room.mode.as_str().to_string(),
        off_bots: room.off_bots.clone(),
        typing: service.room_typing(&room.id),
        work: service.room_work(&room.id).into_iter().map(|(bot, run)| RoomWork { bot, run }).collect(),
    }
}

#[tauri::command]
pub async fn potato_rooms_list(app: AppHandle) -> Result<Vec<RoomView>, String> {
    run(&app, |s| {
        let rooms = rstore::load_rooms(&live::root()).map_err(wire)?;
        Ok(rooms.iter().map(|r| view(&s, r)).collect())
    })
    .await
}

#[tauri::command]
pub async fn potato_room_create(app: AppHandle, name: String, bot_ids: Vec<String>) -> Result<RoomView, String> {
    run(&app, move |s| {
        let root = live::root();
        let (name, bot_ids) = rstore::validate(&name, &bot_ids, &known_bots(&root)?).map_err(wire)?;
        let mut rooms = rstore::load_rooms(&root).map_err(wire)?;
        let now = now_ms();
        let room = Room { id: rstore::new_id('r'), name, bot_ids, created_ms: now, last_ms: now, unread: 0, ..Default::default() };
        rooms.push(room.clone());
        rstore::save_rooms(&root, &rooms).map_err(wire)?;
        s.notify_room(&room.id);
        Ok(view(&s, &room))
    })
    .await
}

#[tauri::command]
pub async fn potato_room_rename(app: AppHandle, room_id: String, name: String) -> Result<RoomView, String> {
    run(&app, move |s| {
        let root = live::root();
        let mut rooms = rstore::load_rooms(&root).map_err(wire)?;
        let i = index_of(&rooms, &room_id)?;
        let (name, _) = rstore::validate(&name, &rooms[i].bot_ids, &known_bots(&root)?).map_err(wire)?;
        rooms[i].name = name;
        rstore::save_rooms(&root, &rooms).map_err(wire)?;
        s.notify_room(&room_id);
        Ok(view(&s, &rooms[i]))
    })
    .await
}

#[tauri::command]
pub async fn potato_room_set_bots(app: AppHandle, room_id: String, bot_ids: Vec<String>) -> Result<RoomView, String> {
    run(&app, move |s| {
        let root = live::root();
        let mut rooms = rstore::load_rooms(&root).map_err(wire)?;
        let i = index_of(&rooms, &room_id)?;
        let (_, bot_ids) = rstore::validate(&rooms[i].name, &bot_ids, &known_bots(&root)?).map_err(wire)?;
        rstore::set_members(&mut rooms[i], bot_ids);
        rstore::save_rooms(&root, &rooms).map_err(wire)?;
        s.notify_room(&room_id);
        Ok(view(&s, &rooms[i]))
    })
    .await
}

/// The group settings screen's Save: instructions, shared folder, Listen or Mentions, and who is switched off here.
/// Nothing is saved unless all of it is valid. Bots read the new settings from the next line they are given.
#[tauri::command]
pub async fn potato_room_set_settings(app: AppHandle, room_id: String, instructions: String, folder: String, mode: String, off_bots: Vec<String>) -> Result<RoomView, String> {
    run(&app, move |s| {
        let root = live::root();
        let mut rooms = rstore::load_rooms(&root).map_err(wire)?;
        let i = index_of(&rooms, &room_id)?;
        rstore::apply_settings(&mut rooms[i], &instructions, &folder, &mode, &off_bots).map_err(wire)?;
        rstore::save_rooms(&root, &rooms).map_err(wire)?;
        s.notify_room(&room_id);
        Ok(view(&s, &rooms[i]))
    })
    .await
}

#[tauri::command]
pub async fn potato_room_delete(app: AppHandle, room_id: String) -> Result<(), String> {
    run(&app, move |s| {
        rstore::delete_room(&live::root(), &room_id).map_err(wire)?;
        s.notify_room(&room_id);
        Ok(())
    })
    .await
}

/// The newest `limit` lines of a group, oldest first (the store keeps at most 500 per read).
#[tauri::command]
pub async fn potato_room_lines(app: AppHandle, room_id: String, limit: u32) -> Result<Vec<RoomLine>, String> {
    run(&app, move |s| s.room_lines(&room_id, limit as usize).map_err(|e| e.to_string())).await
}

/// One line from the person. The engine refuses an empty line with `POTATO_EMPTY`.
#[tauri::command]
pub async fn potato_room_send(app: AppHandle, room_id: String, text: String) -> Result<RoomLine, String> {
    run(&app, move |s| {
        if text.trim().chars().count() > rstore::MAX_TEXT {
            return Err(TOO_LONG.to_string());
        }
        s.send_room(&room_id, &text).map_err(|e| e.to_string())
    })
    .await
}

/// Clears the unread count of a group. Does not announce a change (the page already shows it).
#[tauri::command]
pub async fn potato_room_mark_read(app: AppHandle, room_id: String) -> Result<(), String> {
    run(&app, move |_| {
        let root = live::root();
        let mut rooms = rstore::load_rooms(&root).map_err(wire)?;
        let i = index_of(&rooms, &room_id)?;
        if rooms[i].unread != 0 {
            rooms[i].unread = 0;
            rstore::save_rooms(&root, &rooms).map_err(wire)?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_refuse_when_groups_off() {
        let err = groups_gate(false).unwrap_err();
        assert!(err.starts_with("POTATO_GROUPS_OFF:"));
        assert!(groups_gate(true).is_ok());
    }

    #[test]
    fn bad_room_ids_are_refused_before_any_file_is_read() {
        assert_eq!(index_of(&[], "../etc").unwrap_err(), wire(RoomError::BadId));
        assert_eq!(index_of(&[], "r000000000001").unwrap_err(), wire(RoomError::NotFound));
    }
}
