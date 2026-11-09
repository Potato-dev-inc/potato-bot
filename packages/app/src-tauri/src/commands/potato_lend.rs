//! Tauri commands of Lend an App (the person lends an open Windows app to their bot). Windows only for now;
//! other systems get "not supported". Every command but `candidates` and `list` needs the Labs switch
//! `potato_lend_app_enabled`. `potato://lend` (no payload) tells the page to read `potato_lend_list` again.

use crate::potato::lend::session::{self, LentView, Mode};
use crate::potato::lend::{LendError, WinInfo};

const OFF: &str = "Lend an App is off (Labs)";

fn gate() -> Result<(), String> {
    if crate::potato::live::lend_labs_on() {
        Ok(())
    } else {
        Err(OFF.to_string())
    }
}

fn text(error: LendError) -> String {
    error.to_string()
}

fn mode_of(mode: &str) -> Result<Mode, String> {
    Mode::parse(mode).ok_or_else(|| "mode is visible or hidden".to_string())
}

/// Open windows the person could lend (already-lent ones are included; the page marks them).
#[tauri::command(async)]
pub fn potato_lend_candidates() -> Vec<WinInfo> {
    #[cfg(windows)]
    {
        crate::potato::lend::win::candidates()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[tauri::command(async)]
pub fn potato_lend_start(hwnd: isize, mode: String) -> Result<LentView, String> {
    gate()?;
    session::hub().lend(hwnd, mode_of(&mode)?).map_err(text)
}

#[tauri::command]
pub fn potato_lend_list() -> Vec<LentView> {
    session::hub().list()
}

#[tauri::command(async)]
pub fn potato_lend_bring_back(id: String) -> Result<(), String> {
    gate()?;
    session::hub().bring_back(&id).map_err(text)
}

#[tauri::command(async)]
pub fn potato_lend_set_mode(id: String, mode: String) -> Result<LentView, String> {
    gate()?;
    session::hub().set_mode(&id, mode_of(&mode)?).map_err(text)
}

#[tauri::command(async)]
pub fn potato_lend_take_over(id: String) -> Result<LentView, String> {
    gate()?;
    session::hub().take_over(&id).map_err(text)
}

#[tauri::command(async)]
pub fn potato_lend_hand_back(id: String) -> Result<LentView, String> {
    gate()?;
    session::hub().hand_back(&id).map_err(text)
}

/// A small PNG (at most 320 px wide) of a window, base64, for the picker.
#[tauri::command(async)]
pub fn potato_lend_thumbnail(hwnd: isize) -> Result<String, String> {
    gate()?;
    #[cfg(windows)]
    {
        use base64::Engine;
        let png = crate::potato::lend::capture::png_scaled(hwnd, 320).map_err(text)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(png))
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        Err(text(LendError::Unsupported))
    }
}
