//! The bot groups switch: off unless both it and Potato Bot are on.

use serde_json::json;

use super::live::{groups_labs_on, GROUPS_LABS_KEY, LABS_KEY};

#[test]
fn groups_labs_off_when_the_key_is_missing() {
    assert!(!groups_labs_on(&json!({ LABS_KEY: true })));
}

#[test]
fn groups_labs_off_when_potato_bot_is_off() {
    assert!(!groups_labs_on(&json!({ LABS_KEY: false, GROUPS_LABS_KEY: true })));
    assert!(!groups_labs_on(&json!({ GROUPS_LABS_KEY: true })));
}

#[test]
fn groups_labs_on_only_when_both_are_exactly_true() {
    assert!(groups_labs_on(&json!({ LABS_KEY: true, GROUPS_LABS_KEY: true })));
    assert!(!groups_labs_on(&json!({ LABS_KEY: true, GROUPS_LABS_KEY: "true" })), "a string is not on");
}

#[test]
fn groups_key_is_the_agreed_name() {
    assert_eq!(GROUPS_LABS_KEY, "potato_groups_enabled");
}
