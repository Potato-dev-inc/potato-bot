//! The live step list: steps go running then done, a retry is its own step, a failure puts the X on the step the
//! turn stopped at, and a log line is redacted and cut. The `progress` wire event parses.

use std::collections::HashMap;

use serde_json::json;

use super::protocol::{parse_event, Event, ProgressPhase};
use super::work::{shown, WorkRun};

#[test]
fn a_step_goes_running_then_done_with_its_times() {
    let mut run = WorkRun::new(1000);
    run.apply("s1", "computer_observe", ProgressPhase::Start, true, Some("Started computer_observe"), 1100);
    assert_eq!(run.steps[0].status, "running");
    run.apply("s1", "computer_observe", ProgressPhase::End, true, Some("Finished computer_observe"), 3100);
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.steps[0].status, "done");
    assert_eq!(run.steps[0].ended_ms, Some(3100));
    assert_eq!(run.steps[0].log.len(), 2);
}

#[test]
fn a_retry_is_a_step_that_ends_when_work_resumes() {
    let mut run = WorkRun::new(0);
    run.apply("retry-1", "model", ProgressPhase::Retry, true, None, 10);
    assert_eq!(run.steps[0].tool, "retry");
    assert_eq!(run.steps[0].status, "retry");
    run.apply("s2", "browser_act", ProgressPhase::Start, true, None, 20);
    assert_eq!(run.steps[0].status, "done");
}

#[test]
fn a_failed_turn_puts_the_x_on_the_step_it_stopped_at() {
    let mut run = WorkRun::new(0);
    run.apply("a", "browser_act", ProgressPhase::Start, true, None, 1);
    run.apply("a", "browser_act", ProgressPhase::End, true, None, 2);
    run.apply("b", "computer_act", ProgressPhase::Start, true, None, 3);
    run.fail(4);
    assert!(run.failed);
    assert_eq!(run.steps[0].status, "done");
    assert_eq!(run.steps[1].status, "error");
}

#[test]
fn a_log_line_is_one_short_redacted_line_and_only_three_are_kept() {
    let mut run = WorkRun::new(0);
    let long = "x".repeat(300);
    run.apply("a", "browser_act", ProgressPhase::Start, true, Some(&format!("{long}\nsecond line")), 1);
    assert_eq!(run.steps[0].log[0].chars().count(), 80);
    run.apply("a", "browser_act", ProgressPhase::Start, true, Some("Bearer abcdefghijklmnopqrstuvwxyz0123456789"), 2);
    let line = &run.steps[0].log[1];
    assert!(!line.contains("abcdefghijklmnopqrstuvwxyz0123456789"), "{line}");
    for n in 0..5 {
        run.apply("a", "browser_act", ProgressPhase::Start, true, Some(&format!("line {n}")), 3);
    }
    assert_eq!(run.steps[0].log.len(), 3);
    assert_eq!(run.steps[0].log[2], "line 4");
}

#[test]
fn an_ended_run_stays_folded_and_an_empty_one_shows_nothing() {
    let mut work = HashMap::new();
    let mut typing = HashMap::new();
    let mut run = WorkRun::new(0);
    work.insert("app:dm:fish".to_string(), run.clone());
    typing.insert("app:dm:fish".to_string(), 1);
    assert!(shown(&work, &typing, "app:dm:fish").is_none(), "no steps: only the thinking row");
    typing.clear();
    run.apply("a", "browser_act", ProgressPhase::Start, true, None, 1);
    work.insert("app:dm:fish".to_string(), run.clone());
    assert!(shown(&work, &typing, "app:dm:fish").is_none(), "not typing and not ended");
    run.finish(9);
    work.insert("app:dm:fish".to_string(), run.clone());
    assert!(shown(&work, &typing, "app:dm:fish").is_some_and(|w| w.ended_ms == Some(9) && w.steps[0].status == "done"));
    typing.insert("app:dm:fish".to_string(), 1);
    assert!(shown(&work, &typing, "app:dm:fish").is_some());
    typing.clear();
    run.fail(5);
    work.insert("app:dm:fish".to_string(), run);
    assert!(shown(&work, &typing, "app:dm:fish").is_some_and(|w| w.failed));
}

#[test]
fn the_progress_event_parses_and_a_bad_phase_is_dropped() {
    let ok = parse_event(&json!({"v":1,"type":"progress","conversationId":"app:dm:fish","id":"s1","tool":"computer_observe","phase":"end","ok":false,"log":"Failed computer_observe"}));
    assert_eq!(
        ok,
        Some(Event::Progress { conversation_id: "app:dm:fish".into(), id: "s1".into(), tool: "computer_observe".into(), phase: ProgressPhase::End, ok: false, log: Some("Failed computer_observe".into()), info: Default::default() })
    );
    assert!(parse_event(&json!({"v":1,"type":"progress","conversationId":"c","id":"s1","phase":"sideways"})).is_none());
    assert!(parse_event(&json!({"v":1,"type":"progress","conversationId":"c","phase":"start"})).is_none());
}
