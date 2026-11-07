//! The phone server's saved setting: on/off, the port, and the person's own address. One small file in
//! `<app data>/phone/`. The tunnel keeps its own record next to it (`tunnel.rs`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub enabled: bool,
    pub port: u16,
    /// `https://bot.example.com` or `http://myhost:8123`: where a reverse proxy or a named tunnel in front of the
    /// phone server answers. It joins the QR's addresses ahead of the tunnel.
    pub custom_address: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, port: crate::app_paths::pwa_port_for(crate::app_paths::profile().as_deref()), custom_address: None }
    }
}

pub fn path(data: &Path) -> PathBuf {
    data.join("phone").join("config.json")
}

/// A missing or damaged file is the default.
pub fn load(data: &Path) -> Config {
    let mut config: Config = std::fs::read_to_string(path(data)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // A dev build's window loads Vite on 1420. Both can bind it on Windows (0.0.0.0 and ::), and the phone server then
    // answers the window with the phone page, so a dev build keeps the phone server off that port.
    if cfg!(all(debug_assertions, not(test))) && config.port == DEV_VITE_PORT {
        config.port = DEV_VITE_PORT + 1;
    }
    config
}

const DEV_VITE_PORT: u16 = 1420;

pub fn save(data: &Path, config: &Config) -> Result<(), String> {
    let p = path(data);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("PHONE_SAVE: could not save the phone setting: {e}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    crate::secure_file::write_private(&p, &text).map_err(|e| format!("PHONE_SAVE: could not save the phone setting: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_damaged_files_are_the_default_and_a_save_round_trips() {
        let dir = std::env::temp_dir().join(format!("pm-phone-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), Config::default());
        let c = Config { enabled: false, port: 8123, custom_address: Some("https://bot.example.com".into()) };
        save(&dir, &c).unwrap();
        assert_eq!(load(&dir), c);
        std::fs::write(path(&dir), "{ nope").unwrap();
        assert_eq!(load(&dir), Config::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
