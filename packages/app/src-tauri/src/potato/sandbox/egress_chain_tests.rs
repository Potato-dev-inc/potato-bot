//! Is the network wall of the sandbox really enforced by the install path, or only shipped? These tests pin the whole chain from
//! the files in `packages/potato-sandbox`, so a link cannot be cut without a test failing:
//!
//! 1. the app copies the egress scripts into the guest (`install.rs`) and the base recipe installs them where the boot hook looks;
//! 2. the recipe replaces the install-time `/etc/wsl.conf` (which has no boot hook, so apt works) with the real one, whose one
//!    `[boot]` command is `pm-sbx-boot.sh`;
//! 3. that hook runs the walls and then the probe, stops at the first failure, and writes `boot.done` only when the probe says passed;
//! 4. the desktop refuses to start without `boot.done` and a passed probe;
//! 5. the app refuses to use the computer until the guest says the walls passed (`Readiness::ready`), and the install's last step is a
//!    real start with that check.
//!
//! What this cannot prove: that the rules hold on a real distro. That is the probe's job at every boot (and `test_egress_probe.py`
//! plays it against fakes); it was not run against a live WSL here.

use std::path::{Path, PathBuf};

use super::lifecycle::{Egress, Readiness};

fn guest(rel: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("potato-sandbox").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace('\r', "")
}

#[test]
fn the_install_path_ships_and_installs_both_egress_scripts() {
    let install = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/potato/sandbox/install.rs")).unwrap();
    assert!(install.contains("\"guest/egress/pm-sbx-egress.sh\"") && install.contains("\"guest/egress/pm-sbx-egress-probe.sh\""), "the app does not ship the egress scripts");
    let recipe = guest("recipes/base.json");
    assert!(recipe.contains("/usr/local/sbin/pm-sbx-egress.sh") && recipe.contains("/usr/local/sbin/pm-sbx-egress-probe.sh"));
    assert!(recipe.contains("/usr/local/sbin/pm-sbx-boot.sh"), "the boot hook is installed where wsl.conf names it");
    // a later app update refreshes them too, so an old sandbox does not keep old walls
    let lifecycle = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/potato/sandbox/lifecycle.rs")).unwrap();
    assert!(lifecycle.contains("/usr/local/sbin/pm-sbx-egress.sh") && lifecycle.contains("/usr/local/sbin/pm-sbx-egress-probe.sh"));
}

#[test]
fn only_the_real_wsl_conf_has_the_boot_hook_and_the_recipe_puts_it_in_place() {
    let real = guest("guest/desktop/wsl.conf");
    assert!(real.lines().any(|l| l.trim() == "command=/usr/local/sbin/pm-sbx-boot.sh"), "the real wsl.conf runs the boot hook");
    let during_install = guest("guest/desktop/wsl-install.conf");
    assert!(!during_install.lines().any(|l| l.trim_start().starts_with("command=")), "the install-time conf has no boot hook: apt needs the network");
    let recipe = guest("recipes/base.json");
    assert!(recipe.contains("wsl.conf\\\" > /etc/wsl.conf"), "the recipe replaces /etc/wsl.conf with the real one");
}

#[test]
fn the_boot_hook_runs_the_walls_then_the_probe_and_fails_closed() {
    let boot = guest("guest/desktop/pm-sbx-boot.sh");
    assert!(boot.contains("for step in pm-no-wslg.sh pm-sbx-egress.sh pm-sbx-egress-probe.sh"), "the walls, then the proof, in this order");
    assert!(boot.contains("fail \"$step failed (exit $rc)\""), "a failed step stops the chain");
    assert!(boot.contains("grep -qx 'state=passed' \"$STATUS_DIR/egress-probe.status\""), "an exit code alone is not enough: the probe must say passed");
    let done = boot.find(">\"$STATUS_DIR/boot.done\"").expect("boot.done is written");
    let verdict = boot.find("grep -qx 'state=passed'").unwrap();
    assert!(verdict < done, "boot.done comes after the verdict");
}

#[test]
fn the_desktop_refuses_to_start_without_boot_done_and_a_passed_probe() {
    let start = guest("guest/desktop/pm-sbx-start-desktop.sh");
    assert!(start.contains("BOOT_DONE=/run/pm-sbx/boot.done"));
    assert!(start.contains("REFUSING: the egress probe has not passed"));
}

#[test]
fn the_app_does_not_use_the_computer_until_the_guest_says_the_walls_passed() {
    let base = Readiness { boot_done: true, boot_failed: false, boot_reason: String::new(), egress: Egress::Passed, desktop_up: true, control_up: true };
    assert!(base.ready());
    for egress in [Egress::Unknown, Egress::Failed("gateway reachable".into())] {
        let not_ready = Readiness { egress: egress.clone(), ..base.clone() };
        assert!(!not_ready.ready(), "{egress:?}");
    }
    assert!(!Readiness { boot_done: false, ..base.clone() }.ready());
    assert!(!Readiness { boot_failed: true, ..base.clone() }.ready());
    // the install's last real step is a start with that check, so a set up whose walls do not hold does not finish
    let install = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/potato/sandbox/install.rs")).unwrap();
    assert!(install.contains("Step::BootCheck => {") && install.contains("env.lifecycle.ensure_ready(now_ms())"));
}

#[test]
fn the_walls_close_the_windows_host_the_lan_and_the_metadata_address() {
    let walls = guest("guest/egress/pm-sbx-egress.sh");
    for range in ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16", "100.64.0.0/10", "127.0.0.0/8", "fc00::/7", "fe80::/10"] {
        assert!(walls.contains(range), "the walls lost {range}");
    }
    assert!(walls.contains("10.255.255.254"), "the Windows DNS tunnel address");
    assert!(walls.contains("default route"), "the Windows host is the default gateway, read from the route");
}

#[test]
fn what_the_walls_do_not_do_is_written_down_and_the_site_list_does_not_claim_to_do_it() {
    // The walls are by address range, for the whole computer: the public internet is open to every program and every bot that
    // shares it. The owner's site list limits only the bot's web tools, and its page says so (see AdvancedSites).
    let readme = guest("guest/egress/README.md");
    assert!(readme.contains("Everything on the public internet"), "the README says what stays open");
}
