//! The person's yes, at this PC, to one phone using the mouse and keyboard on the bots' computer ("Can control the
//! bot's computer", Settings > Phone). Saved with the phone's pairing record and mirrored into the screen hub, which is
//! what the phone API and the screen relay check (the relay rechecks it every second, so taking the yes back closes a
//! live control link within moments).
//!
//! Only the PC changes it: the Tauri command `phone_device_control_set` calls `set`; no phone API route does.

use crate::mobile_pairing::SharedPairingStore;
use crate::potato::sandbox::screen::ScreenHub;

/// Saves the yes (or its removal) for one paired phone and applies it to the hub. An unknown phone is an error and
/// changes nothing.
pub fn set(pairing: &SharedPairingStore, hub: &ScreenHub, device_id: &str, on: bool) -> Result<(), String> {
    let known = pairing.lock().set_phone_control(device_id, on)?;
    if !known {
        return Err("PHONE_NOT_FOUND: That phone is not paired any more.".to_string());
    }
    if on {
        hub.allow_phone_control(device_id);
    } else {
        hub.revoke_phone_control(device_id);
    }
    Ok(())
}

/// At startup: every saved yes goes into the hub (the hub starts with none).
pub fn apply_saved(pairing: &SharedPairingStore, hub: &ScreenHub) {
    for id in pairing.lock().phone_control_ids() {
        hub.allow_phone_control(&id);
    }
}

/// A phone that is signed out or removed loses its yes at once (its record, with the saved switch, is gone too).
pub fn clear(hub: &ScreenHub, device_id: &str) {
    hub.revoke_phone_control(device_id);
}
