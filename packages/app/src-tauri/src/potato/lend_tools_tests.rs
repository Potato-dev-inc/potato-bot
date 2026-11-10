//! Pure checks for the password guard and the "no labels" note.

use super::*;

fn elem(role: &str, name: &str, password: bool, value: Option<&str>) -> ObsElem {
    ObsElem { r#ref: "e1".into(), role: role.into(), name: name.into(), value: value.map(str::to_string), rect: (0, 0, 100, 20), password }
}

#[test]
fn password_by_name_property_or_style() {
    assert!(is_password_field("Password", false, false));
    assert!(is_password_field("密碼", false, false));
    assert!(is_password_field("パスワード", false, false));
    assert!(is_password_field("", true, false), "UIA IsPassword with no name");
    assert!(is_password_field("", false, true), "ES_PASSWORD with no name");
    assert!(is_password_field("Search", true, false));
    assert!(!is_password_field("Search", false, false));
    assert!(!is_password_field("", false, false));
}

#[test]
fn obs_elem_password_signs() {
    assert!(elem("edit", "", true, None).is_password());
    assert!(elem("edit", "Your password", false, None).is_password());
    assert!(!elem("edit", "Search", false, None).is_password());
    assert!(!elem("button", "Show password", false, None).is_password(), "a button is not a password box");
}

#[test]
fn password_at_finds_a_nameless_password_box() {
    let mut pw = elem("edit", "", true, None);
    pw.rect = (50, 50, 100, 20);
    let mut wide = elem("edit", "Form", false, None);
    wide.rect = (0, 0, 400, 400);
    let elems = vec![wide, pw];
    assert!(password_at(&elems, 60, 60));
    assert!(!password_at(&elems, 300, 300));
}

#[test]
fn observe_never_returns_a_password_value() {
    let apps = [LentView {
        id: "l1".into(),
        hwnd: 1,
        pid: 2,
        app_name: "App".into(),
        title: "App".into(),
        mode: crate::potato::lend::session::Mode::Visible,
        phase: Phase::Free,
        reason: None,
        picture_live: true,
    }];
    let elems = vec![elem("edit", "", true, Some("hunter2")), elem("edit", "Password", false, Some("hunter3")), elem("edit", "Name", false, Some("Ana"))];
    let out = observe_json(&apps[0], &elems, false, None);
    let text = out.to_string();
    assert!(!text.contains("hunter2") && !text.contains("hunter3"), "{text}");
    assert!(text.contains("Ana"));
}

#[test]
fn no_labels_note_only_for_sparse_chromium() {
    assert_eq!(no_labels_note(0, "Chrome_WidgetWin_1"), Some(NO_LABELS_NOTE));
    assert_eq!(no_labels_note(2, "Chrome_WidgetWin_1"), Some(NO_LABELS_NOTE));
    assert_eq!(no_labels_note(3, "Chrome_WidgetWin_1"), None);
    assert_eq!(no_labels_note(0, "Notepad"), None);
    assert!(NO_LABELS_NOTE.contains("x/y"));
}
