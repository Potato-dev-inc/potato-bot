//! Live CPU and memory limits for the sandbox's desktop work (the host's side).
//!
//! The limits are cgroup v2 settings inside the guest (`guest/control/pm-sbx-limits`, run as root), so a change applies at
//! once with no restart of the sandbox and no `wsl --shutdown`. They are saved by the app, applied again at every start, and
//! read back (with what the work uses now) for the Settings page.

use serde::{Deserialize, Serialize};

/// The default share of the PC's processors the sandbox's work may use.
pub const DEFAULT_CPU_PERCENT: u32 = 50;
pub const MIN_CPU_PERCENT: u32 = 10;
pub const MIN_MEMORY_MB: u32 = 1024;
/// The default memory is a quarter of the PC's, but never less than this (or half the PC on a small one).
pub const DEFAULT_MEMORY_FLOOR_MB: u32 = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub cpu_percent: u32,
    pub memory_mb: u32,
}

/// What suits an ordinary PC: half the processors, a quarter of the memory with a 2 GB floor.
pub fn default_limits(pc_memory_mb: u64) -> Limits {
    Limits { cpu_percent: DEFAULT_CPU_PERCENT, memory_mb: default_memory_mb(pc_memory_mb) }
}

pub fn default_memory_mb(pc_memory_mb: u64) -> u32 {
    let quarter = (pc_memory_mb / 4) as u32;
    let wanted = quarter.max(DEFAULT_MEMORY_FLOOR_MB);
    // On a small PC the floor would be most of the memory: take half of it at most.
    let half = (pc_memory_mb / 2) as u32;
    if pc_memory_mb == 0 {
        wanted
    } else {
        wanted.min(half.max(MIN_MEMORY_MB))
    }
}

/// Keeps a pair inside what the PC can give: 10 to 100 percent, 1 GB up to nine tenths of the memory.
pub fn clamp(limits: Limits, pc_memory_mb: u64) -> Limits {
    let top = if pc_memory_mb == 0 { u32::MAX } else { ((pc_memory_mb * 9 / 10) as u32).max(MIN_MEMORY_MB) };
    Limits { cpu_percent: limits.cpu_percent.clamp(MIN_CPU_PERCENT, 100), memory_mb: limits.memory_mb.clamp(MIN_MEMORY_MB, top) }
}

/// What `pm-sbx-limits status` / `apply` printed.
#[derive(Clone, Debug, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct GuestStatus {
    pub ok: bool,
    pub ready: bool,
    pub cpus: u32,
    pub pc_memory_mb: u64,
    pub cpu_percent: Option<u32>,
    pub memory_mb: Option<u32>,
    pub memory_ceiling_mb: Option<u32>,
    pub cpu_usage_usec: u64,
    pub cpu_throttled_usec: u64,
    pub memory_used_mb: u64,
    pub memory_high_events: u64,
    pub memory_max_events: u64,
    pub oom_kills: u64,
    pub processes: u32,
    pub now_ns: u64,
}

pub fn parse_status(text: &str) -> Option<GuestStatus> {
    let line = text.lines().rev().find(|l| l.trim_start().starts_with('{'))?;
    let status: GuestStatus = serde_json::from_str(line.trim()).ok()?;
    status.ok.then_some(status)
}

/// The work's CPU use between two readings, in percent of the whole PC (all processors = 100).
pub fn cpu_use_percent(earlier: &GuestStatus, later: &GuestStatus) -> Option<f64> {
    let wall_ns = later.now_ns.checked_sub(earlier.now_ns).filter(|n| *n > 0)?;
    let used_us = later.cpu_usage_usec.checked_sub(earlier.cpu_usage_usec)?;
    let cpus = later.cpus.max(1) as f64;
    Some((used_us as f64 * 1000.0 / wall_ns as f64 / cpus * 100.0).clamp(0.0, 100.0))
}

/// The PC's physical memory in MB (0 when Windows does not say).
#[cfg(windows)]
pub fn pc_memory_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a valid MEMORYSTATUSEX with its length set, as the call requires.
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() {
        status.ullTotalPhys / (1024 * 1024)
    } else {
        0
    }
}

#[cfg(not(windows))]
pub fn pc_memory_mb() -> u64 {
    0
}

/// The argument list for the guest script (absolute interpreter, the script as root's own file).
pub fn apply_argv(limits: Limits) -> Vec<String> {
    vec!["/usr/bin/python3".into(), "-I".into(), SCRIPT.into(), "apply".into(), limits.cpu_percent.to_string(), limits.memory_mb.to_string()]
}

pub fn status_argv() -> Vec<String> {
    vec!["/usr/bin/python3".into(), "-I".into(), SCRIPT.into(), "status".into()]
}

pub fn reclaim_argv() -> Vec<String> {
    vec!["/usr/bin/python3".into(), "-I".into(), SCRIPT.into(), "reclaim".into()]
}

pub const SCRIPT: &str = "/usr/local/sbin/pm-sbx-limits";

/// What the Settings page shows: the limits in force, the defaults, and (while the sandbox runs) what it uses.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsView {
    pub cpu_percent: u32,
    pub memory_mb: u32,
    pub default_cpu_percent: u32,
    pub default_memory_mb: u32,
    pub pc_memory_mb: u64,
    pub max_memory_mb: u32,
    pub running: bool,
    pub used_cpu_percent: Option<f64>,
    pub used_memory_mb: Option<u64>,
    /// The work is at its memory limit, so the kernel is slowing and reclaiming it.
    pub memory_pressure: bool,
}

pub fn build_view(saved: Option<Limits>, pc_mb: u64, now: Option<&GuestStatus>, used_cpu: Option<f64>) -> LimitsView {
    let defaults = default_limits(pc_mb);
    let current = clamp(saved.unwrap_or(defaults), pc_mb);
    let max = clamp(Limits { cpu_percent: 100, memory_mb: u32::MAX }, pc_mb).memory_mb;
    let used_memory = now.map(|s| s.memory_used_mb);
    LimitsView {
        cpu_percent: current.cpu_percent,
        memory_mb: current.memory_mb,
        default_cpu_percent: defaults.cpu_percent,
        default_memory_mb: defaults.memory_mb,
        pc_memory_mb: pc_mb,
        max_memory_mb: max,
        running: now.is_some(),
        used_cpu_percent: used_cpu.map(|p| (p * 10.0).round() / 10.0),
        used_memory_mb: used_memory,
        memory_pressure: used_memory.is_some_and(|u| u as f64 >= current.memory_mb as f64 * 0.95),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_half_the_cpu_and_a_quarter_of_the_memory_with_a_2_gb_floor() {
        assert_eq!(default_limits(32_000), Limits { cpu_percent: 50, memory_mb: 8_000 });
        assert_eq!(default_limits(16_384), Limits { cpu_percent: 50, memory_mb: 4_096 });
        assert_eq!(default_limits(8_192).memory_mb, 2_048);
        // a small PC: the floor would be most of it, so half
        assert_eq!(default_limits(3_000).memory_mb, 1_500);
        assert_eq!(default_limits(0).memory_mb, 2_048);
    }

    #[test]
    fn a_pair_is_kept_inside_what_the_pc_can_give() {
        let c = clamp(Limits { cpu_percent: 0, memory_mb: 1 }, 16_000);
        assert_eq!(c, Limits { cpu_percent: 10, memory_mb: 1_024 });
        let c = clamp(Limits { cpu_percent: 500, memory_mb: 99_999 }, 16_000);
        assert_eq!(c, Limits { cpu_percent: 100, memory_mb: 14_400 });
    }

    #[test]
    fn the_guest_answer_is_read_from_its_last_json_line() {
        let text = "noise\n{\"ok\": true, \"ready\": true, \"cpus\": 8, \"cpu_percent\": 50, \"memory_mb\": 2048, \"memory_used_mb\": 700, \"cpu_usage_usec\": 5000000, \"now_ns\": 9000000000}\n";
        let s = parse_status(text).unwrap();
        assert_eq!((s.cpus, s.cpu_percent, s.memory_mb, s.memory_used_mb), (8, Some(50), Some(2048), 700));
        assert!(parse_status("{\"ok\": false, \"error\": \"x\"}").is_none());
        assert!(parse_status("not json").is_none());
    }

    #[test]
    fn cpu_use_is_the_share_of_the_whole_pc_between_two_readings() {
        let a = GuestStatus { cpus: 8, cpu_usage_usec: 0, now_ns: 0, ok: true, ..Default::default() };
        // 4 cores' worth of work over 2 seconds on 8 cpus = 50% of the PC
        let b = GuestStatus { cpus: 8, cpu_usage_usec: 8_000_000, now_ns: 2_000_000_000, ok: true, ..Default::default() };
        let used = cpu_use_percent(&a, &b).unwrap();
        assert!((used - 50.0).abs() < 0.01, "{used}");
        assert!(cpu_use_percent(&b, &a).is_none(), "time going backwards gives no number");
    }

    #[test]
    fn the_page_view_has_defaults_when_nothing_is_saved_and_the_use_while_running() {
        let v = build_view(None, 16_384, None, None);
        assert_eq!((v.cpu_percent, v.memory_mb, v.running), (50, 4_096, false));
        assert_eq!(v.max_memory_mb, 14_745);
        assert!(v.used_memory_mb.is_none());
        let now = GuestStatus { ok: true, memory_used_mb: 4_000, ..Default::default() };
        let v = build_view(Some(Limits { cpu_percent: 30, memory_mb: 4_096 }), 16_384, Some(&now), Some(12.34));
        assert_eq!((v.cpu_percent, v.used_cpu_percent, v.used_memory_mb, v.running), (30, Some(12.3), Some(4_000), true));
        assert!(v.memory_pressure, "4000 of 4096 is at the limit");
        // a saved pair that no longer fits the PC is brought inside it
        let v = build_view(Some(Limits { cpu_percent: 500, memory_mb: 500_000 }), 8_192, None, None);
        assert_eq!((v.cpu_percent, v.memory_mb), (100, 7_372));
    }

    #[test]
    fn the_guest_commands_name_the_root_script_and_nothing_pm_can_edit() {
        let argv = apply_argv(Limits { cpu_percent: 25, memory_mb: 3000 });
        assert_eq!(argv, ["/usr/bin/python3", "-I", "/usr/local/sbin/pm-sbx-limits", "apply", "25", "3000"]);
        assert_eq!(status_argv().last().unwrap(), "status");
        assert_eq!(reclaim_argv().last().unwrap(), "reclaim");
    }
}
