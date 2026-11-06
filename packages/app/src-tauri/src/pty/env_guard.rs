//! The `no_push` environment guard for an agent's pane: the pane cannot push to GitHub.
//!
//! What the guard does to a pane's environment:
//! - `gh` reads no login (`GH_CONFIG_DIR` points at an empty private folder) and never prompts.
//! - A push to ANY destination fails, whatever the URL looks like (a web URL, a folder, a UNC share
//!   path, `file://`). Two layers: (1) every push URL is rewritten (`pushInsteadOf` with an empty
//!   prefix matches every URL) to a remote helper that does not exist, so the push fails before
//!   anything is sent; (2) `core.hooksPath` points at a PM folder whose `pre-push` hook always
//!   refuses, which also stops a push to a `pushurl` (that setting skips the rewrite).
//!   Residual, by design of git: `git push --no-verify` skips the hook, and an agent that edits
//!   the git settings itself is caught after the run (the push check holds the run).
//! - No stored git credential is handed out (`credential.helper` is reset) and git never prompts.
//! - GitHub variables (`GITHUB_*`, `GH_*`, except the two the guard sets) and token, secret,
//!   password and `_PAT` variables the app inherited are removed from the pane.
//!
//! Residual, documented in Settings: the agent still runs as the person and can read files on disk.
//! `*_API_KEY` variables are kept, because agents use them to log in. This module is generic: it
//! never names an agent, and the caller decides which runs ask for the guard.

use crate::tui::LaunchEnv;

/// The guard name a run asks for. An unknown guard name is refused by the spawn (fail closed).
pub const NO_PUSH: &str = "no_push";

/// True when `guard` names a guard this build knows. Callers refuse anything else (fail closed).
pub fn known(guard: &str) -> bool {
    guard == NO_PUSH
}

/// `base` (the account's env, then the pane's hook env) with the guard on top. The guard comes
/// last, so it wins: its sets replace an earlier set of the same name (names compare ignoring
/// case, as on Windows), and its removes apply even to a variable `base` set. Names in `keep` are
/// the app's own pane variables (hook token, pane id); a guard remove never strips those.
pub fn layer(mut base: LaunchEnv, guard: LaunchEnv, keep: &[String]) -> LaunchEnv {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let kept = |name: &str| keep.iter().any(|k| same(k, name));
    let stripped: Vec<String> = guard.remove.into_iter().filter(|name| !kept(name)).collect();
    // Earlier sets the guard replaces or strips are dropped, so the vector shows the guard last.
    base.set.retain(|(name, _)| {
        !guard.set.iter().any(|(g, _)| same(name, g)) && !stripped.iter().any(|r| same(name, r))
    });
    base.set.extend(guard.set);
    for name in stripped {
        if !base.remove.iter().any(|r| same(r, &name)) {
            base.remove.push(name);
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_known_name_is_known() {
        assert!(known(NO_PUSH));
        for name in ["", "NO_PUSH", "no-push", "no_push ", "push"] {
            assert!(!known(name), "{name:?}");
        }
    }

    #[test]
    fn agent_names_stay_out_of_this_file() {
        let src = include_str!("env_guard.rs");
        let generic = src.split("#[cfg(test)]").next().unwrap_or(src).to_ascii_lowercase();
        for name in ["claude", "codex", "cursor", "opencode", "agy"] {
            assert!(!generic.contains(name), "agent name `{name}` in generic env_guard code");
        }
    }
}
