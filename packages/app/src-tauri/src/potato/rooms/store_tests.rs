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
    Room { id: id.into(), name: format!("Room {id}"), bot_ids: ids(bots), created_ms: 1, last_ms: 1, unread: 0, ..Default::default() }
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
        handoff: None,
    }
}

const ROOM: &str = "r0123456789ab";

#[test]
fn validate_accepts_two_bots_and_trims_the_name() {
    let known = ids(&["a", "b", "c"]);
    assert_eq!(validate(" Weekend ", &ids(&["a", "b"]), &known), Ok(("Weekend".to_string(), ids(&["a", "b"]))));
}

#[test]
fn validate_refuses_one_bot_and_eleven_bots() {
    let known: Vec<String> = (0..11).map(|i| format!("bot{i}")).collect();
    assert_eq!(validate("x", &ids(&["bot0"]), &known), Err(RoomError::TooFewBots));
    assert_eq!(validate("x", &known, &known), Err(RoomError::TooManyBots));
    let ten: Vec<String> = known[..10].to_vec();
    assert!(validate("x", &ten, &known).is_ok(), "ten bots is the most");
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

// ---- group settings ----

#[test]
fn an_old_group_file_without_the_new_settings_reads_as_listen_with_nothing_set() {
    let old = r#"{"v":1,"rooms":[{"id":"r0123456789ab","name":"Old","botIds":["a","b"],"createdMs":1,"lastMs":1,"unread":0}]}"#;
    let dir = temp("old_index");
    std::fs::create_dir_all(super::store::dir(&dir)).unwrap();
    std::fs::write(index_path(&dir), old).unwrap();
    let rooms = load_rooms(&dir).unwrap();
    assert_eq!((rooms[0].mode, rooms[0].instructions.as_str(), rooms[0].folder.as_str(), rooms[0].off_bots.len()), (RoomMode::Listen, "", "", 0));
    assert_eq!(rooms[0].folder_path(), "/home/pm/groups/r0123456789ab");
}

#[test]
fn settings_are_saved_cleaned_and_come_back() {
    let dir = temp("settings_roundtrip");
    let mut r = room("r0123456789ab", &["a", "b", "c"]);
    apply_settings(&mut r, "  Be brief.\u{0}\nNo emoji.  ", " Trip-2026 ", "Mentions", &ids(&["c", "ghost", "c"])).unwrap();
    assert_eq!((r.instructions.as_str(), r.folder.as_str(), r.mode, r.off_bots.clone()), ("Be brief.\nNo emoji.", "trip-2026", RoomMode::Mentions, ids(&["c"])));
    save_rooms(&dir, &[r.clone()]).unwrap();
    assert_eq!(load_rooms(&dir).unwrap(), vec![r.clone()]);
    assert_eq!(r.folder_path(), "/home/pm/groups/trip-2026");
    assert!(r.bot_listens("a") && !r.bot_listens("c"));
}

#[test]
fn a_folder_can_never_be_a_path() {
    for bad in ["../etc", "a/b", r"a\b", "/home", "-x", "_x", "a b", "ünï", &"x".repeat(41), ".hidden", "a..b", "C:"] {
        assert_eq!(clean_folder(bad), Err(RoomError::BadFolder), "{bad}");
    }
    assert_eq!(clean_folder("  "), Ok(String::new()));
    assert_eq!(clean_folder("Trip_2026-a"), Ok("trip_2026-a".to_string()));
}

#[test]
fn one_bad_part_changes_nothing_and_not_every_bot_can_be_off() {
    let mut r = room("r0123456789ab", &["a", "b"]);
    let before = r.clone();
    assert_eq!(apply_settings(&mut r, "x", "../x", "listen", &[]), Err(RoomError::BadFolder));
    assert_eq!(apply_settings(&mut r, "x", "", "shout", &[]), Err(RoomError::BadMode));
    assert_eq!(apply_settings(&mut r, "x", "", "listen", &ids(&["a", "b"])), Err(RoomError::AllOff));
    assert_eq!(r, before);
}

#[test]
fn instructions_lose_secrets_control_characters_and_everything_past_2000_characters() {
    let long = "y".repeat(3000);
    assert_eq!(clean_instructions(&long).chars().count(), MAX_INSTRUCTIONS);
    assert!(!clean_instructions("key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 end").contains("abcdefghijklmnop"));
}

#[test]
fn removing_a_member_also_removes_it_from_the_switched_off_list() {
    let mut r = room("r0123456789ab", &["a", "b", "c"]);
    r.off_bots = ids(&["c"]);
    set_members(&mut r, ids(&["a", "b"]));
    assert!(r.off_bots.is_empty());
    assert_eq!(r.bot_ids, ids(&["a", "b"]));
}

#[test]
fn a_hand_off_line_is_saved_with_its_card_and_old_lines_have_none() {
    let dir = temp("handoff_line");
    let mut l = line("l0000000000a1", "@Mochi look it up", 5);
    l.handoff = Some(Handoff { to_id: "mochi".into(), to_name: "Mochi".into(), task: "look it up".into(), passed: true });
    append_line(&dir, "r0123456789ab", l.clone(), None).unwrap();
    append_line(&dir, "r0123456789ab", line("l0000000000a2", "plain", 6), None).unwrap();
    let got = read_lines(&dir, "r0123456789ab", 10);
    assert_eq!(got[0].handoff, l.handoff);
    assert_eq!(got[1].handoff, None);
}
