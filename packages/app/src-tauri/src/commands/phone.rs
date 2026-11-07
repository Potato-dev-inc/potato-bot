//! Tauri commands of Settings > Phone. Desktop only: a phone can never reach them, and none returns a token
//! (the PC never has one; it keeps only a hash). Errors are one string, `CODE: plain sentence`.

use crate::potato::phone::live::{self, PairingOut, PhoneStatus};

fn phone() -> Result<std::sync::Arc<live::Phone>, String> {
    live::phone().ok_or_else(|| "PHONE_NOT_READY: The phone server is not ready yet. Try again in a moment.".to_string())
}

/// Runs a blocking call off the UI thread (the firewall and tunnel calls wait on programs).
async fn run<T: Send + 'static>(call: impl FnOnce(std::sync::Arc<live::Phone>) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let p = phone()?;
    tauri::async_runtime::spawn_blocking(move || call(p)).await.map_err(|e| format!("INTERNAL: {e}"))?
}

#[tauri::command]
pub async fn phone_status() -> Result<PhoneStatus, String> {
    run(|p| Ok(p.status())).await
}

#[tauri::command]
pub async fn phone_pairing_start(via: Option<String>, relink_device_id: Option<String>) -> Result<PairingOut, String> {
    run(move |p| p.start_pairing(via.as_deref(), relink_device_id)).await
}

#[tauri::command]
pub async fn phone_device_revoke(device_id: String) -> Result<PhoneStatus, String> {
    run(move |p| {
        p.revoke(&device_id)?;
        Ok(p.status())
    })
    .await
}

#[tauri::command]
pub async fn phone_set_port(port: u16) -> Result<PhoneStatus, String> {
    run(move |p| {
        p.set_port(port)?;
        Ok(p.status())
    })
    .await
}

#[tauri::command]
pub async fn phone_set_custom_address(address: Option<String>) -> Result<PhoneStatus, String> {
    run(move |p| {
        p.set_custom_address(address)?;
        Ok(p.status())
    })
    .await
}

#[tauri::command]
pub async fn phone_tunnel_set(on: bool) -> Result<PhoneStatus, String> {
    run(move |p| {
        p.set_tunnel(on)?;
        Ok(p.status())
    })
    .await
}

#[tauri::command]
pub async fn phone_firewall_allow() -> Result<PhoneStatus, String> {
    run(|p| {
        p.firewall_allow()?;
        Ok(p.status())
    })
    .await
}
