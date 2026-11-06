//! Desktop notifications for the bot. A tray balloon on Windows, built from a hidden PowerShell
//! process so no window flashes. One reply notification per bot per minute.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const MIN_GAP: Duration = Duration::from_secs(60);

fn last_shown() -> &'static Mutex<HashMap<String, Instant>> {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A bot replied. `enabled` is its "When it replies" choice.
pub fn bot_reply(bot_id: &str, name: &str, text: &str, enabled: bool) {
    if !enabled {
        return;
    }
    {
        let mut last = last_shown().lock();
        let now = Instant::now();
        if last.get(bot_id).is_some_and(|at| now.duration_since(*at) < MIN_GAP) {
            return;
        }
        last.insert(bot_id.to_string(), now);
    }
    let body = format!("{}", crate::redact::text(text));
    show(&format!("{name} replied"), &body);
}

fn quote(text: &str) -> String {
    text.replace('\'', "''")
}

/// Show one tray balloon. Silent on other platforms and when PowerShell cannot start.
pub fn show(title: &str, body: &str) {
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $n = New-Object System.Windows.Forms.NotifyIcon; \
             $n.Icon = [System.Drawing.SystemIcons]::Information; \
             $n.Visible = $true; \
             $n.ShowBalloonTip(6000, '{}', '{}', [System.Windows.Forms.ToolTipIcon]::Info); \
             Start-Sleep -Seconds 7; $n.Dispose()",
            quote(title),
            quote(body)
        );
        let _ = crate::proc_util::hidden_command("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body);
    }
}
