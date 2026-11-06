//! A job's file changes through the engine: the bot's `write_file` and `edit_file` keep the old content while a job runs,
//! the job lists the files, Restore puts them back, and a terminal command or a chat without a job is not covered. Fakes only.

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;

use super::file_undo::State;
use super::sandbox::lease::ScreenLease;
use super::sandbox::FakeSandbox;
use super::sandbox_calls::Port;
use super::service_card_tests::{code, ok, rig, running, terminal, tool, DM};
use super::service_tests::Rig;

fn up(name: &str) -> (Rig, Arc<FakeSandbox>) {
    let (rig, fake) = rig(name);
    fake.use_file_system();
    (rig, fake)
}

fn write(rig: &Rig, id: &str, path: &str, content: &str) -> serde_json::Value {
    tool(rig, DM, id, "write_file", json!({"path": path, "content": content}))
}

#[test]
fn a_write_in_a_job_keeps_the_old_file_and_restore_puts_it_back() {
    let (rig, fake) = up("files-write");
    fake.put_file("/home/pm/plan.md", b"the first plan");
    running(&rig);
    assert!(ok(&write(&rig, "w-1", "plan.md", "a different plan")));
    assert!(ok(&write(&rig, "w-2", "plan.md", "and another one")));
    assert_eq!(fake.file("/home/pm/plan.md").unwrap(), b"and another one");

    let view = rig.svc.job_files("nimbus").unwrap().expect("the job lists its files");
    assert_eq!(view.files.len(), 1, "two writes to one file are one entry");
    assert_eq!((view.files[0].path.as_str(), view.files[0].state, view.files[0].op.as_str()), ("/home/pm/plan.md", State::Saved, "write"));

    let done = rig.svc.restore_job_file("nimbus", &view.job_id, &view.files[0].id).unwrap();
    assert!(done.restored);
    assert_eq!(fake.file("/home/pm/plan.md").unwrap(), b"the first plan", "the file from before the job, not from between the writes");
    assert!(rig.svc.job_files("nimbus").unwrap().unwrap().files[0].restored);
}

#[test]
fn an_edit_keeps_the_old_file_too_and_an_edit_that_changes_nothing_lists_nothing() {
    let (rig, fake) = up("files-edit");
    fake.put_file("/home/pm/todo.txt", b"buy milk");
    running(&rig);
    let missed = tool(&rig, DM, "e-1", "edit_file", json!({"path": "todo.txt", "old": "buy bread", "new": "buy tea"}));
    assert_eq!(code(&missed), "EDIT_TEXT_NOT_FOUND");
    assert!(rig.svc.job_files("nimbus").unwrap().unwrap().files.is_empty(), "nothing was changed, so nothing is listed");
    assert!(ok(&tool(&rig, DM, "e-2", "edit_file", json!({"path": "todo.txt", "old": "milk", "new": "tea"}))));
    let view = rig.svc.job_files("nimbus").unwrap().unwrap();
    assert_eq!((view.files[0].op.as_str(), view.files[0].state), ("edit", State::Saved));
    rig.svc.restore_job_file("nimbus", &view.job_id, &view.files[0].id).unwrap();
    assert_eq!(fake.file("/home/pm/todo.txt").unwrap(), b"buy milk");
}

#[test]
fn a_file_the_job_made_is_removed_by_restore() {
    let (rig, fake) = up("files-created");
    running(&rig);
    assert!(ok(&write(&rig, "w-1", "out/new.txt", "hello")));
    let view = rig.svc.job_files("nimbus").unwrap().unwrap();
    assert_eq!(view.files[0].state, State::Created);
    rig.svc.restore_job_file("nimbus", &view.job_id, &view.files[0].id).unwrap();
    assert!(fake.file("/home/pm/out/new.txt").is_none());
}

#[test]
fn a_file_over_the_cap_is_listed_without_a_copy_and_restore_says_so() {
    let (rig, fake) = up("files-big");
    fake.put_file("/home/pm/huge.bin", &vec![9u8; 600 * 1024]);
    running(&rig);
    assert!(ok(&write(&rig, "w-1", "huge.bin", "small now")));
    let view = rig.svc.job_files("nimbus").unwrap().unwrap();
    assert_eq!(view.files[0].state, State::TooBig);
    let refused = rig.svc.restore_job_file("nimbus", &view.job_id, &view.files[0].id).unwrap_err();
    assert_eq!(refused.code, "POTATO_FILE_UNDO_NOT_SAVED");
}

#[test]
fn what_a_terminal_command_changes_and_what_happens_with_no_job_is_not_covered() {
    let (rig, fake) = up("files-not-covered");
    fake.put_file("/home/pm/a.txt", b"one");
    // no job yet: a write is not kept
    rig.svc.send("nimbus", "just write a file").unwrap();
    assert!(ok(&write(&rig, "w-0", "a.txt", "two")));
    assert!(rig.svc.job_files("nimbus").unwrap().is_none(), "no job, no list");
    // a job: a terminal command that overwrites a file leaves no entry (the page says so)
    running(&rig);
    assert!(ok(&terminal(&rig, "t-1", "echo three > a.txt")));
    assert!(rig.svc.job_files("nimbus").unwrap().unwrap().files.is_empty());
    // a file outside the bot's home is not kept either
    assert!(ok(&write(&rig, "w-1", "/tmp/elsewhere.txt", "x")));
    assert!(rig.svc.job_files("nimbus").unwrap().unwrap().files.is_empty());
}

#[test]
fn restore_is_for_the_owner_in_the_app_the_model_has_no_tool_and_a_wrong_id_finds_nothing() {
    let (rig, fake) = up("files-owner");
    fake.put_file("/home/pm/a.txt", b"old");
    running(&rig);
    assert!(ok(&write(&rig, "w-1", "a.txt", "new")));
    for name in ["restore_file", "undo_file", "file_restore"] {
        assert_eq!(code(&tool(&rig, DM, &format!("x-{name}"), name, json!({"id": "f-1"}))), "UNKNOWN_TOOL");
    }
    assert_eq!(fake.file("/home/pm/a.txt").unwrap(), b"new");
    let view = rig.svc.job_files("nimbus").unwrap().unwrap();
    assert_eq!(rig.svc.restore_job_file("nimbus", &view.job_id, "f-9").unwrap_err().code, "POTATO_FILE_UNDO_NOT_FOUND");
    assert!(rig.svc.restore_job_file("nimbus", "job-../x", "f-1").is_err());
}

#[test]
fn restore_without_a_computer_says_so_and_changes_nothing() {
    let (rig, fake) = up("files-no-computer");
    fake.put_file("/home/pm/a.txt", b"old");
    running(&rig);
    assert!(ok(&write(&rig, "w-1", "a.txt", "new")));
    let view = rig.svc.job_files("nimbus").unwrap().unwrap();
    rig.svc.set_sandbox_port(None);
    let refused = rig.svc.restore_job_file("nimbus", &view.job_id, &view.files[0].id).unwrap_err();
    assert_eq!(refused.code, "SANDBOX_NOT_SET_UP");
    assert!(!rig.svc.job_files("nimbus").unwrap().unwrap().files[0].restored);
    // with the computer back, it works
    rig.svc.set_sandbox_port(Some(Port { sandbox: fake.clone(), lease: Arc::new(Mutex::new(ScreenLease::new())) }));
    rig.svc.restore_job_file("nimbus", &view.job_id, &view.files[0].id).unwrap();
    assert_eq!(fake.file("/home/pm/a.txt").unwrap(), b"old");
}
