//! Search chats: words from every bot chat and group thread, newest first, snippet and marks, from_end, the cap, a long
//! history, and the remembered copy that follows a thread when it changes.

use std::path::PathBuf;

use super::rooms::store::{self as rstore, Room, RoomFrom, RoomLine, Speaker};
use super::search::*;
use super::store::{self, FromKind, Message, MsgKind};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-search-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(id: &str) -> String {
    id.to_uppercase()
}

fn say(root: &PathBuf, bot: &str, at: i64, from: FromKind, text: &str) -> String {
    let m = Message::new(at, from, None, MsgKind::Text, text);
    store::append_message(root, &store::app_conversation(bot), m, None).unwrap().id
}

const ROOM: &str = "r0123456789ab";

fn make_room(root: &PathBuf) {
    let room = Room { id: ROOM.into(), name: "Trip".into(), bot_ids: vec!["fin".into(), "nim".into()], created_ms: 1, last_ms: 1, unread: 0, ..Default::default() };
    rstore::save_rooms(root, &[room]).unwrap();
}

fn room_line(root: &PathBuf, id: &str, at: i64, text: &str) {
    let line = RoomLine {
        id: id.into(),
        from: RoomFrom { kind: Speaker::Person, id: "owner".into(), name: "Ren".into() },
        text: text.into(),
        at,
        hop: 0,
        in_reply_to: None,
        file: None,
        handoff: None,
    };
    rstore::append_line(root, ROOM, line, None).unwrap();
}

#[test]
fn finds_words_in_bots_and_groups_newest_first_with_the_bot_and_the_snippet() {
    let root = temp("both");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    store::add_bot(&root, "nim", "potato", 1).unwrap();
    let old = say(&root, "fin", 100, FromKind::You, "Remember the Lisbon trip budget");
    say(&root, "fin", 200, FromKind::Bot, "Something unrelated");
    let mid = say(&root, "nim", 300, FromKind::Bot, "lisbon flights are cheaper on Tuesday");
    make_room(&root);
    room_line(&root, "l-new", 400, "who books the LISBON hotel?");

    let found = run(&root, "  lisbon ", &names).unwrap();
    assert_eq!(found.matches, 3);
    let order: Vec<&str> = found.hits.iter().map(|h| h.message_id.as_str()).collect();
    assert_eq!(order, ["l-new", mid.as_str(), old.as_str()], "newest first across bots and groups");
    assert_eq!((found.hits[0].kind, found.hits[0].owner_name.as_str(), found.hits[0].speaker.as_str()), ("group", "Trip", "Ren"));
    assert_eq!((found.hits[1].kind, found.hits[1].owner_id.as_str(), found.hits[1].owner_name.as_str()), ("bot", "nim", "NIM"));
    assert_eq!(found.hits[2].speaker, "You");
    assert_eq!(found.hits[2].at, 100);
    // the match is marked, whatever its case
    let hit = &found.hits[0];
    let marked: String = hit.snippet.chars().skip(hit.marks[0][0]).take(hit.marks[0][1] - hit.marks[0][0]).collect();
    assert_eq!(marked, "LISBON");
}

#[test]
fn every_word_must_be_there_in_any_order() {
    let root = temp("words");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    say(&root, "fin", 1, FromKind::You, "the blue fish swims");
    say(&root, "fin", 2, FromKind::You, "only a fish here");
    assert_eq!(run(&root, "fish BLUE", &names).unwrap().matches, 1);
    assert_eq!(run(&root, "fish", &names).unwrap().matches, 2);
    assert_eq!(run(&root, "fish red", &names).unwrap().matches, 0);
}

#[test]
fn cards_and_notes_are_not_searched_and_the_query_is_checked() {
    let root = temp("kinds");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    let note = Message::new(1, FromKind::Note, None, MsgKind::Note, "secretword in a note");
    store::append_message(&root, &store::app_conversation("fin"), note, None).unwrap();
    assert_eq!(run(&root, "secretword", &names).unwrap().matches, 0);
    assert_eq!(run(&root, "   ", &names).unwrap_err().code, "POTATO_SEARCH_BAD_QUERY");
    assert_eq!(run(&root, &"x".repeat(MAX_QUERY_CHARS + 1), &names).unwrap_err().code, "POTATO_SEARCH_BAD_QUERY");
}

#[test]
fn from_end_counts_the_messages_after_the_hit_so_the_thread_can_be_read_back_that_far() {
    let root = temp("fromend");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    say(&root, "fin", 1, FromKind::You, "needle");
    for i in 0..7 {
        say(&root, "fin", 10 + i, FromKind::Bot, "hay");
    }
    assert_eq!(run(&root, "needle", &names).unwrap().hits[0].from_end, 7);
}

#[test]
fn a_long_history_is_scanned_once_and_the_hits_are_capped() {
    let root = temp("long");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    // 4000 messages, every fourth one matches
    for i in 0..4000 {
        let text = if i % 4 == 0 { format!("message {i} with the needle inside") } else { format!("message {i} with hay") };
        say(&root, "fin", i, FromKind::You, &text);
    }
    let first = run(&root, "needle", &names).unwrap();
    assert_eq!(first.matches, 1000);
    assert_eq!(first.hits.len(), MAX_HITS, "the page gets 50, not a thousand");
    assert_eq!(first.searched, 4000);
    assert_eq!(first.hits[0].at, 3996, "the newest match is first");
    // a second search reuses the remembered copy: the thread is not read again
    let before = cached_threads();
    let again = run(&root, "needle message", &names).unwrap();
    assert_eq!(cached_threads(), before);
    assert_eq!(again.matches, 1000);
}

#[test]
fn a_new_message_is_found_at_once_because_the_copy_follows_the_file() {
    let root = temp("fresh");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    say(&root, "fin", 1, FromKind::You, "hello");
    assert_eq!(run(&root, "latecomer", &names).unwrap().matches, 0);
    say(&root, "fin", 2, FromKind::You, "the latecomer arrived");
    assert_eq!(run(&root, "latecomer", &names).unwrap().matches, 1);
}

#[test]
fn a_long_message_shows_a_short_snippet_around_the_match() {
    let root = temp("snippet");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    let text = format!("{} needle {}", "a".repeat(300), "b".repeat(300));
    say(&root, "fin", 1, FromKind::You, &text);
    let hit = run(&root, "needle", &names).unwrap().hits.remove(0);
    let len = hit.snippet.chars().count();
    assert!(len < 160, "{len}");
    assert!(hit.snippet.starts_with('…') && hit.snippet.ends_with('…'));
    let mark = hit.marks[0];
    let marked: String = hit.snippet.chars().skip(mark[0]).take(mark[1] - mark[0]).collect();
    assert_eq!(marked, "needle");
}

#[test]
fn text_in_another_script_and_with_odd_capitals_keeps_its_marks_in_place() {
    let root = temp("script");
    store::add_bot(&root, "fin", "fish", 1).unwrap();
    say(&root, "fin", 1, FromKind::You, "İstanbul 旅行 計畫 and ÉCOLE");
    let hit = run(&root, "計畫 école", &names).unwrap().hits.remove(0);
    let marked: Vec<String> = hit.marks.iter().map(|m| hit.snippet.chars().skip(m[0]).take(m[1] - m[0]).collect()).collect();
    assert_eq!(marked, ["計畫", "ÉCOLE"]);
}
