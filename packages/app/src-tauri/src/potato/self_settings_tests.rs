//! The rules of settings from chat that need no engine: which settings there are, how each value is checked, the words on the
//! card, and the journal that Undo reads.

use std::path::PathBuf;

use serde_json::json;

use super::self_settings::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-selfset-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn only_four_settings_exist_and_the_names_are_exact() {
    for (word, field) in [("name", Field::Name), ("Instructions", Field::Instructions), (" persona ", Field::Persona), ("MODEL", Field::Model)] {
        assert_eq!(Field::parse(word), Some(field));
    }
    for word in ["level", "limit", "tools", "connectors", "name2", "n", "", "model.base_url"] {
        assert_eq!(Field::parse(word), None, "{word}");
    }
    assert!(!Field::Name.restarts() && Field::Instructions.restarts() && Field::Persona.restarts() && Field::Model.restarts());
}

#[test]
fn values_are_checked_like_the_app_checks_them() {
    assert_eq!(clean_value(Field::Name, "  Cloud ").unwrap(), "Cloud");
    assert!(clean_value(Field::Name, "").is_err());
    assert!(clean_value(Field::Name, &"x".repeat(41)).is_err());
    assert!(clean_value(Field::Name, "two\nlines").is_err());
    assert!(clean_value(Field::Instructions, "   ").is_err(), "clearing is the owner's");
    assert!(clean_value(Field::Instructions, &"x".repeat(4001)).is_err());
    assert_eq!(clean_value(Field::Persona, " PRO ").unwrap(), "pro");
    assert!(clean_value(Field::Persona, "wizard").is_err());
    assert_eq!(clean_value(Field::Model, " gpt-4.1-mini ").unwrap(), "gpt-4.1-mini");
    assert_eq!(clean_value(Field::Model, "openai/gpt-4o").unwrap(), "openai/gpt-4o");
    assert_eq!(clean_value(Field::Model, "llama3:8b").unwrap(), "llama3:8b");
    for bad in ["", "has space", "https://x.example.com?key=1", "https://evil.example.com", "//evil.example.com", "a/../b", "a;b", &"m".repeat(81)] {
        assert!(clean_value(Field::Model, bad).is_err(), "{bad}");
    }
}

#[test]
fn the_secret_looking_text_in_instructions_is_taken_out_before_it_is_kept() {
    let text = clean_value(Field::Instructions, "Use sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghij for everything").unwrap();
    assert!(!text.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ"), "{text}");
}

#[test]
fn a_request_without_a_value_or_with_an_unknown_setting_is_refused_with_a_sentence() {
    assert_eq!(parse_request(&json!({"setting": "name"})).unwrap_err().0, "POTATO_SETTING_BAD_VALUE");
    assert_eq!(parse_request(&json!({"setting": "level", "value": "full"})).unwrap_err().0, NOT_ALLOWED);
    assert_eq!(parse_request(&json!({"value": "x"})).unwrap_err().0, NOT_ALLOWED);
    assert_eq!(parse_request(&json!({"setting": "name", "value": 7})).unwrap_err().0, "POTATO_SETTING_BAD_VALUE");
    assert_eq!(parse_request(&json!({"setting": "name", "value": "Cloud"})).unwrap(), (Field::Name, "Cloud".to_string()));
}

#[test]
fn the_card_words_are_short_and_plain() {
    assert_eq!(describe(Field::Name, "Nimbus", "Cloud"), "Changed its name from \u{201c}Nimbus\u{201d} to \u{201c}Cloud\u{201d}.");
    assert_eq!(describe(Field::Persona, "fish", "pro"), "Changed its persona from Fish to Professional.");
    assert_eq!(describe(Field::Model, "a", "b"), "Changed its model from a to b.");
    let long = "word ".repeat(100);
    let card = describe(Field::Instructions, "", &long);
    assert!(card.starts_with("Set its instructions:") && card.contains('…') && card.chars().count() < 140, "{card}");
}

#[test]
fn the_journal_keeps_each_change_marks_undone_and_is_capped() {
    let root = temp("journal");
    let one = record(&root, "nimbus", Field::Name, "Nimbus", "Cloud", 10).unwrap();
    assert_eq!((one.field.as_str(), one.prior.as_str(), one.current.as_str(), one.undone), ("name", "Nimbus", "Cloud", false));
    assert!(find(&root, "nimbus", &one.id).is_ok());
    assert_eq!(find(&root, "mochi", &one.id).unwrap_err().code, "POTATO_SETTING_CHANGE_GONE", "another bot's change is not found");
    assert!(undone_ids(&root).is_empty());
    mark_undone(&root, &one.id).unwrap();
    assert!(undone_ids(&root).contains(&one.id));
    for n in 0..KEEP + 5 {
        record(&root, "nimbus", Field::Persona, "fish", "potato", n as i64).unwrap();
    }
    assert!(find(&root, "nimbus", &one.id).is_err(), "the oldest went");
}

#[test]
fn a_damaged_journal_reads_as_nothing_undone_and_is_never_overwritten() {
    let root = temp("damaged");
    std::fs::write(path(&root), "{ nope").unwrap();
    assert!(undone_ids(&root).is_empty());
    assert!(record(&root, "nimbus", Field::Name, "a", "b", 1).is_err());
    assert_eq!(std::fs::read_to_string(path(&root)).unwrap(), "{ nope");
}
