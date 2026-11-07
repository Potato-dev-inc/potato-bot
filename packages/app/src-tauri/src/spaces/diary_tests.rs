//! The space diary: lines go where the spec says, one object per line, deletes take them out, day files are capped,
//! old files go.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::diary::{self, JournalLine, DAY_FILE_MAX_BYTES, KEEP_MS};
use super::ids::SpaceId;

const T0: u64 = 1_700_000_000_000;
const DAY: u64 = 24 * 60 * 60 * 1000;

struct Vault(PathBuf);

impl Vault {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("pm-diary-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for Vault {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn space() -> SpaceId { SpaceId([0xab; 16]) }

fn line(id: &str, at: u64, text: &str) -> JournalLine {
    JournalLine { id: id.into(), at, who: "Sam".into(), via: diary::VIA_SPACE.into(), channel: "general".into(), text: text.into() }
}

#[test]
fn a_line_lands_in_the_day_file_of_the_space() {
    let v = Vault::new("path");
    assert!(diary::append(&v.0, &space(), &line("aa01", T0, "hello there")).unwrap());
    let day = diary::day_of(T0);
    let file = v.0.join("spaces").join("ab".repeat(16)).join("journal").join(format!("{day}.jsonl"));
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 1);
    let obj: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(obj["id"], "aa01");
    assert_eq!(obj["at"], T0);
    assert_eq!(obj["who"], "Sam");
    assert_eq!(obj["via"], "Space");
    assert_eq!(obj["channel"], "general");
    assert_eq!(obj["text"], "hello there");
    assert_eq!(obj.as_object().unwrap().len(), 6, "exactly the six fields of the spec");
}

#[test]
fn a_message_with_line_breaks_is_still_one_line() {
    let v = Vault::new("lines");
    diary::append(&v.0, &space(), &line("aa01", T0, "one\ntwo\r\nthree \"quoted\" \u{2028}")).unwrap();
    diary::append(&v.0, &space(), &line("aa02", T0 + 1, "next")).unwrap();
    let all = diary::read_all(&v.0, &space());
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].text, "one\ntwo\r\nthree \"quoted\" \u{2028}");
}

#[test]
fn a_bad_message_id_is_refused_so_it_cannot_make_a_path_or_a_wrong_match() {
    let v = Vault::new("badid");
    assert!(diary::append(&v.0, &space(), &line("../x", T0, "t")).is_err());
    assert!(diary::append(&v.0, &space(), &line("", T0, "t")).is_err());
    assert_eq!(diary::remove(&v.0, &space(), "../x").unwrap(), 0);
}

#[test]
fn delete_takes_only_that_message_out_in_every_day_file() {
    let v = Vault::new("delete");
    diary::append(&v.0, &space(), &line("aa01", T0, "keep one")).unwrap();
    diary::append(&v.0, &space(), &line("aa02", T0 + 1, "remove me")).unwrap();
    diary::append(&v.0, &space(), &line("aa03", T0 + DAY, "next day")).unwrap();
    diary::append(&v.0, &space(), &line("aa02", T0 + DAY + 1, "same id elsewhere")).unwrap();
    assert_eq!(diary::remove(&v.0, &space(), "aa02").unwrap(), 2);
    let left: Vec<String> = diary::read_all(&v.0, &space()).into_iter().map(|l| l.id).collect();
    assert_eq!(left, vec!["aa01".to_string(), "aa03".to_string()]);
    // a message that is not there changes nothing
    assert_eq!(diary::remove(&v.0, &space(), "aa99").unwrap(), 0);
}

#[test]
fn a_day_left_empty_loses_its_file() {
    let v = Vault::new("empty");
    diary::append(&v.0, &space(), &line("aa01", T0, "only line")).unwrap();
    assert_eq!(diary::remove(&v.0, &space(), "aa01").unwrap(), 1);
    assert!(diary::read_all(&v.0, &space()).is_empty());
    let dir = diary::journal_dir(&v.0, &space());
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
}

#[test]
fn a_day_file_stops_at_two_megabytes() {
    let v = Vault::new("cap");
    let big = "x".repeat(60_000);
    let mut stored = 0usize;
    for i in 0..60 {
        if diary::append(&v.0, &space(), &line(&format!("{i:04x}"), T0, &big)).unwrap() { stored += 1; }
    }
    let file = diary::journal_dir(&v.0, &space()).join(format!("{}.jsonl", diary::day_of(T0)));
    let size = std::fs::metadata(&file).unwrap().len();
    assert!(size <= DAY_FILE_MAX_BYTES, "{size}");
    assert!(stored < 60 && stored >= 30, "{stored}");
    // another day still takes lines
    assert!(diary::append(&v.0, &space(), &line("ffff", T0 + 3 * DAY, "fresh day")).unwrap());
}

#[test]
fn files_older_than_ninety_days_go_and_newer_ones_stay() {
    let v = Vault::new("prune");
    let now = T0 + 200 * DAY;
    diary::append(&v.0, &space(), &line("aa01", now - KEEP_MS - 2 * DAY, "old")).unwrap();
    diary::append(&v.0, &space(), &line("aa02", now - KEEP_MS + 2 * DAY, "recent")).unwrap();
    diary::append(&v.0, &space(), &line("aa03", now, "today")).unwrap();
    assert_eq!(diary::prune(&v.0, &space(), now), 1);
    let ids: Vec<String> = diary::read_all(&v.0, &space()).into_iter().map(|l| l.id).collect();
    assert_eq!(ids, vec!["aa02".to_string(), "aa03".to_string()]);
}

#[test]
fn two_spaces_do_not_share_a_folder() {
    let v = Vault::new("two");
    let other = SpaceId([0xcd; 16]);
    diary::append(&v.0, &space(), &line("aa01", T0, "here")).unwrap();
    diary::append(&v.0, &other, &line("aa02", T0, "there")).unwrap();
    assert_eq!(diary::read_all(&v.0, &space()).len(), 1);
    assert_eq!(diary::remove(&v.0, &other, "aa01").unwrap(), 0, "the other space has no such line");
    assert_eq!(diary::read_all(&v.0, &space()).len(), 1);
}
