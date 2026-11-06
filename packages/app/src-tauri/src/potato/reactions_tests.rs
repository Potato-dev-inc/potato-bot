//! Emoji reactions: the fixed set, toggling on and off, one per emoji per message, kept across a reopen, capped.

use std::path::PathBuf;

use super::reactions::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-reactions-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn toggling_adds_then_removes_one_reaction() {
    let root = temp("toggle");
    assert!(toggle(&root, "m-a", "👍").unwrap());
    assert_eq!(load(&root).get("m-a"), Some(&vec!["👍".to_string()]));
    // a second toggle of the same emoji takes it off again: never two of one emoji on a message
    assert!(!toggle(&root, "m-a", "👍").unwrap());
    assert!(load(&root).get("m-a").is_none());
}

#[test]
fn different_emoji_stack_on_one_message_in_the_fixed_order() {
    let root = temp("stack");
    toggle(&root, "m-a", "🙏").unwrap();
    toggle(&root, "m-a", "👍").unwrap();
    toggle(&root, "m-b", "🎉").unwrap();
    assert_eq!(load(&root).get("m-a"), Some(&vec!["👍".to_string(), "🙏".to_string()]));
    assert_eq!(load(&root).get("m-b"), Some(&vec!["🎉".to_string()]));
}

#[test]
fn reactions_survive_reading_the_files_again() {
    let root = temp("persist");
    toggle(&root, "m-a", "❤️").unwrap();
    toggle(&root, "m-a", "😂").unwrap();
    // a new process reads only the file
    let again = load(&root);
    assert_eq!(again.get("m-a"), Some(&vec!["❤️".to_string(), "😂".to_string()]));
    assert!(root.join("reactions.json").exists());
}

#[test]
fn an_emoji_outside_the_set_is_refused_and_writes_nothing() {
    let root = temp("unknown");
    for bad in ["💀", "", "👍👍", "thumbs up", "<b>", "👍 "] {
        let e = toggle(&root, "m-a", bad).unwrap_err();
        assert_eq!(e.code, "POTATO_REACTION_INVALID", "{bad:?}");
    }
    assert!(load(&root).is_empty());
    assert!(!root.join("reactions.json").exists());
}

#[test]
fn only_the_newest_messages_keep_their_reactions() {
    let root = temp("cap");
    for i in 0..(KEEP + 3) {
        toggle(&root, &format!("m-{i}"), "👍").unwrap();
    }
    let all = load(&root);
    assert_eq!(all.len(), KEEP);
    assert!(!all.contains_key("m-0"));
    assert!(all.contains_key(&format!("m-{}", KEEP + 2)));
}

#[test]
fn a_damaged_file_is_an_error_and_is_not_overwritten() {
    let root = temp("damaged");
    std::fs::write(root.join("reactions.json"), "{not json").unwrap();
    assert!(load(&root).is_empty());
    assert!(toggle(&root, "m-a", "👍").is_err());
    assert_eq!(std::fs::read_to_string(root.join("reactions.json")).unwrap(), "{not json");
}
