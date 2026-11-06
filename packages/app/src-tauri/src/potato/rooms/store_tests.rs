//! The room store: ids, names, bot counts, lines, the 5000-line cap, deletion. Each test has its own folder.

use std::path::PathBuf;

use super::store::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-rooms-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn room(id: &str, bots: &[&str]) -> Room {
    Room { id: id.into(), name: format!("Room {id}"), bot_ids: ids(bots), created_ms: 1, last_ms: 1, unread: 0 }
}

fn line(id: &str, text: &str, at: i64) -> RoomLine {
    RoomLine {
        id: id.into(),
        from: RoomFrom { kind: Speaker::Person, id: "owner".into(), name: "Ren".into() },
        text: text.into(),
        at,
        hop: 0,
        in_reply_to: None,
        file: None,
    }
}

const ROOM: &str = "r0123456789ab";

#[test]
fn validate_accepts_two_bots_and_trims_the_name() {
    let known = ids(&["a", "b", "c"]);
    assert_eq!(validate(" Weekend ", &ids(&["a", "b"]), &known), Ok(("Weekend".to_string(), ids(&["a", "b"]))));
}

#[test]
fn validate_refuses_one_bot_and_six_bots() {
    let known: Vec<String> = (0..6).map(|i| format!("bot{i}")).collect();
    assert_eq!(validate("x", &ids(&["bot0"]), &known), Err(RoomError::TooFewBots));
    assert_eq!(validate("x", &known, &known), Err(RoomError::TooManyBots));
    let five: Vec<String> = known[..5].to_vec();
    assert!(validate("x", &five, &known).is_ok(), "five bots is the most");
}

#[test]
fn validate_refuses_a_duplicate_an_unknown_bot_and_bad_names() {
    let known = ids(&["a", "b", "c"]);
    assert_eq!(validate("x", &ids(&["a", "a"]), &known), Err(RoomError::DuplicateBot));
    assert_eq!(validate("x", &ids(&["a", "z"]), &known), Err(RoomError::UnknownBot("z".into())));
    assert_eq!(validate("   ", &ids(&["a", "b"]), &known), Err(RoomError::NameEmpty));
    let long = "n".repeat(61);
    assert_eq!(validate(&long, &ids(&["a", "b"]), &known), Err(RoomError::NameTooLong));
    assert!(validate(&"n".repeat(60), &ids(&["a", "b"]), &known).is_ok());
}

#[test]
fn valid_id_checks_the_shape_of_an_id() {
    assert!(valid_id('r', "r0123456789ab"));
    assert!(!valid_id('r', "r0123"));
    assert!(!valid_id('r', "x0123456789ab"));
    assert!(!valid_id('r', "r0123456789AB"), "lowercase only");
}

#[test]
fn lines_path_refuses_anything_but_a_room_id() {
    let root = temp("path");
    assert!(lines_path(&root, "../x").is_none());
    assert!(lines_path(&root, "r0123456789ab/../../x").is_none());
    assert!(lines_path(&root, ROOM).unwrap().starts_with(dir(&root)));
}

#[test]
fn new_ids_have_the_right_shape() {
    assert!(valid_id('r', &new_id('r')));
    assert!(valid_id('l', &new_id('l')));
}

#[test]
fn round_trip_rooms() {
    let root = temp("rooms");
    assert!(load_rooms(&root).unwrap().is_empty(), "a missing index is empty");
    let rooms = vec![room(ROOM, &["a", "b"]), room("r1123456789ab", &["a", "c"])];
    save_rooms(&root, &rooms).unwrap();
    assert_eq!(load_rooms(&root).unwrap(), rooms);
}

#[test]
fn a_damaged_index_is_an_error_and_is_not_overwritten() {
    let root = temp("damaged");
    std::fs::create_dir_all(dir(&root)).unwrap();
    std::fs::write(index_path(&root), "{ damaged").unwrap();
    assert!(matches!(load_rooms(&root), Err(RoomError::Io(_))));
    assert_eq!(std::fs::read_to_string(index_path(&root)).unwrap(), "{ damaged");
}

#[test]
fn append_and_read_lines_oldest_first() {
    let root = temp("lines");
    for (i, text) in ["one", "two", "three"].iter().enumerate() {
        append_line(&root, ROOM, line(&format!("l0123456789a{i}"), text, i as i64), None).unwrap();
    }
    let last_two = read_lines(&root, ROOM, 2);
    assert_eq!(last_two.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["two", "three"]);
    assert_eq!(read_lines(&root, ROOM, 100).len(), 3);
}

#[test]
fn a_line_text_is_cut_to_8000_characters_and_cleaned() {
    let root = temp("cut");
    append_line(&root, ROOM, line("l0123456789ab", &"é".repeat(9_000), 1), None).unwrap();
    assert_eq!(read_lines(&root, ROOM, 1)[0].text.chars().count(), 8_000);
}

#[test]
fn read_skips_a_broken_line() {
    let root = temp("broken");
    append_line(&root, ROOM, line("l0123456789aa", "good one", 1), None).unwrap();
    let path = lines_path(&root, ROOM).unwrap();
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("not json\n");
    std::fs::write(&path, text).unwrap();
    append_line(&root, ROOM, line("l0123456789ab", "good two", 2), None).unwrap();
    let got = read_lines(&root, ROOM, 10);
    assert_eq!(got.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["good one", "good two"]);
}

#[test]
fn trim_keeps_the_newest_5000_lines() {
    let root = temp("trim");
    let path = lines_path(&root, ROOM).unwrap();
    std::fs::create_dir_all(dir(&root)).unwrap();
    let mut text = String::new();
    for i in 0..5_010 {
        text.push_str(&serde_json::to_string(&line(&format!("l{:012x}", i), "x", i)).unwrap());
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();
    trim_lines(&root, ROOM).unwrap();
    let kept = std::fs::read_to_string(&path).unwrap();
    assert_eq!(kept.lines().count(), 5_000);
    let first: RoomLine = serde_json::from_str(kept.lines().next().unwrap()).unwrap();
    assert_eq!(first.at, 10, "line number 11 is the first kept");
}

#[test]
fn delete_removes_the_index_entry_and_the_lines() {
    let root = temp("delete");
    save_rooms(&root, &[room(ROOM, &["a", "b"])]).unwrap();
    append_line(&root, ROOM, line("l0123456789ab", "hi", 1), None).unwrap();
    delete_room(&root, ROOM).unwrap();
    assert!(load_rooms(&root).unwrap().is_empty());
    assert!(!lines_path(&root, ROOM).unwrap().exists());
    assert_eq!(delete_room(&root, ROOM), Err(RoomError::NotFound));
}

#[test]
fn every_error_has_a_plain_wire_text() {
    let all = [
        RoomError::NameEmpty,
        RoomError::NameTooLong,
        RoomError::TooFewBots,
        RoomError::TooManyBots,
        RoomError::DuplicateBot,
        RoomError::UnknownBot("z".into()),
        RoomError::TooManyRooms,
        RoomError::NotFound,
        RoomError::BadId,
        RoomError::Io("disk".into()),
    ];
    for error in all {
        let wire = error.to_wire();
        assert!(wire.starts_with("POTATO_ROOM_"), "{wire}");
        assert!(wire.contains(": "), "{wire}");
    }
}
