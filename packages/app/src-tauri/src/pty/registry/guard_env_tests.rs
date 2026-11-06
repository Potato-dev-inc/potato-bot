//! A guarded pane's launch env: account < hooks < pane id < guard (the guard is last and wins).

use super::*;
use crate::tui::LaunchEnv;

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
}

fn account() -> LaunchEnv {
    LaunchEnv {
        set: pairs(&[("GH_CONFIG_DIR", "C:/account/gh"), ("ACCOUNT_HOME", "C:/account/home"), ("ORDER", "account")]),
        remove: vec!["ACCOUNT_DROP".into()],
    }
}

fn hooks() -> Vec<(String, String)> {
    pairs(&[("PM_HOOK_TOKEN", "hook-token"), ("ORDER", "hooks")])
}

fn value_of(env: &LaunchEnv, key: &str) -> Option<String> {
    // The last set wins, like the child process sees it.
    env.set.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[test]
fn without_a_guard_the_env_is_the_account_then_the_hooks_then_the_pane_id() {
    let env = spawn::compose_pane_env(account(), hooks(), "pane-1", None);
    assert_eq!(value_of(&env, "ORDER").as_deref(), Some("hooks"), "hooks win over the account");
    assert_eq!(value_of(&env, "GH_CONFIG_DIR").as_deref(), Some("C:/account/gh"));
    assert_eq!(value_of(&env, crate::pty::PANE_ID_ENV).as_deref(), Some("pane-1"));
    assert_eq!(env.remove, vec!["ACCOUNT_DROP".to_string()]);
}
