//! The Skills tab's store: the brain's file layout read and written, the limits, no path escapes, delete moves the file.

use std::path::PathBuf;

use super::skills::*;
use super::store;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-skills-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    store::add_bot(&dir, "fin", "fish", 1).unwrap();
    dir
}

/// A skill file exactly as the brain writes it (`skills.ts` `render`).
fn put(root: &PathBuf, slug: &str, name: &str, description: &str, body: &str) {
    let folder = dir(root, "fin").unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(format!("{slug}.md")), format!("# {name}\nDescription: {description}\n\n{body}\n")).unwrap();
}

#[test]
fn lists_the_skills_the_brain_wrote_by_name_with_their_text() {
    let root = temp("list");
    assert!(list(&root, "fin").unwrap().is_empty(), "no folder yet is an empty list");
    put(&root, "weekly-report", "Weekly report", "Summarise the week", "1. Open the notes\n2. Write five lines");
    put(&root, "abc", "Abc", "", "just steps");
    let skills = list(&root, "fin").unwrap();
    assert_eq!(skills.iter().map(|s| s.slug.as_str()).collect::<Vec<_>>(), ["abc", "weekly-report"]);
    assert_eq!(skills[1].name, "Weekly report");
    assert_eq!(skills[1].description, "Summarise the week");
    assert_eq!(skills[1].body, "1. Open the notes\n2. Write five lines");
    assert_eq!(skills[0].description, "");
}

#[test]
fn a_file_whose_name_is_not_a_plain_slug_or_has_no_title_is_not_listed() {
    let root = temp("junk");
    put(&root, "good", "Good", "d", "b");
    let folder = dir(&root, "fin").unwrap();
    std::fs::write(folder.join("has space.md"), "# x\n\nb\n").unwrap();
    std::fs::write(folder.join("notitle.md"), "no heading here\n").unwrap();
    std::fs::write(folder.join("note.txt"), "# t\n").unwrap();
    assert_eq!(list(&root, "fin").unwrap().len(), 1);
}

#[test]
fn edit_keeps_the_name_and_the_brain_layout_and_cleans_the_description() {
    let root = temp("edit");
    put(&root, "weekly-report", "Weekly report", "old", "old steps");
    let saved = save(&root, "fin", "weekly-report", "  a  new\nline  ", "new steps\r\nsecond").unwrap();
    assert_eq!((saved.name.as_str(), saved.description.as_str(), saved.body.as_str()), ("Weekly report", "a new line", "new steps\nsecond"));
    let raw = std::fs::read_to_string(dir(&root, "fin").unwrap().join("weekly-report.md")).unwrap();
    assert_eq!(raw, "# Weekly report\nDescription: a new line\n\nnew steps\nsecond\n");
    // and the list reads back what was saved
    assert_eq!(list(&root, "fin").unwrap()[0].body, "new steps\nsecond");
}

#[test]
fn edit_refuses_empty_steps_and_a_file_over_the_limit_and_changes_nothing() {
    let root = temp("limits");
    put(&root, "s", "S", "d", "keep me");
    assert_eq!(save(&root, "fin", "s", "d", "   ").unwrap_err().code, "POTATO_SKILL_INVALID");
    assert_eq!(save(&root, "fin", "s", "d", &"x".repeat(SKILL_CAP)).unwrap_err().code, "POTATO_SKILL_INVALID");
    assert_eq!(list(&root, "fin").unwrap()[0].body, "keep me");
    // a long description is cut to its limit, not refused
    let saved = save(&root, "fin", "s", &"d".repeat(500), "ok").unwrap();
    assert_eq!(saved.description.chars().count(), DESCRIPTION_MAX);
}

#[test]
fn a_slug_with_a_path_in_it_reaches_no_file() {
    let root = temp("paths");
    put(&root, "s", "S", "d", "b");
    let outside = root.join("brain").join("fin").join("vault").join("state").join("notes").join("secret.md");
    std::fs::write(&outside, "# Secret\n\nhidden\n").unwrap();
    for bad in ["../secret", "..", "a/b", "a\\b", "", "s.md", "/etc/passwd"] {
        assert_eq!(save(&root, "fin", bad, "d", "b").unwrap_err().code, "POTATO_SKILL_NOT_FOUND", "{bad}");
        assert_eq!(delete(&root, "fin", bad).unwrap_err().code, "POTATO_SKILL_NOT_FOUND", "{bad}");
    }
    assert!(outside.exists());
}

#[test]
fn delete_moves_the_file_to_removed_and_never_overwrites_an_earlier_one() {
    let root = temp("delete");
    put(&root, "s", "S", "d", "first");
    delete(&root, "fin", "s").unwrap();
    assert!(list(&root, "fin").unwrap().is_empty());
    put(&root, "s", "S", "d", "second");
    delete(&root, "fin", "s").unwrap();
    let removed = dir(&root, "fin").unwrap().join(".removed");
    assert!(std::fs::read_to_string(removed.join("s.md")).unwrap().contains("first"));
    assert!(std::fs::read_to_string(removed.join("s-2.md")).unwrap().contains("second"));
    assert_eq!(delete(&root, "fin", "s").unwrap_err().code, "POTATO_SKILL_NOT_FOUND");
}

#[test]
fn an_unknown_bot_is_refused() {
    let root = temp("bot");
    assert!(list(&root, "nobody").is_err());
}
