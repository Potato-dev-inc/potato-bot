//! Undo for a bot's file changes in a job: the first change to a file keeps the old content, a created file is removed by
//! Restore, a big file or a long job is listed as not saved, and nothing outside the bot's home or the job's folder is reached.

use std::path::PathBuf;

use super::file_undo::*;
use super::sandbox::{FakeSandbox, Recorded};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-fileundo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn computer() -> FakeSandbox {
    let fake = FakeSandbox::new();
    fake.use_file_system();
    fake
}

const JOB: &str = "job-0a1b2c3d";

fn keep(root: &PathBuf, fake: &FakeSandbox, path: &str, op: &str, at: i64) {
    snapshot(root, "fin", JOB, fake, path, op, at);
}

#[test]
fn the_first_change_keeps_the_old_content_and_later_changes_do_not_replace_it() {
    let root = temp("first");
    let fake = computer();
    fake.put_file("/home/pm/notes.txt", b"version 0");
    keep(&root, &fake, "/home/pm/notes.txt", "write", 10);
    // the bot writes, then writes again
    fake.put_file("/home/pm/notes.txt", b"version 1");
    keep(&root, &fake, "/home/pm/notes.txt", "edit", 20);
    let files = list(&root, "fin", JOB).unwrap();
    assert_eq!(files.len(), 1, "one entry for one file");
    assert_eq!((files[0].path.as_str(), files[0].state, files[0].bytes, files[0].at, files[0].op.as_str()), ("/home/pm/notes.txt", State::Saved, 9, 10, "write"));
    // Restore brings back "version 0", not "version 1"
    fake.put_file("/home/pm/notes.txt", b"version 2");
    let done = restore(&root, "fin", JOB, &files[0].id, &fake).unwrap();
    assert!(done.restored);
    assert_eq!(fake.file("/home/pm/notes.txt").unwrap(), b"version 0");
    assert!(list(&root, "fin", JOB).unwrap()[0].restored);
}

#[test]
fn a_file_the_job_made_is_listed_as_created_and_restore_removes_it() {
    let root = temp("created");
    let fake = computer();
    keep(&root, &fake, "/home/pm/new/report.md", "write", 5);
    let files = list(&root, "fin", JOB).unwrap();
    assert_eq!((files[0].state, files[0].bytes), (State::Created, 0));
    fake.put_file("/home/pm/new/report.md", b"made by the job");
    restore(&root, "fin", JOB, &files[0].id, &fake).unwrap();
    assert!(fake.file("/home/pm/new/report.md").is_none());
    assert!(fake.calls().contains(&Recorded::Shell("rm -f -- '/home/pm/new/report.md'".to_string())));
}

#[test]
fn a_binary_file_comes_back_byte_for_byte() {
    let root = temp("binary");
    let fake = computer();
    let bytes: Vec<u8> = (0..=255u8).collect();
    fake.put_file("/home/pm/data.bin", &bytes);
    keep(&root, &fake, "/home/pm/data.bin", "write", 1);
    fake.put_file("/home/pm/data.bin", b"text");
    restore(&root, "fin", JOB, &list(&root, "fin", JOB).unwrap()[0].id, &fake).unwrap();
    assert_eq!(fake.file("/home/pm/data.bin").unwrap(), bytes);
}

#[test]
fn a_file_over_the_cap_is_listed_as_not_saved_and_cannot_be_restored() {
    let root = temp("big");
    let fake = computer();
    fake.put_file("/home/pm/big.bin", &vec![7u8; MAX_SNAPSHOT_BYTES + 10]);
    keep(&root, &fake, "/home/pm/big.bin", "write", 1);
    let files = list(&root, "fin", JOB).unwrap();
    assert_eq!(files[0].state, State::TooBig);
    assert_eq!(restore(&root, "fin", JOB, &files[0].id, &fake).unwrap_err().code, "POTATO_FILE_UNDO_NOT_SAVED");
    assert!(!root.join("file-undo").join("fin").join(JOB).join(format!("{}.bin", files[0].id)).exists(), "nothing big was copied");
    // a file exactly at the cap is kept
    fake.put_file("/home/pm/edge.bin", &vec![1u8; MAX_SNAPSHOT_BYTES]);
    keep(&root, &fake, "/home/pm/edge.bin", "write", 2);
    assert_eq!(list(&root, "fin", JOB).unwrap()[1].state, State::Saved);
}

#[test]
fn a_long_job_stops_keeping_files_at_the_limits_and_says_so() {
    let root = temp("limits");
    let fake = computer();
    // by total size: 16 files of the cap fill 8 MB, the 17th is not kept
    for n in 0..17 {
        let path = format!("/home/pm/f{n}.bin");
        fake.put_file(&path, &vec![n as u8; MAX_SNAPSHOT_BYTES]);
        keep(&root, &fake, &path, "write", n);
    }
    let files = list(&root, "fin", JOB).unwrap();
    assert_eq!(files.iter().filter(|e| e.state == State::Saved).count(), 16);
    assert_eq!(files[16].state, State::OverLimit);
    // by count: small files, the 101st entry is not kept
    let root = temp("limits-count");
    for n in 0..(MAX_FILES + 3) {
        let path = format!("/home/pm/s{n}.txt");
        fake.put_file(&path, b"x");
        keep(&root, &fake, &path, "write", n as i64);
    }
    let files = list(&root, "fin", JOB).unwrap();
    assert_eq!(files.len(), MAX_FILES);
    assert_eq!(files.iter().filter(|e| e.state == State::Saved).count(), MAX_FILES);
    assert!(view(&root, "fin", JOB).unwrap().overflow, "the page is told that some files are not in the list");
}

#[test]
fn only_files_in_the_bots_home_are_kept() {
    let root = temp("covered");
    let fake = computer();
    for path in ["/etc/passwd", "/home/pm", "/home/pm/", "/home/other/x", "/home/pm/../etc/passwd", "/tmp/x", "relative.txt"] {
        fake.put_file(path, b"x");
        keep(&root, &fake, path, "write", 1);
        assert!(!covered(path), "{path}");
    }
    assert!(list(&root, "fin", JOB).unwrap().is_empty());
    assert!(covered("/home/pm/a/b.txt"));
}

#[test]
fn a_job_or_bot_name_cannot_point_outside_the_undo_folder() {
    let root = temp("names");
    let fake = computer();
    fake.put_file("/home/pm/a.txt", b"x");
    for job in ["../../outside", "job-../x", "", "job-a/b", "x"] {
        snapshot(&root, "fin", job, &fake, "/home/pm/a.txt", "write", 1);
        assert!(list(&root, "fin", job).is_err(), "{job}");
    }
    snapshot(&root, "../fin", JOB, &fake, "/home/pm/a.txt", "write", 1);
    assert!(list(&root, "../fin", JOB).is_err());
    assert!(!root.join("file-undo").exists(), "nothing was made");
}

#[test]
fn restore_of_an_unknown_file_or_a_damaged_list_is_an_error_and_changes_nothing() {
    let root = temp("damaged");
    let fake = computer();
    fake.put_file("/home/pm/a.txt", b"old");
    keep(&root, &fake, "/home/pm/a.txt", "write", 1);
    assert_eq!(restore(&root, "fin", JOB, "f-99", &fake).unwrap_err().code, "POTATO_FILE_UNDO_NOT_FOUND");
    let index = root.join("file-undo").join("fin").join(JOB).join("index.json");
    std::fs::write(&index, "{ nope").unwrap();
    assert!(list(&root, "fin", JOB).is_err());
    keep(&root, &fake, "/home/pm/b.txt", "write", 2);
    assert_eq!(std::fs::read_to_string(&index).unwrap(), "{ nope", "a damaged list is never overwritten");
}

#[test]
fn a_computer_that_cannot_be_read_keeps_nothing_and_does_not_stop_the_change() {
    let root = temp("down");
    let fake = FakeSandbox::not_installed();
    snapshot(&root, "fin", JOB, &fake, "/home/pm/a.txt", "write", 1);
    assert!(list(&root, "fin", JOB).unwrap().is_empty());
}

#[test]
fn only_the_newest_jobs_keep_their_folders() {
    let root = temp("prune");
    let fake = computer();
    fake.put_file("/home/pm/a.txt", b"x");
    for n in 0..(KEEP_JOBS + 3) {
        std::thread::sleep(std::time::Duration::from_millis(15));
        snapshot(&root, "fin", &format!("job-0000000{n}"), &fake, "/home/pm/a.txt", "write", n as i64);
    }
    let folders = std::fs::read_dir(root.join("file-undo").join("fin")).unwrap().count();
    assert_eq!(folders, KEEP_JOBS);
    assert!(list(&root, "fin", &format!("job-0000000{}", KEEP_JOBS + 2)).unwrap().len() == 1, "the newest is there");
}
