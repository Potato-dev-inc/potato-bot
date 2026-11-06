//! Live steering in the engine: a message written during a turn is shown as "Queued" until the bot picks it up, nothing is
//! lost across a restart of a job, a status question during a slow tool call gets the app's own note, and the step list
//! keeps what each step did (scrubbed). Fakes only.

use std::sync::Arc;

use serde_json::{json, Value};

use super::jobs;
use super::protocol::{parse_event, Event, ProgressPhase, StepInfo};
use super::service::*;
use super::service_tests::Rig;
use super::store::{FromKind, MsgKind};
use super::work::WorkRun;

const DM: &str = "app:dm:nimbus";
// A fake token, built from pieces so no scanner mistakes this file for a leak.
fn fake_token() -> String {
    ["ghp", "_", "AbCdEf0123456789AbCdEf0123456789"].concat()
}

fn rig(name: &str) -> Rig {
    let rig = Rig::ready(name);
    rig.svc.set_spawner(Arc::new(|job| job()));
    rig.answer_all();
    rig
}

fn typing_on(rig: &Rig) {
    rig.post(vec![json!({"v":1,"type":"typing","conversationId":DM,"on":true})]);
}

fn queued_ids(rig: &Rig) -> Vec<String> {
    rig.thread().into_iter().filter(|m| m.queued).map(|m| m.text).collect()
}

fn start(rig: &Rig, id: &str, tool: &str, detail: &str) {
    rig.post(vec![json!({"v":1,"type":"progress","conversationId":DM,"id":id,"tool":tool,"phase":"start","detail":detail})]);
}

#[test]
fn a_message_sent_during_a_turn_is_queued_until_the_bot_picks_it_up() {
    let rig = rig("queued_label");
    let first = rig.svc.send("nimbus", "do the big thing").unwrap();
    typing_on(&rig);
    assert!(queued_ids(&rig).is_empty(), "the first message starts the turn; it waits for nothing");
    let second = rig.svc.send("nimbus", "also check the logs").unwrap();
    assert_eq!(queued_ids(&rig), vec!["also check the logs".to_string()]);
    // the bot reads it between two tool rounds
    rig.post(vec![json!({"v":1,"type":"picked","conversationId":DM,"ids":[second.id]})]);
    assert!(queued_ids(&rig).is_empty(), "picked up: the label is gone");
    // a third one is never picked, but the turn's reply still clears it (it ran as the next turn)
    let third = rig.svc.send("nimbus", "and one more").unwrap();
    assert_eq!(queued_ids(&rig).len(), 1);
    rig.post(vec![json!({"v":1,"type":"reply","conversationId":DM,"inReplyTo":third.id,"segments":["done"]})]);
    assert!(queued_ids(&rig).is_empty());
    let _ = first;
}

#[test]
fn a_message_sent_when_nothing_runs_is_not_queued() {
    let rig = rig("not_queued");
    rig.svc.send("nimbus", "hello again").unwrap();
    assert!(queued_ids(&rig).is_empty());
}

#[test]
fn a_failed_turn_clears_the_label_too() {
    let rig = rig("queued_error");
    rig.svc.send("nimbus", "start").unwrap();
    typing_on(&rig);
    let second = rig.svc.send("nimbus", "wait for me").unwrap();
    assert_eq!(queued_ids(&rig).len(), 1);
    rig.post(vec![json!({"v":1,"type":"error","code":"INTERNAL","conversationId":DM,"inReplyTo":second.id})]);
    assert!(queued_ids(&rig).is_empty());
}

// ---- the app's own note for a status question -----------------------------------------------------------------------

#[test]
fn a_status_question_during_a_slow_tool_call_gets_the_apps_note() {
    let rig = rig("status_note");
    rig.svc.send("nimbus", "build it").unwrap();
    typing_on(&rig);
    start(&rig, "s1", "run_terminal", "Ran: cargo build --release");
    rig.advance(25_000);
    let asked = rig.svc.send("nimbus", "status?").unwrap();
    let notes = rig.notes();
    let note = notes.iter().find(|n| n.contains("note from the app")).unwrap_or_else(|| panic!("no note in {notes:?}"));
    assert!(note.contains("cargo build --release") && note.contains("running 25s"), "{note}");
    assert!(note.contains("note from the app, not from"), "marked as the app's own words: {note}");
    // the message itself still goes to the bot (queued, read between steps) and nothing was answered for it
    assert!(rig.thread().iter().any(|m| m.id == asked.id && m.queued));
    // asking again at once does not fill the thread
    rig.svc.send("nimbus", "status??").unwrap();
    assert_eq!(rig.notes().iter().filter(|n| n.contains("note from the app")).count(), 1);
}

#[test]
fn a_status_question_during_a_short_step_gets_no_note() {
    let rig = rig("status_short");
    rig.svc.send("nimbus", "build it").unwrap();
    typing_on(&rig);
    start(&rig, "s1", "run_terminal", "Ran: ls");
    rig.advance(5_000);
    rig.svc.send("nimbus", "status?").unwrap();
    assert!(!rig.notes().iter().any(|n| n.contains("note from the app")));
}

#[test]
fn an_ordinary_message_during_a_slow_tool_call_gets_no_note() {
    let rig = rig("status_not");
    rig.svc.send("nimbus", "build it").unwrap();
    typing_on(&rig);
    start(&rig, "s1", "run_terminal", "Ran: sleep 100");
    rig.advance(60_000);
    rig.svc.send("nimbus", "please also rename the folder").unwrap();
    assert!(!rig.notes().iter().any(|n| n.contains("note from the app")));
}

#[test]
fn the_status_note_names_the_job_from_its_saved_record() {
    let rig = rig("status_job");
    rig.svc.send("nimbus", "do the big thing").unwrap();
    rig.post(vec![json!({"v":1,"type":"sandbox_call","callId":"j1","conversationId":DM,"tool":"job_start","args":{"goal":"Tidy the project folder"},"image":false})]);
    typing_on(&rig);
    start(&rig, "s1", "run_terminal", "Ran: npm install");
    rig.advance(90_000);
    rig.svc.send("nimbus", "how is it going?").unwrap();
    let note = rig.notes().into_iter().find(|n| n.contains("note from the app")).expect("a note");
    assert!(note.contains("Working on: Tidy the project folder") && note.contains("npm install"), "{note}");
}

// ---- nothing is lost when the app restarts in the middle of a job -------------------------------------------------

#[test]
fn a_waiting_message_survives_a_restart_of_a_running_job() {
    let rig = rig("restart_queue");
    let first = rig.svc.send("nimbus", "do the big thing").unwrap();
    rig.post(vec![json!({"v":1,"type":"sandbox_call","callId":"j1","conversationId":DM,"tool":"job_start","args":{"goal":"Tidy the project folder"},"image":false})]);
    rig.post(vec![json!({"v":1,"type":"reply","conversationId":DM,"inReplyTo":first.id,"segments":["working"]}), json!({"v":1,"type":"usage","model":"deepseek-flash","promptTokens":0,"completionTokens":10,"cacheHitTokens":0,"cacheMissTokens":0})]);
    typing_on(&rig);
    let mine = rig.svc.send("nimbus", "also tell me when the tests pass").unwrap();
    let saved = jobs::load(&rig.root, "nimbus").unwrap();
    assert_eq!(saved.queued.len(), 1, "kept with the job: {saved:?}");
    assert_eq!(saved.queued[0].id, mine.id);
    // the app quits and starts again on the same folders
    let svc = Service::new(rig.root.clone(), rig.host.clone(), rig.launcher.clone(), rig.brains.clone());
    svc.tick();
    let items = svc.inbox_poll("nimbus", 0, 0)["items"].as_array().cloned().unwrap_or_default();
    let sends: Vec<&Value> = items.iter().filter(|i| i["type"] == "send").collect();
    assert_eq!(sends.len(), 2, "the job turn and the waiting message: {sends:?}");
    assert!(sends[0]["text"].as_str().unwrap().starts_with("Keep working on your job."));
    assert_eq!(sends[1]["id"], json!(mine.id), "the same id it has in the thread");
    assert_eq!(sends[1]["text"], json!("also tell me when the tests pass"));
    assert_eq!(sends[1]["from"]["trust"], json!("owner_pc"));
    // it is in the thread once, not twice
    let thread = svc.messages("nimbus", 500).unwrap();
    assert_eq!(thread.iter().filter(|m| m.text == "also tell me when the tests pass").count(), 1);
    // once the bot picks it up it is not kept any more
    let events: Vec<Event> = [json!({"v":1,"type":"picked","conversationId":DM,"ids":[mine.id]})].iter().filter_map(parse_event).collect();
    svc.ingest("nimbus", events);
    assert!(jobs::load(&rig.root, "nimbus").unwrap().queued.is_empty());
    assert!(svc.messages("nimbus", 500).unwrap().iter().all(|m| !m.queued));
}

#[test]
fn nothing_is_kept_for_a_chat_without_a_job() {
    let rig = rig("no_job_queue");
    rig.svc.send("nimbus", "start").unwrap();
    typing_on(&rig);
    rig.svc.send("nimbus", "second").unwrap();
    assert!(jobs::load(&rig.root, "nimbus").is_none());
}

// ---- the step list ------------------------------------------------------------------------------------------------

fn info(detail: &str) -> StepInfo {
    StepInfo { detail: Some(detail.into()), ..StepInfo::default() }
}

#[test]
fn durations_are_the_real_time_between_start_and_end_and_a_reused_id_is_a_new_row() {
    let mut run = WorkRun::new(0);
    run.apply_with("a", "run_terminal", ProgressPhase::Start, true, None, &info("Ran: sleep 3"), 1_000);
    run.apply_with("a", "run_terminal", ProgressPhase::End, true, None, &StepInfo::default(), 4_000);
    // the same id again (some providers number their calls from 0 every round)
    run.apply_with("a", "run_terminal", ProgressPhase::Start, true, None, &info("Ran: sleep 10"), 5_000);
    run.apply_with("a", "run_terminal", ProgressPhase::End, true, None, &StepInfo::default(), 15_000);
    assert_eq!(run.steps.len(), 2);
    assert_eq!((run.steps[0].started_ms, run.steps[0].ended_ms), (1_000, Some(4_000)));
    assert_eq!((run.steps[1].started_ms, run.steps[1].ended_ms), (5_000, Some(15_000)));
    assert_eq!(run.steps[0].detail.as_deref(), Some("Ran: sleep 3"));
}

#[test]
fn what_a_step_did_is_scrubbed_again_here_and_cut() {
    let token = fake_token();
    let mut run = WorkRun::new(0);
    let long = format!("Ran: echo {}", "word ".repeat(60));
    let loud = StepInfo {
        detail: Some(format!("Ran: curl -H 'Authorization: Bearer {token}' https://example.com")),
        full: Some(format!("export GITHUB_TOKEN={token}\ncurl https://example.com")),
        why: Some(format!("because {token}")),
        ..StepInfo::default()
    };
    run.apply_with("a", "run_terminal", ProgressPhase::Start, true, None, &loud, 0);
    let end = StepInfo { tail: Some((1..=30).map(|i| format!("line {i} {token}")).collect::<Vec<_>>().join("\n")), error: Some(format!("denied for {token}")), exit: Some(1), ..StepInfo::default() };
    run.apply_with("a", "run_terminal", ProgressPhase::End, false, None, &end, 10);
    run.apply_with("b", "run_terminal", ProgressPhase::Start, true, None, &info(&long), 20);
    let wire = serde_json::to_string(&run).unwrap();
    assert!(!wire.contains(&token), "the token reached the thread: {wire}");
    let a = &run.steps[0];
    assert_eq!(a.tail.as_ref().unwrap().lines().count(), 10, "the last 10 lines");
    assert!(a.tail.as_ref().unwrap().contains("line 30") && !a.tail.as_ref().unwrap().contains("line 20 "));
    assert_eq!(a.exit, Some(1));
    assert_eq!(a.status, "error");
    assert!(a.error.is_some());
    let b = run.steps[1].detail.as_ref().unwrap();
    assert!(b.chars().count() <= 120 && b.ends_with('\u{2026}') && !b.contains('\n'), "{b}");
}

#[test]
fn the_progress_event_parses_with_its_new_fields() {
    let value = json!({"v":1,"type":"progress","conversationId":DM,"id":"x","tool":"run_terminal","phase":"end","ok":false,"detail":"Ran: ls","full":"ls -la","why":"look","tail":"a\nb","error":"boom","exit":2});
    match parse_event(&value) {
        Some(Event::Progress { info, ok, .. }) => {
            assert!(!ok);
            assert_eq!(info.detail.as_deref(), Some("Ran: ls"));
            assert_eq!(info.exit, Some(2));
            assert_eq!(info.error.as_deref(), Some("boom"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(parse_event(&json!({"v":1,"type":"picked","conversationId":DM,"ids":["m-1","m-2"]})), Some(Event::Picked { ids, .. }) if ids.len() == 2));
}

#[test]
fn the_view_says_the_work_continues_when_a_job_runs() {
    let rig = rig("continues");
    rig.svc.send("nimbus", "go").unwrap();
    rig.post(vec![json!({"v":1,"type":"sandbox_call","callId":"j1","conversationId":DM,"tool":"job_start","args":{"goal":"Tidy"},"image":false})]);
    typing_on(&rig);
    start(&rig, "s1", "run_terminal", "Ran: ls");
    let view = rig.svc.state().unwrap().bots.into_iter().find(|b| b.id == "nimbus").unwrap();
    assert!(view.work.expect("a list").continues);
    let _ = (FromKind::You, MsgKind::Text);
}
