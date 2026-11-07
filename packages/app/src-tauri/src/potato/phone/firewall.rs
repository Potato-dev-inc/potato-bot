//! The Windows Firewall rule that lets phones on the home network reach the phone server.
//!
//! Windows usually asks once when a program starts listening; if the person pressed Cancel, or the network was
//! marked Public, phones are blocked and nothing in the app says why. Here the app reads the rule and says so. It
//! never changes the firewall by itself: "Allow" starts an elevated command, so Windows asks the person to
//! confirm, and the rule it adds is for private networks and this one port only.

use serde::Serialize;

pub const RULE_NAME: &str = "Potato Bot phone";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ok,
    Missing,
    Unknown,
    #[cfg_attr(windows, allow(dead_code))]
    NotWindows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub state: State,
    pub message: String,
}

pub fn message(state: State, port: u16) -> String {
    match state {
        State::Ok => "Windows Firewall lets phones on your home network in.".to_string(),
        State::Missing => format!("Windows Firewall may be blocking your phone. Press Allow to add a rule for private networks and port {port} only. Windows will ask you to confirm."),
        State::Unknown => "The app could not check Windows Firewall.".to_string(),
        State::NotWindows => "This computer does not use Windows Firewall.".to_string(),
    }
}

/// `netsh ... show rule` succeeded (the rule exists) and names this port. The labels are translated on other
/// Windows languages, so only the exit code and the port number are read.
pub fn state_from_show(exit_ok: bool, output: &str, port: u16) -> State {
    if !exit_ok {
        return State::Missing;
    }
    let want = port.to_string();
    let named = output.lines().any(|line| line.split(|c: char| !c.is_ascii_digit()).any(|t| t == want));
    if named {
        State::Ok
    } else {
        State::Missing
    }
}

/// The netsh text run by the elevated shell: the old rule goes (the port may have changed), the new one is added
/// for the private profile and one TCP port.
pub fn netsh_script(port: u16) -> String {
    format!(
        "netsh advfirewall firewall delete rule name=\"{RULE_NAME}\"; netsh advfirewall firewall add rule name=\"{RULE_NAME}\" dir=in action=allow protocol=TCP localport={port} profile=private"
    )
}

/// PowerShell arguments that run `netsh_script` with the Windows administrator prompt.
pub fn elevated_args(port: u16) -> Vec<String> {
    let inner = netsh_script(port).replace('\'', "''");
    vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        format!("Start-Process -FilePath powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-Command','{inner}'"),
    ]
}

pub fn status(port: u16) -> Status {
    #[cfg(windows)]
    {
        let out = crate::proc_util::hidden_command("netsh").args(["advfirewall", "firewall", "show", "rule", &format!("name={RULE_NAME}")]).output();
        let state = match out {
            Ok(o) => state_from_show(o.status.success(), &String::from_utf8_lossy(&o.stdout), port),
            Err(_) => State::Unknown,
        };
        Status { state, message: message(state, port) }
    }
    #[cfg(not(windows))]
    {
        Status { state: State::NotWindows, message: message(State::NotWindows, port) }
    }
}

/// Opens the Windows administrator prompt. The person presses Yes; if they press No nothing changes.
pub fn allow(port: u16) -> Result<(), String> {
    #[cfg(windows)]
    {
        let out = crate::proc_util::hidden_command("powershell").args(elevated_args(port)).output().map_err(|_| "PHONE_FIREWALL: Windows would not open the confirm window.".to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err("PHONE_FIREWALL: The rule was not added. If you pressed No, nothing changed.".to_string())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = port;
        Err("PHONE_FIREWALL: This computer does not use Windows Firewall.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_read_by_exit_code_and_port_number() {
        assert_eq!(state_from_show(false, "No rules match the specified criteria.", 1420), State::Missing);
        assert_eq!(state_from_show(true, "LocalPort:  1420\nAction: Allow", 1420), State::Ok);
        assert_eq!(state_from_show(true, "本機連接埠:  1420", 1420), State::Ok);
        assert_eq!(state_from_show(true, "LocalPort:  8123", 1420), State::Missing, "a rule for another port does not count");
        assert_eq!(state_from_show(true, "LocalPort:  14200", 1420), State::Missing, "a number inside another number does not count");
    }

    #[test]
    fn the_rule_is_private_networks_and_one_port_only_and_asks_windows() {
        let script = netsh_script(8123);
        assert!(script.contains("profile=private") && script.contains("localport=8123") && script.contains("dir=in") && script.contains("protocol=TCP"));
        assert!(!script.contains("profile=public") && !script.contains("profile=any") && !script.contains("localport=any"));
        let args = elevated_args(8123).join(" ");
        assert!(args.contains("-Verb RunAs"), "Windows must ask: {args}");
    }

    #[test]
    fn the_missing_message_says_what_happened_and_what_pressing_does() {
        let m = message(State::Missing, 1420);
        assert!(m.contains("blocking") && m.contains("private networks") && m.contains("1420") && m.contains("confirm"));
    }
}
