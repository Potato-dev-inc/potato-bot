//! Making the sandbox: the steps, the guest files, and the progress the person sees.
//!
//! The steps (`plan`):
//!
//! 1. `CheckWsl`: Windows' Linux feature answers, and the virtual machine can list distros. If not, the
//!    install stops with `NeedsWindowsFeature` ("Needs Windows' Linux feature (admin + restart)"). This code
//!    never turns the feature on: that is `Wsl::enable_windows_feature`, which needs a `Consent` made on the
//!    person's button press.
//! 2. `CheckFree`: our name is not already taken. A distro list that cannot answer is the feature, not a step.
//! 3. `CreateDistro`: `wsl --install Ubuntu-24.04 --name <n> --location <dir> --no-launch`, with `<dir>`
//!    under `<app data>/potato/sandbox/<n>/` (`Layout`), never the repo. On a PC where the feature was just
//!    turned on, that call can answer success after only enabling Windows' Linux feature: no distro is
//!    registered until the restart. The distro is checked for, and a missing one stops the install with
//!    `NeedsRestart` instead of letting the next step fail with "no distribution with the supplied name".
//! 4. `CopyGuestFiles`: the guest files go in over stdin as a tar stream (`GuestFiles::tar_stream`) and are
//!    unpacked by `tar` inside the distro. Nothing is copied through a Windows folder. Then the short install-time
//!    `/etc/wsl.conf` (systemd off) is put in place and the distro is restarted once, so the installer runs without
//!    systemd (a fresh Ubuntu starts snapd, unattended-upgrades and PackageKit, which compete with apt).
//! 5. `ApplyRecipes`: `apply-recipe.py` runs as root; its `PROGRESS n/total text` lines become progress
//!    events (`parse_progress_line`; `total` is 1000 and `n` the share of the whole job done, so the bar moves while
//!    packages download), its `ERROR text` line becomes the failure reason.
//! 6. `Restart`: `wsl --terminate <n>` so `/etc/wsl.conf` takes effect.
//! 7. `BootCheck`: a real start with every readiness check (`Lifecycle::ensure_ready`), then a stop.
//! 8. `Finish`: the completion marker `/etc/pm-sbx/installed` is written in the guest (before the reset point, so a
//!    restored reset point is complete too).
//! 9. `Snapshot` (optional, on by default): `wsl --export` to the reset point.
//!
//! A distro of ours without the finished record (`InstallRecords`, kept by the host app; `SavedInstall`) is "Setup didn't
//! finish": it was cut off (the app quit, the PC slept, a crash). `InstallOptions::resume` runs the plan again against the
//! distro that is there (no download, no name check), with the options saved when it started; every step is safe to repeat
//! (`apply-recipe.py` skips what it finished). A cut-off set up is never taken away by the app quitting.
//!
//! A cancel after the distro exists unregisters that distro again (it is ours, made a minute ago), so a retry
//! starts clean. A failure after the distro exists keeps it, with the record unfinished, so "Continue setup"
//! resumes. No secret is made or handled here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::image::{self, ImageError};
use super::lifecycle::{Lifecycle, GUEST_SHARE};
use parking_lot::Mutex;
use super::wsl::{GuestUser, SandboxName, Wsl, WslError};

const APPLY_TIMEOUT: Duration = Duration::from_secs(90 * 60);
const COPY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PROGRESS_TEXT_CHARS: usize = 200;
/// The short `/etc/wsl.conf` for the install (a file of the guest archive).
const INSTALL_CONF: &str = "/usr/local/share/pm-sbx/desktop/wsl-install.conf";
/// What the person is told the Ubuntu download weighs (`wsl --install` prints no percentage).
const UBUNTU_DOWNLOAD_TEXT: &str = "about 340 MB";
const MAX_OPTION_VALUE: usize = 200;
const ADDONS: [&str; 2] = ["coding", "daily"];
/// The default reset point.
pub const BASE_SNAPSHOT: &str = "base";

// ---- where things live ---------------------------------------------------------------------------

/// `<app data>/potato/sandbox/`: every distro folder and every reset point is under it.
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// `potato_root` is the `<app data>/potato` folder.
    pub fn new(potato_root: &Path) -> Self {
        Self { root: potato_root.join("sandbox") }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the distro's disk lives: `<root>/<name>/`.
    pub fn distro_dir(&self, name: &SandboxName) -> PathBuf {
        self.root.join(name.as_str())
    }

    pub fn snapshots_dir(&self, name: &SandboxName) -> PathBuf {
        self.root.join("snapshots").join(name.as_str())
    }

    /// `snapshot` is `[a-z0-9-]{1,30}`; anything else is `None`.
    pub fn snapshot_path(&self, name: &SandboxName, snapshot: &str) -> Option<PathBuf> {
        valid_snapshot_id(snapshot).then(|| self.snapshots_dir(name).join(format!("{snapshot}.tar")))
    }
}

pub fn valid_snapshot_id(id: &str) -> bool {
    (1..=30).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

// ---- the guest files -----------------------------------------------------------------------------

/// The folder with the guest side: `guest/{desktop,control,egress,page-browser}/` and `recipes/` (the
/// shape of `packages/potato-sandbox/`). The app finds it in its resources.
#[derive(Clone, Debug)]
pub struct GuestFiles {
    root: PathBuf,
}

const GUEST_FOLDERS: [&str; 4] = ["desktop", "control", "egress", "page-browser"];

/// Paths under the guest folder that an install and a start run (the installer, the walls and their probe, the
/// desktop starter, the control service, the base recipe).
const REQUIRED_FILES: [&str; 9] = [
    "guest/desktop/apply-recipe.py",
    "guest/desktop/pm-sbx-start-desktop.sh",
    "guest/egress/pm-sbx-egress.sh",
    "guest/egress/pm-sbx-egress-probe.sh",
    "guest/control/pm-sbx-control",
    "guest/control/pm-sbx-cli",
    "guest/control/pm-sbx-computer-mcp",
    "guest/control/pm-sbx-mcp-hub",
    "recipes/base.json",
];

fn shipped(file_name: &str) -> bool {
    !(file_name.ends_with(".md") || file_name.starts_with("test_") || file_name.ends_with(".pyc"))
}

impl GuestFiles {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    /// Every file to ship: (path inside the tar, path on disk), sorted.
    pub fn list(&self) -> std::io::Result<Vec<(String, PathBuf)>> {
        let mut files = Vec::new();
        for folder in GUEST_FOLDERS {
            collect(&self.root.join("guest").join(folder), folder, &mut files)?;
        }
        collect(&self.root.join("recipes"), "recipes", &mut files)?;
        files.sort();
        Ok(files)
    }

    /// The files without which an install or a start cannot work must all be there.
    pub fn check(&self) -> Result<(), InstallError> {
        for rel in REQUIRED_FILES {
            if !self.root.join(rel).is_file() {
                return Err(InstallError::GuestFilesMissing);
            }
        }
        Ok(())
    }

    /// A short, stable-in-this-build stamp of the guest files. A sandbox carries the stamp it was last
    /// updated with, so a start can refresh the guest executables it was made before (a new `pm-sbx-cli`,
    /// for one) without a full reset.
    pub fn version(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Ok(files) = self.list() {
            for (name, path) in files {
                name.hash(&mut hasher);
                if let Ok(bytes) = std::fs::read(path) {
                    strip_cr(&bytes).hash(&mut hasher);
                }
            }
        }
        format!("{:016x}", hasher.finish())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The sha256 (lowercase hex) of the guest tar stream: the digest a prebuilt image is made for.
    pub fn digest(&self) -> std::io::Result<String> {
        let bytes = self.tar_stream()?;
        Ok(format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes)))
    }

    /// A ustar archive of `list()`, with Windows line endings taken out of every file.
    pub fn tar_stream(&self) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::new();
        for (name, path) in self.list()? {
            let bytes = std::fs::read(&path)?;
            write_tar_entry(&mut out, &name, &strip_cr(&bytes))?;
        }
        out.extend_from_slice(&[0u8; 1024]);
        Ok(out)
    }
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if path.is_dir() {
            collect(&path, &format!("{prefix}/{name}"), out)?;
        } else if shipped(&name) {
            out.push((format!("{prefix}/{name}"), path));
        }
    }
    Ok(())
}

/// `\r\n` becomes `\n`: a checkout with Windows line endings would break `#!/bin/bash` scripts.
pub fn strip_cr(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            continue;
        }
        out.push(*b);
    }
    out
}

fn octal(field: &mut [u8], value: u64) {
    let width = field.len() - 1;
    let text = format!("{value:0width$o}");
    field[..width].copy_from_slice(text.as_bytes());
    field[width] = 0;
}

/// One regular file as a ustar header and padded data.
pub fn write_tar_entry(out: &mut Vec<u8>, name: &str, data: &[u8]) -> std::io::Result<()> {
    if name.is_empty() || name.len() > 100 || !name.is_ascii() || name.starts_with('/') || name.contains("..") {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad name in the guest archive"));
    }
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    let executable = !(name.ends_with(".json") || name.ends_with(".conf") || name.ends_with(".desktop"));
    octal(&mut header[100..108], if executable { 0o755 } else { 0o644 });
    octal(&mut header[108..116], 0);
    octal(&mut header[116..124], 0);
    octal(&mut header[124..136], data.len() as u64);
    octal(&mut header[136..148], 0);
    header[148..156].copy_from_slice(b"        ");
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
    let check = format!("{sum:06o}");
    header[148..154].copy_from_slice(check.as_bytes());
    header[154] = 0;
    header[155] = b' ';
    out.extend_from_slice(&header);
    out.extend_from_slice(data);
    let pad = (512 - data.len() % 512) % 512;
    out.extend(std::iter::repeat(0u8).take(pad));
    Ok(())
}

// ---- the options ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    /// The light desktop (fluxbox).
    Light,
    /// XFCE, the plain Ubuntu look.
    Xfce,
    /// XFCE with the Zorin look (themes, icons, wallpaper and font from the Zorin PPA). Not Zorin's own desktop.
    Zorin,
    /// Linux Mint's Cinnamon desktop (Mint's own packages are not on Ubuntu's archive; Cinnamon is).
    Mint,
}

impl Desktop {
    /// The desktop recipe (the one with a session).
    pub fn recipe_id(self) -> &'static str {
        match self {
            Desktop::Light => "light",
            Desktop::Xfce | Desktop::Zorin => "xfce",
            Desktop::Mint => "mint",
        }
    }

    /// Every recipe this choice installs, desktop first (the look needs XFCE already there).
    pub fn recipe_ids(self) -> Vec<&'static str> {
        match self {
            Desktop::Zorin => vec!["xfce", "zorin-look"],
            other => vec![other.recipe_id()],
        }
    }

    /// The session `/etc/pm-sbx/desktop.conf` names (what `pm-sbx-start-desktop` runs).
    pub fn session(self) -> &'static str {
        match self {
            Desktop::Light => "fluxbox",
            Desktop::Xfce | Desktop::Zorin => "xfce4",
            Desktop::Mint => "cinnamon",
        }
    }

    /// The wizard's word for it (the other way round from `parse`).
    pub fn word(self) -> &'static str {
        match self {
            Desktop::Light => "light",
            Desktop::Xfce => "ubuntu",
            Desktop::Zorin => "zorin",
            Desktop::Mint => "mint",
        }
    }

    /// The wizard's word for it: `light`, `ubuntu`, `zorin` or `mint`.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "light" => Some(Desktop::Light),
            "ubuntu" => Some(Desktop::Xfce),
            "zorin" => Some(Desktop::Zorin),
            "mint" => Some(Desktop::Mint),
            _ => None,
        }
    }
}

/// The wizard's answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallOptions {
    pub name: SandboxName,
    /// The desktop the sandbox starts in.
    pub desktop: Desktop,
    /// Other desktops already installed in this sandbox (the person can switch to them later).
    pub others: Vec<Desktop>,
    /// `coding` and/or `daily`.
    pub addons: Vec<String>,
    /// `(recipe.option, value)`, for example `("coding.pkg", "pnpm")`.
    pub options: Vec<(String, String)>,
    /// Save a reset point at the end.
    pub snapshot: bool,
    /// Carry on with a set up that was cut off: the distro is already there, so no name check and no download.
    pub resume: bool,
}

impl InstallOptions {
    pub fn new(name: SandboxName, desktop: Desktop) -> Self {
        Self { name, desktop, others: Vec::new(), addons: Vec::new(), options: Vec::new(), snapshot: true, resume: false }
    }

    /// Every desktop this sandbox has installed, the active one first.
    pub fn desktops(&self) -> Vec<Desktop> {
        let mut all = vec![self.desktop];
        all.extend(self.others.iter().copied().filter(|d| *d != self.desktop));
        all
    }

    /// Every recipe the install applies. Chrome is always one of them: the prebuilt image leaves it out, so the
    /// ApplyRecipes step (the same on both the image plan and the usual plan) installs it on the user's PC.
    fn chosen(&self) -> Vec<&str> {
        let mut ids = self.desktop.recipe_ids();
        ids.push("chrome");
        ids.extend(self.addons.iter().map(String::as_str));
        ids
    }

    pub fn validate(&self) -> Result<(), InstallError> {
        let bad = |why: &'static str| Err(InstallError::BadOptions(why));
        if self.others.contains(&self.desktop) || self.others.iter().enumerate().any(|(i, d)| self.others[..i].contains(d)) {
            return bad("a desktop is listed twice");
        }
        for (i, addon) in self.addons.iter().enumerate() {
            if !ADDONS.contains(&addon.as_str()) {
                return bad("an add-on is coding or daily");
            }
            if self.addons[..i].contains(addon) {
                return bad("an add-on is listed twice");
            }
        }
        let chosen = self.chosen();
        for (key, value) in &self.options {
            let Some((recipe, option)) = key.split_once('.') else { return bad("an option is recipe.option") };
            let word = |w: &str| {
                let mut chars = w.chars();
                (1..=48).contains(&w.len())
                    && chars.next().is_some_and(|c| c.is_ascii_lowercase())
                    && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            };
            if !word(recipe) || !word(option) {
                return bad("an option name is lowercase words");
            }
            if !chosen.contains(&recipe) {
                return bad("an option belongs to a recipe that is not chosen");
            }
            if value.len() > MAX_OPTION_VALUE
                || !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b',' | b'.' | b'_' | b'+' | b'-'))
            {
                return bad("an option value is lowercase letters, digits and , . _ + -");
            }
        }
        Ok(())
    }

    /// The command line for `apply-recipe.py` (`base` is always installed by the script itself).
    pub fn recipe_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for id in self.chosen() {
            args.push("--recipe".to_string());
            args.push(id.to_string());
        }
        for (key, value) in &self.options {
            args.push("--opt".to_string());
            args.push(format!("{key}={value}"));
        }
        args
    }
}

// ---- the record of a set up ----------------------------------------------------------------------

/// Bumped when the guest marker or the record changes shape.
pub const INSTALL_VERSION: u32 = 1;
/// The completion marker in the guest, written by the `Finish` step.
pub const GUEST_MARKER: &str = "/etc/pm-sbx/installed";

/// What the host keeps about a set up: whether it ended, and the answers it started with (so it can be carried on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedInstall {
    pub done: bool,
    /// `None` for a sandbox made before records existed (its answers are not known).
    pub options: Option<InstallOptions>,
}

impl SavedInstall {
    pub fn started(options: &InstallOptions) -> Self {
        Self { done: false, options: Some(options.clone()) }
    }

    pub fn finished(options: Option<&InstallOptions>) -> Self {
        Self { done: true, options: options.cloned() }
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::json!({ "version": INSTALL_VERSION, "done": self.done });
        if let Some(o) = &self.options {
            value["desktop"] = o.desktop.word().into();
            value["others"] = o.others.iter().map(|d| d.word()).collect::<Vec<_>>().into();
            value["addons"] = o.addons.clone().into();
            value["options"] = o.options.iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>().into();
            value["snapshot"] = o.snapshot.into();
        }
        value
    }

    /// `None` when the value is not a record this version understands.
    pub fn from_json(name: &SandboxName, value: &serde_json::Value) -> Option<Self> {
        let done = value.get("done")?.as_bool()?;
        let options = match value.get("desktop").and_then(|d| d.as_str()) {
            None => None,
            Some(word) => {
                let mut o = InstallOptions::new(name.clone(), Desktop::parse(word)?);
                o.others = value
                    .get("others")
                    .and_then(|v| v.as_array())
                    .map(|list| list.iter().filter_map(|v| v.as_str().and_then(Desktop::parse)).collect())
                    .unwrap_or_default();
                o.addons = value.get("addons")?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
                for pair in value.get("options")?.as_array()? {
                    let pair = pair.as_array()?;
                    o.options.push((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()));
                }
                o.snapshot = value.get("snapshot")?.as_bool()?;
                o.validate().ok()?;
                Some(o)
            }
        };
        Some(Self { done, options })
    }
}

/// Where the record lives. This folder writes no file on Windows, so the host app provides the store (a file under its
/// app data); tests use `MemoryRecords`.
pub trait InstallRecords: Send + Sync {
    /// False for "no records are kept": a sandbox is then taken as finished, whatever is in it.
    fn keeps_records(&self) -> bool {
        true
    }
    fn load(&self, name: &SandboxName) -> Option<SavedInstall>;
    fn save(&self, name: &SandboxName, saved: &SavedInstall);
    fn clear(&self, name: &SandboxName);
}

/// Keeps nothing (the default of a `WslSandbox` nobody gave a store).
pub struct NoRecords;

impl InstallRecords for NoRecords {
    fn keeps_records(&self) -> bool {
        false
    }
    fn load(&self, _name: &SandboxName) -> Option<SavedInstall> {
        None
    }
    fn save(&self, _name: &SandboxName, _saved: &SavedInstall) {}
    fn clear(&self, _name: &SandboxName) {}
}

/// Keeps the record in memory.
#[derive(Default)]
pub struct MemoryRecords {
    map: Mutex<std::collections::HashMap<String, SavedInstall>>,
}

impl InstallRecords for MemoryRecords {
    fn load(&self, name: &SandboxName) -> Option<SavedInstall> {
        self.map.lock().get(name.as_str()).cloned()
    }
    fn save(&self, name: &SandboxName, saved: &SavedInstall) {
        self.map.lock().insert(name.to_string(), saved.clone());
    }
    fn clear(&self, name: &SandboxName) {
        self.map.lock().remove(name.as_str());
    }
}

/// What the `Finish` step writes into the guest.
pub fn marker_text(options: &InstallOptions) -> String {
    format!("version={INSTALL_VERSION}\ndesktop={}\nrecipe_args={}\n", options.desktop.word(), options.recipe_args().join(" "))
}

// ---- steps and progress --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    CheckWsl,
    CheckFree,
    CreateDistro,
    CopyGuestFiles,
    ApplyRecipes,
    Restart,
    BootCheck,
    Finish,
    Snapshot,
    DownloadImage,
    ImportImage,
}

impl Step {
    pub fn id(self) -> &'static str {
        match self {
            Step::CheckWsl => "check_linux",
            Step::CheckFree => "check_free",
            Step::CreateDistro => "create",
            Step::CopyGuestFiles => "copy",
            Step::ApplyRecipes => "apply",
            Step::Restart => "restart",
            Step::BootCheck => "check_start",
            Step::Finish => "finish",
            Step::Snapshot => "snapshot",
            Step::DownloadImage => "download_image",
            Step::ImportImage => "import_image",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::CheckWsl => "Checking Windows' Linux feature",
            Step::CheckFree => "Checking the name is free",
            Step::CreateDistro => "Downloading Ubuntu",
            Step::CopyGuestFiles => "Copying the sandbox files",
            Step::ApplyRecipes => "Installing the desktop and tools",
            Step::Restart => "Restarting the sandbox",
            Step::BootCheck => "Checking it starts safely",
            Step::Finish => "Finishing up",
            Step::Snapshot => "Saving a reset point",
            Step::DownloadImage => "Downloading the sandbox image",
            Step::ImportImage => "Importing the sandbox image",
        }
    }

    /// Share of the whole install, for the percentage bar.
    pub fn weight(self) -> u32 {
        match self {
            Step::CheckWsl | Step::CheckFree | Step::Restart | Step::Finish => 1,
            Step::CopyGuestFiles => 3,
            Step::BootCheck => 5,
            Step::Snapshot => 10,
            Step::DownloadImage => 55,
            Step::ImportImage => 8,
            Step::CreateDistro => 25,
            Step::ApplyRecipes => 45,
        }
    }
}

pub fn plan(options: &InstallOptions) -> Vec<Step> {
    let mut steps = if options.resume {
        // The distro is there (it is the one that was cut off): nothing to check or download.
        vec![Step::CheckWsl]
    } else {
        vec![Step::CheckWsl, Step::CheckFree, Step::CreateDistro]
    };
    steps.extend([Step::CopyGuestFiles, Step::ApplyRecipes, Step::Restart, Step::BootCheck, Step::Finish]);
    if options.snapshot {
        steps.push(Step::Snapshot);
    }
    steps
}

/// What the person sees: a step name, a bar, and (while recipes run) the line the installer printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub step: &'static str,
    pub title: &'static str,
    /// 1-based position of the step.
    pub index: usize,
    pub of: usize,
    /// 0 to 100 over the whole install.
    pub percent: u8,
    pub detail: Option<String>,
    /// The real bytes (done, total) of a download, for a bar that counts bytes. None for the other steps.
    pub bytes: Option<(u64, u64)>,
}

pub fn percent_at(steps: &[Step], index: usize, fraction: f64) -> u8 {
    let total: u32 = steps.iter().map(|s| s.weight()).sum();
    let before: u32 = steps.iter().take(index).map(|s| s.weight()).sum();
    let within = steps.get(index).map(|s| f64::from(s.weight()) * fraction.clamp(0.0, 1.0)).unwrap_or(0.0);
    (((f64::from(before) + within) / f64::from(total.max(1))) * 100.0).round().clamp(0.0, 100.0) as u8
}

/// One `PROGRESS n/total text` line of `apply-recipe.py`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyLine {
    pub done: u32,
    pub total: u32,
    pub text: String,
}

pub fn parse_progress_line(line: &str) -> Option<ApplyLine> {
    let rest = line.trim().strip_prefix("PROGRESS ")?;
    let (counts, text) = rest.split_once(' ').unwrap_or((rest, ""));
    let (done, total) = counts.split_once('/')?;
    let all_digits = |s: &str| !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(done) || !all_digits(total) {
        return None;
    }
    let (done, total) = (done.parse::<u32>().ok()?, total.parse::<u32>().ok()?);
    if total == 0 || done > total {
        return None;
    }
    let text = crate::redact::text(text).into_inner().chars().filter(|c| !c.is_control()).take(PROGRESS_TEXT_CHARS).collect();
    Some(ApplyLine { done, total, text })
}

/// An `ERROR text` line of `apply-recipe.py`.
pub fn parse_error_line(line: &str) -> Option<String> {
    let text = line.trim().strip_prefix("ERROR ")?;
    Some(crate::redact::text(text).into_inner().chars().filter(|c| !c.is_control()).take(PROGRESS_TEXT_CHARS).collect())
}

// ---- errors --------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Windows' Linux feature is missing. Enabling it needs admin and a restart, and the person's button.
    NeedsWindowsFeature,
    /// Windows' Linux feature was turned on, but Windows has not restarted yet, so no distro is registered.
    NeedsRestart,
    /// A distro with this name is already there.
    AlreadyInstalled,
    BadOptions(&'static str),
    GuestFilesMissing,
    Cancelled,
    Step { step: &'static str, reason: String },
}

impl InstallError {
    pub fn code(&self) -> &'static str {
        match self {
            InstallError::NeedsWindowsFeature => "SANDBOX_NEEDS_LINUX_FEATURE",
            InstallError::NeedsRestart => "SANDBOX_NEEDS_RESTART",
            InstallError::AlreadyInstalled => "SANDBOX_ALREADY_INSTALLED",
            InstallError::BadOptions(_) => "SANDBOX_BAD_OPTIONS",
            InstallError::GuestFilesMissing => "SANDBOX_GUEST_FILES_MISSING",
            InstallError::Cancelled => "SANDBOX_INSTALL_CANCELLED",
            InstallError::Step { .. } => "SANDBOX_INSTALL_FAILED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            InstallError::NeedsWindowsFeature => "Needs Windows' Linux feature (admin + restart).".to_string(),
            InstallError::NeedsRestart => {
                "Restart your PC to finish, then come back.".to_string()
            }
            InstallError::AlreadyInstalled => "A sandbox with this name is already set up.".to_string(),
            InstallError::BadOptions(why) => format!("These choices cannot be used: {why}."),
            InstallError::GuestFilesMissing => "The sandbox files that ship with the app are missing.".to_string(),
            InstallError::Cancelled => "Set up was stopped.".to_string(),
            InstallError::Step { step, reason } => format!("Set up failed at \"{step}\": {reason}"),
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for InstallError {}

fn step_error(step: Step, error: impl std::fmt::Display) -> InstallError {
    let reason: String = crate::redact::text(&error.to_string()).into_inner().chars().take(200).collect();
    InstallError::Step { step: step.title(), reason }
}

/// What `apply-recipe.py` means by an exit code.
pub fn apply_exit_reason(code: Option<i32>) -> &'static str {
    match code {
        Some(1) => "a step failed",
        Some(2) => "the choices are not valid",
        Some(3) => "it did not run as root",
        Some(4) => "the desktop user has more rights than it may",
        _ => "it stopped unexpectedly",
    }
}

// ---- the run -------------------------------------------------------------------------------------

pub struct InstallEnv<'a> {
    pub wsl: &'a Wsl,
    pub layout: &'a Layout,
    pub files: &'a GuestFiles,
    pub lifecycle: &'a Lifecycle,
    pub records: &'a dyn InstallRecords,
}

fn emit(progress: &mut dyn FnMut(Progress), steps: &[Step], index: usize, fraction: f64, detail: Option<String>) {
    let step = steps[index];
    progress(Progress {
        step: step.id(),
        title: step.title(),
        index: index + 1,
        of: steps.len(),
        percent: percent_at(steps, index, fraction),
        detail,
        bytes: None,
    });
}

/// Runs the plan. `now_ms` is read once per start check. A cancel after the distro was made unregisters it again.
/// A failure after the distro was made keeps it, with the record unfinished (so "Continue setup" resumes it).
pub fn run(
    env: &InstallEnv<'_>,
    options: &InstallOptions,
    cancel: &AtomicBool,
    now_ms: &dyn Fn() -> i64,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), InstallError> {
    options.validate()?;
    env.files.check()?;
    let image = if options.resume { None } else { choose_image(env, options) };
    let steps = if image.is_some() { image_plan(options) } else { plan(options) };
    let mut created = false;
    // The steps run through a Wsl whose calls die when `kill` turns true; a watcher turns it true as soon as the
    // person cancels, so a running step (the download, the recipes) stops now, not at the next step. The clean-up
    // below uses `env.wsl`, which nothing cancels, and touches only our own distro.
    let kill = Arc::new(AtomicBool::new(false));
    let step_wsl = env.wsl.with_cancel(kill.clone());
    let step_env = InstallEnv { wsl: &step_wsl, layout: env.layout, files: env.files, lifecycle: env.lifecycle, records: env.records };
    let finished = AtomicBool::new(false);
    let mut outcome = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !finished.load(Ordering::SeqCst) {
                if cancel.load(Ordering::SeqCst) {
                    kill.store(true, Ordering::SeqCst);
                    // A boot check in the middle of a start ends with the Stop (it shares the lifecycle's start lock).
                    if matches!(env.lifecycle.phase(), super::lifecycle::Phase::Starting) {
                        let _ = env.lifecycle.stop();
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        let outcome = run_steps(&step_env, options, steps, image.as_ref(), cancel, now_ms, progress, &mut created);
        finished.store(true, Ordering::SeqCst);
        outcome
    });
    if outcome.is_err() && kill.load(Ordering::SeqCst) {
        // The step failed because we killed it: that is a cancel, not a failure.
        outcome = Err(InstallError::Cancelled);
    }
    if created && outcome.is_err() {
        // A cancel takes our new distro away, so a retry starts clean. Any other failure after the distro exists keeps
        // it and its record (still "not finished"), so "Continue setup" can carry on. Before the distro is registered
        // (the restart case) there is nothing to keep.
        let cancelled = matches!(outcome, Err(InstallError::Cancelled));
        let registered = matches!(env.wsl.is_installed(&options.name), Ok(true));
        if cancelled || !registered {
            let _ = env.lifecycle.stop();
            let _ = env.wsl.unregister(&options.name);
            env.records.clear(&options.name);
        }
    }
    outcome
}

/// The image the set up starts from, when there is one for this choice and these guest files.
fn choose_image(env: &InstallEnv<'_>, options: &InstallOptions) -> Option<(image::Manifest, image::ImageEntry)> {
    // Dev override (debug builds): the file from PM_SBX_IMAGE_FILE is used whatever its guest digest is, so a
    // first run with an image built from slightly older guest files still goes through the image path.
    if let Some((_, sha)) = image::dev_override_from_env() {
        let key = image::image_key_for(options.desktop)?;
        let manifest = image::load_manifest(env.files.root()).unwrap_or(image::Manifest { schema: 1, base_url: String::new(), images: std::collections::BTreeMap::new() });
        let entry = manifest.images.get(key).cloned().unwrap_or(image::ImageEntry { file: "dev-override.tar.zst".to_string(), size: 0, sha256: sha, guest_digest: String::new() });
        return Some((manifest, entry));
    }
    let manifest = image::load_manifest(env.files.root())?;
    let key = image::image_key_for(options.desktop)?;
    let entry = manifest.entry(key)?.clone();
    let guest = env.files.digest().ok()?;
    if entry.guest_digest != guest {
        return None;
    }
    Some((manifest, entry))
}

/// The steps for a set up that starts from the prebuilt image (no Restart and no Snapshot: the image is the reset point).
pub fn image_plan(_options: &InstallOptions) -> Vec<Step> {
    vec![Step::CheckWsl, Step::CheckFree, Step::DownloadImage, Step::ImportImage, Step::CopyGuestFiles, Step::ApplyRecipes, Step::BootCheck, Step::Finish]
}

/// What runs after an image that could not be used: the usual plan from CreateDistro on.
pub fn fallback_tail(options: &InstallOptions) -> Vec<Step> {
    let mut tail = vec![Step::CreateDistro, Step::CopyGuestFiles, Step::ApplyRecipes, Step::Restart, Step::BootCheck, Step::Finish];
    if options.snapshot {
        tail.push(Step::Snapshot);
    }
    tail
}

fn image_error(step: Step, error: ImageError) -> InstallError {
    match error {
        ImageError::Cancelled => InstallError::Cancelled,
        other => step_error(step, other),
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn run_steps(
    env: &InstallEnv<'_>,
    options: &InstallOptions,
    mut steps: Vec<Step>,
    image: Option<&(image::Manifest, image::ImageEntry)>,
    cancel: &AtomicBool,
    now_ms: &dyn Fn() -> i64,
    progress: &mut dyn FnMut(Progress),
    created: &mut bool,
) -> Result<(), InstallError> {
    let name = &options.name;
    let mut image_file: Option<PathBuf> = None;
    let mut index = 0;
    while index < steps.len() {
        let step = steps[index];
        if cancel.load(Ordering::SeqCst) {
            return Err(InstallError::Cancelled);
        }
        emit(progress, &steps, index, 0.0, None);
        if matches!(step, Step::DownloadImage | Step::ImportImage) {
            let result = image_step(env, options, step, &steps, index, image, &mut image_file, cancel, progress, created);
            match result {
                Ok(()) => index += 1,
                Err(InstallError::Cancelled) => return Err(InstallError::Cancelled),
                Err(_) => {
                    // The image is not used: take back a half-made import, and carry on the usual way from CreateDistro.
                    if step == Step::ImportImage && matches!(env.wsl.is_installed(name), Ok(true)) {
                        let _ = env.wsl.unregister(name);
                    }
                    image_file = None;
                    steps.truncate(index);
                    steps.extend(fallback_tail(options));
                }
            }
            continue;
        }
        let result = (|| -> Result<(), InstallError> {
            match step {
                Step::CheckWsl => {
                    match env.wsl.feature_state() {
                        super::wsl::FeatureState::Ready => {}
                        super::wsl::FeatureState::Missing => return Err(InstallError::NeedsWindowsFeature),
                        super::wsl::FeatureState::RestartPending => return Err(InstallError::NeedsRestart),
                    }
                }
                Step::CheckFree => match env.wsl.is_installed(name) {
                    Ok(true) => return Err(InstallError::AlreadyInstalled),
                    Ok(false) => {}
                    // `--status` can answer while the virtual machine cannot start yet (a restart is pending).
                    // That is the feature, not a broken step.
                    Err(_) => return Err(InstallError::NeedsWindowsFeature),
                },
                Step::CreateDistro => {
                    let dir = env.layout.distro_dir(name);
                    std::fs::create_dir_all(&dir).map_err(|e| step_error(step, e))?;
                    // From here on the distro is "set up not finished" until the last step says otherwise.
                    env.records.save(name, &SavedInstall::started(options));
                    *created = true;
                    create_distro(env, name, &dir, &steps, index, progress).map_err(|e| step_error(step, e))?;
                    // `wsl --install` answers 0 after only turning Windows' Linux feature on: the distro is not
                    // registered until the computer restarts. Check, so the next step does not fail with the
                    // confusing "There is no distribution with the supplied name."
                    match env.wsl.is_installed(name) {
                        Ok(true) => {}
                        Ok(false) | Err(_) => return Err(InstallError::NeedsRestart),
                    }
                }
                Step::CopyGuestFiles => {
                    if options.resume {
                        // Whatever the cut-off run left going inside the distro stops here (an apt that kept running, a held lock).
                        let _ = env.wsl.terminate(name);
                    }
                    let tar = env.files.tar_stream().map_err(|e| step_error(step, e))?;
                    env.wsl.exec(name, GuestUser::Root, &["/bin/mkdir", "-p", GUEST_SHARE], None, COPY_TIMEOUT).and_then(|o| o.ok()).map_err(|e| step_error(step, e))?;
                    env.wsl
                        .exec(name, GuestUser::Root, &["/bin/tar", "-x", "-m", "-C", GUEST_SHARE, "-f", "-"], Some(tar), COPY_TIMEOUT)
                        .and_then(|o| o.ok())
                        .map_err(|e| step_error(step, e))?;
                    restart_without_systemd(env, name);
                }
                Step::ApplyRecipes => apply_recipes(env, options, &steps, index, progress)?,
                Step::Restart => env.wsl.terminate(name).map_err(|e| step_error(step, e))?,
                Step::BootCheck => {
                    env.lifecycle.ensure_ready(now_ms()).map_err(|e| step_error(step, e.message()))?;
                    // Only a set up that takes a reset point stops the sandbox here; otherwise it stays running and the
                    // first real start reuses it. A stop that cannot talk to WSL must not turn a working sandbox into a
                    // failed set up; the install's own cleanup stops it again.
                    if steps.contains(&Step::Snapshot) {
                        let _ = env.lifecycle.stop();
                    }
                }
                Step::Finish => {
                    let script = format!(
                        "umask 022 && mkdir -p /etc/pm-sbx && cat > {GUEST_MARKER}.tmp && mv {GUEST_MARKER}.tmp {GUEST_MARKER}"
                    );
                    env.wsl
                        .exec(name, GuestUser::Root, &["/bin/sh", "-c", &script], Some(marker_text(options).into_bytes()), COPY_TIMEOUT)
                        .and_then(|o| o.ok())
                        .map_err(|e| step_error(step, e))?;
                    // The reset point is taken from a stopped distro, so only a set up with a snapshot stops it here.
                    if steps.contains(&Step::Snapshot) {
                        env.wsl.terminate(name).map_err(|e| step_error(step, e))?;
                    }
                }
                Step::Snapshot => {
                    let tar = env.layout.snapshot_path(name, BASE_SNAPSHOT).ok_or(InstallError::BadOptions("bad snapshot name"))?;
                    if let Some(dir) = tar.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| step_error(step, e))?;
                    }
                    env.wsl.export(name, &tar).map_err(|e| step_error(step, e))?;
                }
                Step::DownloadImage | Step::ImportImage => {}
            }
            Ok(())
        })();
        result?;
        index += 1;
    }
    env.records.save(name, &SavedInstall::finished(Some(options)));
    emit_done(progress, &steps);
    Ok(())
}

/// The two image steps. The image file is the download (cached, verified); its unpacked tar is the base reset point.
#[allow(clippy::too_many_arguments)]
fn image_step(
    env: &InstallEnv<'_>,
    options: &InstallOptions,
    step: Step,
    steps: &[Step],
    index: usize,
    image: Option<&(image::Manifest, image::ImageEntry)>,
    image_file: &mut Option<PathBuf>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
    created: &mut bool,
) -> Result<(), InstallError> {
    let name = &options.name;
    let (manifest, entry) = image.ok_or(InstallError::BadOptions("no sandbox image"))?;
    match step {
        Step::DownloadImage => {
            // From here on the distro is "set up not finished" until the last step says otherwise.
            env.records.save(name, &SavedInstall::started(options));
            *created = true;
            let cache = env.layout.root().join("cache");
            emit(progress, steps, index, 0.0, Some("Checking the image file".to_string()));
            // The first callback is where this transfer starts counting (a resumed download starts above zero).
            let mut clock: Option<(u64, Instant)> = None;
            let file = image::fetch(manifest, entry, &cache, cancel, &mut |done, total| {
                let fraction = if total == 0 { 0.0 } else { done as f64 / total as f64 };
                let (base, since) = *clock.get_or_insert((done, Instant::now()));
                progress(Progress {
                    step: step.id(),
                    title: step.title(),
                    index: index + 1,
                    of: steps.len(),
                    percent: percent_at(steps, index, fraction),
                    detail: Some(transfer_text(done, total, done.saturating_sub(base), since.elapsed())),
                    bytes: Some((done, total)),
                });
            })
            .map_err(|e| image_error(step, e))?;
            *image_file = Some(file);
            Ok(())
        }
        Step::ImportImage => {
            let file = image_file.as_ref().ok_or(InstallError::BadOptions("no sandbox image file"))?;
            let tar = env.layout.snapshot_path(name, BASE_SNAPSHOT).ok_or(InstallError::BadOptions("bad snapshot name"))?;
            if let Some(dir) = tar.parent() {
                std::fs::create_dir_all(dir).map_err(|e| step_error(step, e))?;
            }
            let dir = env.layout.distro_dir(name);
            std::fs::create_dir_all(&dir).map_err(|e| step_error(step, e))?;
            let unpacking = Instant::now();
            emit(progress, steps, index, 0.1, Some("Unpacking the sandbox image".to_string()));
            image::unpack_to_tar(file, &tar, cancel, &mut |done, total| {
                let fraction = if total == 0 { 1.0 } else { done as f64 / total as f64 };
                let detail = format!("Unpacking: {}", transfer_text(done, total, done, unpacking.elapsed()));
                emit(progress, steps, index, 0.1 + 0.4 * fraction, Some(detail));
            })
            .map_err(|e| image_error(step, e))?;
            // `wsl --import` prints nothing while it copies, so it runs on a worker while this thread reads the size of the
            // distro's disk file each tick. The import is not cut short here; cancel is handled after it, as before.
            let tar_size = std::fs::metadata(&tar).map(|m| m.len()).unwrap_or(0);
            let disk = dir.join("ext4.vhdx");
            let wsl = env.wsl;
            let copying = Instant::now();
            emit(progress, steps, index, 0.5, Some("Copying into Linux".to_string()));
            let (done_tx, done_rx) = mpsc::channel::<()>();
            let imported = std::thread::scope(|scope| {
                let worker = scope.spawn(move || {
                    let result = wsl.import(name, &dir, &tar);
                    let _ = done_tx.send(());
                    result
                });
                while let Err(mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(PROGRESS_TICK) {
                    let written = std::fs::metadata(&disk).map(|m| m.len()).unwrap_or(0);
                    emit(progress, steps, index, copy_fraction(written, tar_size), Some(copy_detail(written, copying.elapsed())));
                }
                worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            });
            imported.map_err(|e| step_error(step, e))?;
            match env.wsl.is_installed(name) {
                Ok(true) => Ok(()),
                Ok(false) | Err(_) => Err(step_error(step, "the image did not register")),
            }
        }
        _ => Ok(()),
    }
}

/// Puts the short install-time wsl.conf in place and restarts the distro, so the installer runs without systemd. This only
/// makes the install faster, so a failure here is not an install failure: the installer still works with systemd on.
fn restart_without_systemd(env: &InstallEnv<'_>, name: &SandboxName) {
    // Not over the real wsl.conf: a set up that is carried on after the recipes ended already has it (and the boot hook it names).
    let script = format!("test -e /usr/local/sbin/pm-sbx-boot.sh || cp {INSTALL_CONF} /etc/wsl.conf");
    let copied = env.wsl.exec(name, GuestUser::Root, &["/bin/sh", "-c", &script], None, COPY_TIMEOUT).and_then(|o| o.ok());
    if copied.is_ok() {
        let _ = env.wsl.terminate(name);
    }
}

enum CreateEvent {
    /// `wsl --install` printed its second line (the first is "Downloading", the second "Installing", in the person's language).
    Unpacking,
    Done(Result<(), WslError>),
}

fn mm_ss(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// How often the wizard's line is refreshed while a step works without reporting (the copy, the recipes).
const PROGRESS_TICK: Duration = Duration::from_millis(500);

/// "1.2 GB" from one GB up, else the MB text.
fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else {
        mb(bytes)
    }
}

/// "about 5 s left" under 90 s, then "about 2 min left".
fn left_text(secs: f64) -> String {
    let secs = secs.max(0.0).round() as u64;
    if secs < 90 {
        format!("about {} s left", secs.max(1))
    } else {
        format!("about {} min left", (secs + 30) / 60)
    }
}

/// `240.0 MB of 544.0 MB`, and while bytes are moving, `, 62.0 MB/s, about 5 s left`. `moved` is the bytes moved since this
/// transfer started (a resumed download starts above zero), so a cached file that is only checked shows no speed.
pub fn transfer_text(done: u64, total: u64, moved: u64, elapsed: Duration) -> String {
    let mut text = format!("{} of {}", mb(done), mb(total));
    let secs = elapsed.as_secs_f64();
    if done < total && moved > 0 && secs >= 0.5 {
        let speed = moved as f64 / secs;
        text.push_str(&format!(", {}/s, {}", mb(speed as u64), left_text((total - done) as f64 / speed)));
    }
    text
}

/// `Copying into Linux: 1.2 GB written, 0:25`.
pub fn copy_detail(written: u64, elapsed: Duration) -> String {
    format!("Copying into Linux: {} written, {}", size_text(written), mm_ss(elapsed))
}

/// The share of the step for the copy: 0.5 to 0.95 as the distro's disk file grows toward the size of the tar it copies.
pub fn copy_fraction(written: u64, expected: u64) -> f64 {
    if expected == 0 {
        return 0.5;
    }
    0.5 + 0.45 * (written as f64 / expected as f64).min(1.0)
}

/// The wizard's line during the recipes: the guest's own line (it names the unit, and carries speed and time left while
/// something downloads), then the elapsed time. Before the guest says anything, it says the recipes are starting.
pub fn recipe_detail(line: Option<&str>, elapsed: Duration) -> String {
    match line.filter(|text| !text.is_empty()) {
        Some(text) => format!("{text} \u{b7} {}", mm_ss(elapsed)),
        None => format!("Starting the recipes \u{b7} {}", mm_ss(elapsed)),
    }
}

/// The share of the step done and the line to show, for how long `wsl --install` has been running. It prints no
/// percentage, so the bar eases toward the end of each phase and the line carries the elapsed time.
pub fn create_progress(elapsed: Duration, unpacking_for: Option<Duration>) -> (f64, String) {
    match unpacking_for {
        None => {
            let t = elapsed.as_secs_f64();
            (0.7 * t / (t + 60.0), format!("Downloading Ubuntu ({UBUNTU_DOWNLOAD_TEXT}) \u{b7} {}", mm_ss(elapsed)))
        }
        Some(since) => {
            let u = since.as_secs_f64();
            (0.7 + 0.28 * u / (u + 15.0), format!("Unpacking Ubuntu \u{b7} {}", mm_ss(elapsed)))
        }
    }
}

/// `wsl --install` on a thread, while this one keeps the bar and the elapsed time moving once a second.
fn create_distro(
    env: &InstallEnv<'_>,
    name: &SandboxName,
    dir: &Path,
    steps: &[Step],
    index: usize,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), WslError> {
    let (tx, rx) = mpsc::channel::<CreateEvent>();
    let wsl = env.wsl;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut lines = 0usize;
            let mut last = String::new();
            let result = wsl.install_distro_streaming(name, dir, &mut |raw| {
                // wsl.exe prints UTF-16: after a split at the newline byte, NULs are left in the text.
                let line: String = raw.chars().filter(|c| *c != '\0' && !c.is_control()).collect();
                let line = line.trim().to_string();
                if line.is_empty() || line == last {
                    return;
                }
                last = line;
                lines += 1;
                if lines == 2 {
                    let _ = tx.send(CreateEvent::Unpacking);
                }
            });
            let _ = tx.send(CreateEvent::Done(result));
        });
        let started = Instant::now();
        let mut unpacking: Option<Instant> = None;
        loop {
            let (fraction, detail) = create_progress(started.elapsed(), unpacking.map(|t| t.elapsed()));
            emit(progress, steps, index, fraction, Some(detail));
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(CreateEvent::Done(result)) => return result,
                Ok(CreateEvent::Unpacking) => {
                    unpacking.get_or_insert_with(Instant::now);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(WslError::Start("the download stopped unexpectedly".to_string())),
            }
        }
    })
}

fn emit_done(progress: &mut dyn FnMut(Progress), steps: &[Step]) {
    if let Some(last) = steps.last() {
        progress(Progress {
            step: last.id(),
            title: last.title(),
            index: steps.len(),
            of: steps.len(),
            percent: 100,
            detail: Some("Done".to_string()),
        bytes: None,
        });
    }
}

fn apply_recipes(
    env: &InstallEnv<'_>,
    options: &InstallOptions,
    steps: &[Step],
    index: usize,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), InstallError> {
    let step = steps[index];
    let script = format!("{GUEST_SHARE}/desktop/apply-recipe.py");
    let args = options.recipe_args();
    let mut argv: Vec<&str> = vec!["/usr/bin/python3", "-I", &script];
    argv.extend(args.iter().map(String::as_str));
    let mut last_error: Option<String> = None;
    // The guest's lines come only when something changes, so this thread also wakes every tick to refresh the elapsed time.
    let wsl = env.wsl;
    let name = &options.name;
    let started = Instant::now();
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let mut fraction = 0.0;
    let mut line: Option<String> = None;
    emit(progress, steps, index, fraction, Some(recipe_detail(None, started.elapsed())));
    let output = std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            wsl.exec_streaming(name, GuestUser::Root, &argv, None, APPLY_TIMEOUT, &mut |text: &str| {
                let _ = line_tx.send(text.to_string());
            })
        });
        loop {
            match line_rx.recv_timeout(PROGRESS_TICK) {
                Ok(text) => {
                    if let Some(p) = parse_progress_line(&text) {
                        fraction = f64::from(p.done) / f64::from(p.total);
                        line = Some(p.text);
                    } else if let Some(error) = parse_error_line(&text) {
                        last_error = Some(error);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            emit(progress, steps, index, fraction, Some(recipe_detail(line.as_deref(), started.elapsed())));
        }
        worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
    .map_err(|e: WslError| step_error(step, e))?;
    if output.success() {
        return Ok(());
    }
    let reason = last_error.unwrap_or_else(|| apply_exit_reason(output.code).to_string());
    Err(InstallError::Step { step: step.title(), reason })
}
