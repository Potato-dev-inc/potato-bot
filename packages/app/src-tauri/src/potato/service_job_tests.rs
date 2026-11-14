//! Job turns in the engine (Professional mode, P3). A job is a saved chain of turns: when a turn ends, the app starts the
//! next one by itself, with the goal, the to-do list and the step log as context. Fakes only. What it shows: a restart
//! resumes a running job once; the limits stop it with a plain note and a reason; three turns without progress stop it; Stop
//! and Pause all end it for good; job turns do not touch the 20-a-day count; two resumes at once start one turn; a group,
//! a routine and a stranger cannot use the job tools; a person's message in the middle is delivered between turns.

use std::sync::Arc;

use serde_json::{json, Value};

use super::cards;
use super::cost;
use super::hold;
use super::jobs::{self, reason, Status};
use super::waits;
use super::rooms::store::{self as rstore, Room};
use super::service::*;
use super::service_tests::Rig;
use super::store::{self, BrainKind, FromKind, MsgKind, Settings};

const DM: &str = "app:dm:nimbus";
const KEEP_GOING: &str = "Keep working on your job.";

/// A rig with a ready brain, tool calls run at once, and the first message answered.
fn rig(name: &str) -> Rig {
    let rig = Rig::ready(name);
    rig.svc.set_spawner(Arc::new(|job| job()));
    rig.answer_all();
    rig
}

fn reply_to(id: &str) -> Value {
    json!({"v":1,"type":"reply","conversationId":DM,"inReplyTo":id,"segments":["working"]})
}

fn usage(completion: u64) -> Value {
    json!({"v":1,"type":"usage","model":"deepseek-flash","promptTokens":0,"completionTokens":completion,"cacheHitTokens":0,"cacheMissTokens":0})
}

/// The end of a turn as the brain writes it: the reply, then the usage.
fn end_turn(rig: &Rig, id: &str) {
    rig.post(vec![reply_to(id), usage(10)]);
}

fn tool(rig: &Rig, conversation: &str, call_id: &str, name: &str, args: Value) -> Value {
    rig.post(vec![json!({"v":1,"type":"sandbox_call","callId":call_id,"conversationId":conversation,"tool":name,"args":args,"image":false})]);
    rig.inbox(0).0.into_iter().find(|i| i["type"] == "sandbox_result" && i["callId"] == call_id).unwrap_or_else(|| panic!("no answer to {call_id}"))
}

fn ok(item: &Value) -> bool {
    item["ok"] == json!(true)
}

fn code(item: &Value) -> String {
    item["error"]["code"].as_str().unwrap_or("").to_string()
}

/// The job turns the brain has been given, oldest first.
fn continuations(rig: &Rig) -> Vec<Value> {
    rig.inbox(0).0.into_iter().filter(|i| i["type"] == "send" && i["text"].as_str().is_some_and(|t| t.starts_with(KEEP_GOING))).collect()
}

fn job(rig: &Rig) -> jobs::Job {
    jobs::load(&rig.root, "nimbus").expect("a job")
}

/// A person's message, a started job, and the id of the turn that started it.
fn running(rig: &Rig) -> String {
    let sent = rig.svc.send("nimbus", "do the big thing").unwrap();
    let started = tool(rig, DM, "j-start", "job_start", json!({"goal": "Tidy the project folder"}));
    assert!(ok(&started), "{started:?}");
    sent.id
}

fn log(rig: &Rig, n: u32) {
    let answer = tool(rig, DM, &format!("j-log-{n}"), "job_log", json!({"step": format!("step {n}")}));
    assert!(ok(&answer), "{answer:?}");
}

/// The id of the newest job turn in the inbox.
fn last_continuation(rig: &Rig) -> String {
    continuations(rig).last().expect("a job turn").get("id").and_then(Value::as_str).unwrap().to_string()
}

fn last_note(rig: &Rig) -> String {
    rig.notes().last().cloned().unwrap_or_default()
}

/// The one card the job waits for, which must be a Keep going card for this reason. Answering Stop ends the job.
fn keep_going_card(rig: &Rig, why: &str) -> cards::Card {
    let saved = job(rig);
    assert_eq!(saved.status, Status::Waiting, "a limit asks first");
    let open = cards::open_cards(&rig.root, "nimbus");
    assert_eq!(open.len(), 1);
    assert_eq!(saved.waiting.as_deref(), Some(open[0].id.as_str()));
    assert_eq!(open[0].keep_going.as_deref(), Some(why));
    assert_eq!(open[0].choices, vec!["Keep going", "Stop"]);
    open[0].clone()
}

// ---- the chain ----------------------------------------------------------------------------------------------------

#[test]
fn when_a_turn_ends_the_host_starts_the_next_with_the_goal_the_list_the_steps_and_the_spend() {
    let rig = rig("chain");
    let first = running(&rig);
    assert_eq!(job(&rig).status, Status::Running);
    assert!(continuations(&rig).is_empty(), "nothing starts while the turn runs");
    tool(&rig, DM, "t-add", "scratchpad_add", json!({"text": "Sort the photos"}));
    log(&rig, 1);
    end_turn(&rig, &first);
    let next = continuations(&rig);
    assert_eq!(next.len(), 1, "{next:?}");
    let item = &next[0];
    assert_eq!(item["conversationId"], DM);
    assert_eq!(item["from"]["trust"], "owner_pc", "the job works with the owner's tools");
    assert!(item["text"].as_str().unwrap().contains("Do not ask me for things you can decide yourself"));
    let context = item["context"].as_str().unwrap();
    for part in ["Goal: Tidy the project folder", "- [ ] Sort the photos", "- step 1", "Steps finished (1 in all", "of 240 min"] {
        assert!(context.contains(part), "{part} missing in {context}");
    }
    assert_eq!(item["capabilities"], json!({ "sandbox": false }));
    // the job turn is not a line from the person in the thread
    assert!(!rig.thread().iter().any(|m| m.from.kind == FromKind::You && m.text.starts_with(KEEP_GOING)));
    let saved = job(&rig);
    assert_eq!(saved.turns, 1);
    assert_eq!(saved.no_progress, 0, "a logged step is progress");
}

#[test]
fn a_job_turn_that_ends_cleanly_without_job_done_goes_on_and_job_done_ends_the_chain() {
    let rig = rig("done");
    let first = running(&rig);
    log(&rig, 1);
    end_turn(&rig, &first);
    let second = last_continuation(&rig);
    log(&rig, 2);
    end_turn(&rig, &second);
    assert_eq!(continuations(&rig).len(), 2);
    let third = last_continuation(&rig);
    let done = tool(&rig, DM, "j-done", "job_done", json!({"summary": "Everything is tidy."}));
    assert!(ok(&done), "{done:?}");
    end_turn(&rig, &third);
    assert_eq!(continuations(&rig).len(), 2, "no turn after job_done");
    let saved = job(&rig);
    assert_eq!((saved.status, saved.summary.as_deref()), (Status::Done, Some("Everything is tidy.")));
    assert_eq!(last_note(&rig), "Nimbus finished the job.");
}

#[test]
fn job_turns_do_not_use_the_twenty_a_day_count() {
    let rig = rig("count");
    let mut turn = running(&rig);
    for n in 1..=25 {
        log(&rig, n);
        end_turn(&rig, &turn);
        assert_eq!(continuations(&rig).len(), n as usize, "turn {n} was started");
        turn = last_continuation(&rig);
    }
    let day = rig.host.local_day(rig.host.now_ms());
    assert_eq!(cost::day_of(&rig.root, &day, "nimbus").unwrap().brain_turns, 0, "the brain-started count is untouched");
    // a routine still has all twenty
    for i in 0..20 {
        rig.advance(61_000);
        assert!(rig.svc.send_as("nimbus", &format!("routine {i}"), Origin::Routine { title: "nightly", context: None }).unwrap().delivered, "routine {i}");
    }
    assert_eq!(cost::day_of(&rig.root, &day, "nimbus").unwrap().brain_turns, 20);
}

#[test]
fn a_persons_message_in_the_middle_is_delivered_between_turns_and_starts_no_second_chain() {
    let rig = rig("between");
    let first = running(&rig);
    log(&rig, 1);
    end_turn(&rig, &first);
    let turn = last_continuation(&rig);
    // the person writes while the job turn runs
    let mine = rig.svc.send("nimbus", "how is it going?").unwrap();
    assert!(mine.delivered);
    let (items, _) = rig.inbox(0);
    let order: Vec<&str> = items.iter().filter(|i| i["type"] == "send").map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(&order[order.len() - 2..], [turn.as_str(), mine.id.as_str()], "queued behind the running turn, in order");
    // the job turn ends: the next one is queued after the person's message
    log(&rig, 2);
    end_turn(&rig, &turn);
    assert_eq!(continuations(&rig).len(), 2);
    // the person's turn ends: it does not start a third
    end_turn(&rig, &mine.id);
    assert_eq!(continuations(&rig).len(), 2, "one chain, one turn open at a time");
}

// ---- restart ------------------------------------------------------------------------------------------------------

#[test]
fn after_a_restart_a_running_job_starts_again_once_with_a_note() {
    let rig = rig("restart");
    let first = running(&rig);
    log(&rig, 1);
    end_turn(&rig, &first);
    assert_eq!(continuations(&rig).len(), 1);
    // the app quits mid-turn: a new service on the same folders, nothing in memory
    let svc = Service::new(rig.root.clone(), rig.host.clone(), rig.launcher.clone(), rig.brains.clone());
    assert_eq!(jobs::load(&rig.root, "nimbus").unwrap().status, Status::Running);
    let before = rig.spawned();
    svc.tick();
    assert_eq!(rig.spawned(), before + 1, "the brain starts for the job");
    let items = |svc: &Service| svc.inbox_poll("nimbus", 0, 0)["items"].as_array().cloned().unwrap_or_default();
    let sends: Vec<Value> = items(&svc).into_iter().filter(|i| i["type"] == "send").collect();
    assert_eq!(sends.len(), 1, "{sends:?}");
    assert!(sends[0]["text"].as_str().unwrap().starts_with(KEEP_GOING));
    let context = sends[0]["context"].as_str().unwrap();
    assert!(context.contains("The app was restarted") && context.contains("Goal: Tidy the project folder") && context.contains("- step 1"), "{context}");
    assert!(svc.messages("nimbus", 500).unwrap().iter().any(|m| m.kind == MsgKind::Note && m.text == "Nimbus is picking the job up again after a restart."));
    // asking again, by tick or by hand, starts nothing more
    svc.ingest("nimbus", vec![super::protocol::parse_event(&json!({"v":1,"type":"hello","protocol":1,"version":"t","provider":"deepseek","packs":["fish","potato"],"surfaces":["app"]})).unwrap()]);
    rig.advance(10_000);
    svc.tick();
    assert_eq!(svc.resume_jobs(), 0);
    let again: Vec<Value> = items(&svc).into_iter().filter(|i| i["type"] == "send").collect();
    assert_eq!(again.len(), 1, "once");
    assert_eq!(rig.spawned(), before + 1);
    // the time the app was closed is not counted against the job
    assert!(jobs::load(&rig.root, "nimbus").unwrap().turn_started_ms.is_some_and(|at| at >= rig.host.now_ms() - 20_000));
}

#[test]
fn a_stopped_a_done_and_a_waiting_job_do_not_resume() {
    for (name, status) in [("stopped", Status::Stopped), ("done", Status::Done), ("failed", Status::Failed), ("waiting", Status::Waiting)] {
        let rig = rig(&format!("noresume-{name}"));
        running(&rig);
        let mut saved = job(&rig);
        saved.status = status;
        jobs::save(&rig.root, "nimbus", &saved).unwrap();
        let svc = Service::new(rig.root.clone(), rig.host.clone(), rig.launcher.clone(), rig.brains.clone());
        assert_eq!(svc.resume_jobs(), 0, "{name}");
        rig.advance(10_000);
        svc.tick();
        assert!(svc.inbox_poll("nimbus", 0, 0)["items"].as_array().unwrap().is_empty(), "{name}");
    }
}

#[test]
fn a_damaged_job_file_resumes_nothing_and_does_not_crash() {
    let rig = rig("damaged");
    running(&rig);
    std::fs::write(jobs::path(&rig.root, "nimbus").unwrap(), "{ not json").unwrap();
    let svc = Service::new(rig.root.clone(), rig.host.clone(), rig.launcher.clone(), rig.brains.clone());
    assert_eq!(svc.resume_jobs(), 0);
    svc.tick();
    assert!(jobs::load(&rig.root, "nimbus").is_none());
}

#[test]
fn two_resumes_at_once_start_one_turn() {
    let rig = rig("concurrent");
    running(&rig);
    let svc = Service::new(rig.root.clone(), rig.host.clone(), rig.launcher.clone(), rig.brains.clone());
    let started: usize = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8).map(|_| s.spawn(|| svc.resume_jobs())).collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(started, 1, "seven of the eight were refused");
    let sends = svc.inbox_poll("nimbus", 0, 0)["items"].as_array().unwrap().iter().filter(|i| i["type"] == "send").count();
    assert_eq!(sends, 1);
}

// ---- limits -------------------------------------------------------------------------------------------------------

#[test]
fn the_time_limit_stops_the_job_with_a_note_and_a_reason() {
    let rig = rig("time");
    let first = running(&rig);
    log(&rig, 1);
    rig.advance(jobs::MAX_JOB_MS + 1_000);
    end_turn(&rig, &first);
    assert!(continuations(&rig).is_empty(), "no turn past the limit");
    let card = keep_going_card(&rig, reason::TIME_LIMIT);
    assert_eq!(card.action, "Nimbus's job reached its time limit of 4 hours.");
    // Stop on the card ends it, with the same plain note as before
    rig.svc.answer_card("nimbus", &card.id, "1").unwrap();
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::TIME_LIMIT)));
    assert_eq!(last_note(&rig), "Nimbus's job stopped: it reached the time limit of 4 hours. 1 step was finished.");
}

#[test]
fn the_money_limit_stops_the_job_with_a_note_and_a_reason() {
    let rig = rig("money");
    rig.svc.set_settings(Settings { brain: BrainKind::Deepseek, daily_limit_cents: 2_000, ..Settings::default() }).unwrap();
    let first = running(&rig);
    log(&rig, 1);
    // about 5.17 dollars of output in one turn
    rig.post(vec![reply_to(&first), usage(4_700_000)]);
    assert!(continuations(&rig).is_empty());
    let card = keep_going_card(&rig, reason::MONEY_LIMIT);
    assert_eq!(card.action, "Nimbus's job reached its spending limit of $5.00.");
    assert!(job(&rig).spent_nano >= jobs::MAX_JOB_NANO);
    rig.svc.answer_card("nimbus", &card.id, "1").unwrap();
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::MONEY_LIMIT)));
    assert_eq!(last_note(&rig), "Nimbus's job stopped: it reached the spending limit of $5.00. 1 step was finished.");
}

#[test]
fn the_daily_money_pause_still_applies_and_stops_the_job() {
    let rig = rig("daily");
    rig.svc.set_settings(Settings { brain: BrainKind::Deepseek, daily_limit_cents: 50, ..Settings::default() }).unwrap();
    let first = running(&rig);
    log(&rig, 1);
    rig.post(vec![reply_to(&first), usage(500_000)]); // about 55 cents: past the day's 50
    assert!(continuations(&rig).is_empty(), "no job turn is started past the day's limit");
    // it asks like the other stops (a Keep going / Stop card); Stop ends it
    let saved = job(&rig);
    assert_eq!(saved.status, Status::Waiting);
    let card = crate::potato::cards::open_cards(&rig.root, "nimbus").remove(0);
    assert_eq!(card.keep_going.as_deref(), Some(reason::DAILY_LIMIT));
    rig.svc.answer_card("nimbus", &card.id, "1").unwrap();
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::DAILY_LIMIT)));
    assert_eq!(last_note(&rig), "Nimbus's job stopped: today's limit is used up. Raise it in Settings or wait until tomorrow. 1 step was finished.");
}

#[test]
fn the_cost_of_the_bot_while_the_job_runs_is_added_to_the_job() {
    let rig = rig("spend");
    let first = running(&rig);
    log(&rig, 1);
    end_turn(&rig, &first);
    assert!(job(&rig).spent_nano > 0, "ten tokens cost something small");
    let turn = last_continuation(&rig);
    log(&rig, 2);
    rig.post(vec![reply_to(&turn), usage(1_000_000)]);
    assert!(job(&rig).spent_nano >= 1_100_000_000, "a million output tokens are $1.10: {}", job(&rig).spent_nano);
}

// ---- no progress --------------------------------------------------------------------------------------------------

#[test]
fn three_turns_in_a_row_with_nothing_new_stop_the_job() {
    let rig = rig("noprogress");
    let mut turn = running(&rig);
    // turn 1 and 2 finish nothing: the job goes on
    for n in 1..=2 {
        end_turn(&rig, &turn);
        assert_eq!(continuations(&rig).len(), n, "turn {n} was followed by another");
        assert_eq!(job(&rig).no_progress, n as u32);
        turn = last_continuation(&rig);
    }
    // the third turn finishes nothing either
    end_turn(&rig, &turn);
    assert_eq!(continuations(&rig).len(), 2, "no fourth turn");
    let card = keep_going_card(&rig, reason::NO_PROGRESS);
    assert_eq!(card.action, "Nimbus's job looks stuck: 3 turns in a row finished nothing new.");
    assert!(card.why.contains("It has not finished any step yet."), "{}", card.why);
    rig.svc.answer_card("nimbus", &card.id, "1").unwrap();
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::NO_PROGRESS)));
    assert_eq!(last_note(&rig), "Nimbus's job stopped: 3 turns in a row finished nothing new.");
}

#[test]
fn a_new_step_or_a_change_to_the_to_do_list_is_progress_and_resets_the_count() {
    let rig = rig("progress");
    let mut turn = running(&rig);
    end_turn(&rig, &turn);
    turn = last_continuation(&rig);
    assert_eq!(job(&rig).no_progress, 1);
    // a changed to-do list counts
    tool(&rig, DM, "t-add", "scratchpad_add", json!({"text": "Sort the photos"}));
    end_turn(&rig, &turn);
    turn = last_continuation(&rig);
    assert_eq!(job(&rig).no_progress, 0, "the list changed");
    end_turn(&rig, &turn);
    turn = last_continuation(&rig);
    assert_eq!(job(&rig).no_progress, 1);
    log(&rig, 1);
    end_turn(&rig, &turn);
    assert_eq!(job(&rig).no_progress, 0, "a step was logged");
    assert_eq!(job(&rig).status, Status::Running);
}

#[test]
fn a_model_failure_counts_as_no_progress_and_the_job_goes_on_until_the_third() {
    let rig = rig("failures");
    let mut turn = running(&rig);
    for n in 1..=2 {
        log(&rig, n); // even with a step, a failed turn is no progress
        rig.post(vec![usage(10), json!({"v":1,"type":"error","code":"PROVIDER_DOWN","conversationId":DM,"inReplyTo":turn})]);
        assert_eq!(job(&rig).status, Status::Running, "failure {n} does not end the job");
        assert_eq!(job(&rig).no_progress, n as u32);
        assert_eq!(continuations(&rig).len(), n as usize);
        turn = last_continuation(&rig);
    }
    rig.post(vec![usage(10), json!({"v":1,"type":"error","code":"PROVIDER_DOWN","conversationId":DM,"inReplyTo":turn})]);
    let card = keep_going_card(&rig, reason::NO_PROGRESS);
    assert!(card.why.contains("What it tried: step 1; step 2."), "the card says what it tried: {}", card.why);
    assert_eq!(continuations(&rig).len(), 2);
    // the failed turns did not tell the person to say continue: the job went on by itself
    assert!(rig.notes().iter().all(|n| !n.contains("Say continue")), "{:?}", rig.notes());
}

// ---- waits --------------------------------------------------------------------------------------------------------

/// An open wait of the bot, as `wait_until` makes it.
fn open_wait(rig: &Rig, note: &str) -> String {
    let wait = waits::make("nimbus", &json!({ "check": "test -f done.txt", "note": note }), false, rig.host.now_ms()).unwrap();
    waits::create(&rig.root, wait).unwrap().id
}

#[test]
fn a_turn_that_ends_with_a_wait_open_parks_the_job_and_does_not_count_as_no_progress() {
    let rig = rig("parked");
    let first = running(&rig);
    open_wait(&rig, "the render");
    end_turn(&rig, &first);
    assert!(continuations(&rig).is_empty(), "no next turn while the wait is open");
    let saved = job(&rig);
    assert_eq!((saved.status, saved.wait_parked, saved.no_progress, saved.turns), (Status::Running, true, 0, 1));
    assert_eq!(last_note(&rig), "Nimbus is waiting for the render and will go on when it is done.");
    // the 5-second tick does not start a turn either, and says nothing more
    let notes = rig.notes().len();
    for _ in 0..3 {
        rig.advance(10_000);
        rig.svc.tick();
    }
    assert!(continuations(&rig).is_empty());
    assert_eq!(rig.notes().len(), notes);
    assert!(cards::open_cards(&rig.root, "nimbus").is_empty());
}

#[test]
fn when_the_wait_is_over_the_parked_job_goes_on_and_many_waits_never_stop_it() {
    let rig = rig("resumes");
    let mut turn = running(&rig);
    // five waits in a row, each ending a turn with nothing logged: the three-turn stop does not apply
    for n in 1..=5 {
        let id = open_wait(&rig, "the render");
        end_turn(&rig, &turn);
        assert_eq!(continuations(&rig).len(), n - 1, "parked at wait {n}");
        assert_eq!(job(&rig).no_progress, 0);
        waits::cancel(&rig.root, Some("nimbus"), &id).unwrap();
        rig.advance(10_000);
        rig.svc.tick();
        assert_eq!(continuations(&rig).len(), n, "the end of wait {n} resumes the job");
        assert!(!job(&rig).wait_parked);
        turn = last_continuation(&rig);
    }
    assert_eq!(job(&rig).status, Status::Running);
    assert!(cards::open_cards(&rig.root, "nimbus").is_empty());
}

#[test]
fn a_failed_turn_is_not_parked_even_with_a_wait_open_and_the_wait_time_is_not_the_jobs() {
    let rig = rig("failwait");
    let turn = running(&rig);
    open_wait(&rig, "the render");
    rig.post(vec![usage(10), json!({"v":1,"type":"error","code":"PROVIDER_DOWN","conversationId":DM,"inReplyTo":turn})]);
    assert_eq!(job(&rig).no_progress, 1, "a failure is still no progress");
    assert!(!job(&rig).wait_parked);
    assert_eq!(continuations(&rig).len(), 1);
    // a long wait does not spend the job's time
    let turn = last_continuation(&rig);
    end_turn(&rig, &turn);
    assert!(job(&rig).wait_parked);
    let spent = job(&rig).spent_ms;
    rig.advance(3 * 60 * 60 * 1000);
    rig.svc.tick();
    assert_eq!(job(&rig).spent_ms, spent);
    assert_eq!(job(&rig).status, Status::Running);
}

// ---- the day's computer actions ---------------------------------------------------------------------------------------

fn today(rig: &Rig) -> String {
    rig.host.local_day(rig.host.now_ms())
}

fn set_cap(rig: &Rig, cap: u32) {
    store::save_settings(&rig.root, &Settings { sandbox_actions_per_day: cap, ..Settings::default() }).unwrap();
}

/// What a refused computer call does to the bot's running job (`Calls::spend`).
fn hit_the_cap(rig: &Rig, cap: u32) {
    jobs::mark_actions_capped(&rig.root, "nimbus", &today(rig), cap, rig.host.now_ms());
}

#[test]
fn a_job_that_hits_the_action_cap_is_paused_with_a_card_and_not_counted_as_stuck() {
    let rig = rig("cap-pause");
    let first = running(&rig);
    set_cap(&rig, 50);
    hit_the_cap(&rig, 50);
    end_turn(&rig, &first);
    assert!(continuations(&rig).is_empty(), "no turn that cannot use the computer");
    let card = keep_going_card(&rig, reason::ACTION_LIMIT);
    assert_eq!(card.action, "Nimbus's job is paused: it used all 50 of today's computer actions.");
    assert!(card.why.contains("goes on by itself tomorrow") && card.why.contains("Computer actions per day"), "{}", card.why);
    assert_eq!(job(&rig).no_progress, 0);
    // still the same day and the same cap: it stays paused, with one card
    for _ in 0..3 {
        rig.advance(10_000);
        rig.svc.tick();
    }
    assert!(continuations(&rig).is_empty());
    assert_eq!(cards::open_cards(&rig.root, "nimbus").len(), 1);
    assert_eq!(job(&rig).status, Status::Waiting);
}

#[test]
fn a_paused_job_goes_on_by_itself_when_the_next_day_starts() {
    let rig = rig("cap-tomorrow");
    let first = running(&rig);
    set_cap(&rig, 50);
    hit_the_cap(&rig, 50);
    end_turn(&rig, &first);
    keep_going_card(&rig, reason::ACTION_LIMIT);
    rig.advance(24 * 60 * 60 * 1000);
    rig.svc.tick();
    let saved = job(&rig);
    assert_eq!((saved.status, saved.actions_cap.clone()), (Status::Running, None));
    // the brain was idle for a day and is a new one; the turn was handed over when the decision is gone from the record
    assert!(saved.decision.is_none(), "the job's next turn was sent with the decision");
    assert!(cards::open_cards(&rig.root, "nimbus").is_empty());
}

#[test]
fn a_paused_job_goes_on_by_itself_when_the_cap_is_raised() {
    let rig = rig("cap-raised");
    let first = running(&rig);
    set_cap(&rig, 50);
    hit_the_cap(&rig, 50);
    end_turn(&rig, &first);
    keep_going_card(&rig, reason::ACTION_LIMIT);
    set_cap(&rig, 200);
    rig.advance(10_000);
    rig.svc.tick();
    assert_eq!(job(&rig).status, Status::Running);
    assert!(job(&rig).decision.is_none(), "the job's next turn was sent");
    assert!(cards::open_cards(&rig.root, "nimbus").is_empty());
}

#[test]
fn keep_going_while_the_cap_is_still_hit_asks_again_without_spending_a_turn_and_stop_ends_the_job() {
    let rig = rig("cap-again");
    let first = running(&rig);
    set_cap(&rig, 50);
    hit_the_cap(&rig, 50);
    end_turn(&rig, &first);
    let card = keep_going_card(&rig, reason::ACTION_LIMIT);
    rig.svc.answer_card("nimbus", &card.id, "0").unwrap();
    assert!(continuations(&rig).is_empty(), "no turn was started");
    let again = keep_going_card(&rig, reason::ACTION_LIMIT);
    assert_ne!(again.id, card.id);
    rig.svc.answer_card("nimbus", &again.id, "1").unwrap();
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::ACTION_LIMIT)));
    assert_eq!(last_note(&rig), "Nimbus's job stopped: today's computer actions are used up. Raise the limit in Settings or wait until tomorrow.");
}

#[test]
fn a_job_without_a_cap_hit_is_not_asked_and_a_cap_hit_without_a_job_marks_nothing() {
    let rig = rig("cap-none");
    let first = running(&rig);
    end_turn(&rig, &first);
    assert_eq!(continuations(&rig).len(), 1);
    assert!(cards::open_cards(&rig.root, "nimbus").is_empty());
    // no running job: nothing is written
    let other = Rig::ready("cap-nojob");
    hit_the_cap(&other, 50);
    assert!(jobs::load(&other.root, "nimbus").is_none());
}

#[test]
fn the_eighty_percent_note_comes_once_a_day() {
    let rig = rig("cap-warn");
    set_cap(&rig, 100);
    let day = today(&rig);
    cost::spend_sandbox_actions(&rig.root, &day, "nimbus", 79, 100).unwrap();
    rig.svc.warn_actions_nearly_used("nimbus");
    assert!(rig.notes().iter().all(|n| !n.contains("computer actions")), "79 of 100 is not yet 80%");
    cost::spend_sandbox_actions(&rig.root, &day, "nimbus", 1, 100).unwrap();
    rig.svc.warn_actions_nearly_used("nimbus");
    rig.svc.warn_actions_nearly_used("nimbus");
    let warned: Vec<String> = rig.notes().into_iter().filter(|n| n.contains("computer actions")).collect();
    assert_eq!(warned, vec!["Nimbus has used most of today's computer actions (80% of 100). At 100 it stops using the computer until tomorrow, or until you raise the limit in Settings.".to_string()]);
    // tomorrow it can say it again
    rig.advance(24 * 60 * 60 * 1000);
    let day = today(&rig);
    cost::spend_sandbox_actions(&rig.root, &day, "nimbus", 85, 100).unwrap();
    rig.svc.warn_actions_nearly_used("nimbus");
    assert_eq!(rig.notes().iter().filter(|n| n.contains("computer actions")).count(), 2);
}

// ---- the computer stays on for a job ------------------------------------------------------------------------------

#[test]
fn a_running_or_parked_job_holds_the_computer_from_the_idle_stop_and_a_job_that_waits_for_an_answer_or_no_job_holds_nothing() {
    use super::sandbox::lease::ScreenLease;
    use super::sandbox::FakeSandbox;
    use super::sandbox_calls::Port;
    let rig = rig("hold");
    let fake = Arc::new(FakeSandbox::new());
    rig.svc.set_sandbox_port(Some(Port { sandbox: fake.clone(), lease: Arc::new(parking_lot::Mutex::new(ScreenLease::new())) }));
    // no job: nothing is held
    rig.advance(10_000);
    rig.svc.tick();
    assert!(fake.job_hold_log().is_empty());
    // a running job holds for a minute from now, renewed at every look
    let first = running(&rig);
    rig.advance(10_000);
    rig.svc.tick();
    let first_hold = fake.job_hold_log();
    assert_eq!(first_hold.len(), 1);
    assert_eq!(first_hold[0], rig.host.now_ms() + 60_000);
    rig.advance(10_000);
    rig.svc.tick();
    assert_eq!(fake.job_hold_log().len(), 2);
    // parked for a wait: still held
    open_wait(&rig, "the render");
    end_turn(&rig, &first);
    assert!(job(&rig).wait_parked);
    rig.advance(10_000);
    rig.svc.tick();
    assert_eq!(fake.job_hold_log().len(), 3);
    // waiting for the owner's answer (a card), or stopped: nothing is held
    let mut saved = job(&rig);
    saved.status = Status::Waiting;
    jobs::save(&rig.root, "nimbus", &saved).unwrap();
    rig.advance(10_000);
    rig.svc.tick();
    assert_eq!(fake.job_hold_log().len(), 3);
    saved.status = Status::Stopped;
    jobs::save(&rig.root, "nimbus", &saved).unwrap();
    rig.advance(10_000);
    rig.svc.tick();
    assert_eq!(fake.job_hold_log().len(), 3);
}

// ---- Stop and Pause all -------------------------------------------------------------------------------------------

#[test]
fn stop_in_the_middle_of_a_job_ends_it_and_nothing_continues() {
    let rig = rig("stop");
    let first = running(&rig);
    log(&rig, 1);
    end_turn(&rig, &first);
    let turn = last_continuation(&rig);
    rig.svc.stop_bot_by_person("nimbus");
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::STOPPED_BY_YOU)));
    assert_eq!(last_note(&rig), "Nimbus's job was stopped. 1 step was finished.");
    // the dying turn still reports: nothing follows it
    let sends = continuations(&rig).len();
    end_turn(&rig, &turn);
    rig.post(vec![json!({"v":1,"type":"error","code":"INTERNAL","conversationId":DM,"inReplyTo":turn})]);
    assert_eq!(continuations(&rig).len(), sends, "no turn after Stop");
    let brains = rig.spawned();
    for _ in 0..3 {
        rig.advance(10_000);
        rig.svc.tick();
    }
    assert_eq!(rig.svc.resume_jobs(), 0);
    assert_eq!(rig.spawned(), brains, "no brain was started for it again");
    assert_eq!(job(&rig).status, Status::Stopped);
}

#[test]
fn pause_all_ends_a_job_but_quitting_the_app_does_not() {
    let rig = rig("pause");
    let first = running(&rig);
    log(&rig, 1);
    end_turn(&rig, &first);
    // quit: every brain is stopped, the job stays running for the next start
    rig.svc.stop_all(std::time::Duration::ZERO);
    assert_eq!(job(&rig).status, Status::Running);
    // Pause all: the job is stopped for good
    let held = hold::pause(&rig.root, Some(&*rig.svc), false, std::time::Duration::ZERO, rig.host.now_ms());
    assert!(held.held);
    let saved = job(&rig);
    assert_eq!((saved.status, saved.stop_reason.as_deref()), (Status::Stopped, Some(reason::PAUSED)));
    assert_eq!(last_note(&rig), "Nimbus's job was stopped because bots were paused. 1 step was finished.");
    rig.advance(10_000);
    rig.svc.tick();
    assert_eq!(rig.svc.resume_jobs(), 0);
}

// ---- who may use the tools ----------------------------------------------------------------------------------------

#[test]
fn a_group_cannot_start_or_log_a_job() {
    let rig = rig("room");
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    rstore::save_rooms(&rig.root, &[Room { id: "r0123456789ab".into(), name: "Weekend".into(), bot_ids: vec!["nimbus".into(), "mochi".into()], created_ms: 1, last_ms: 1, unread: 0, ..Default::default() }]).unwrap();
    let room = store::app_room_conversation("r0123456789ab");
    for (n, (name, args)) in [("job_start", json!({"goal": "take over"})), ("job_log", json!({"step": "x"})), ("job_done", json!({"summary": "x"}))].into_iter().enumerate() {
        let answer = tool(&rig, &room, &format!("r-{n}"), name, args);
        assert_eq!((answer["ok"].clone(), code(&answer)), (json!(false), "TOOL_NOT_ALLOWED".to_string()), "{name}: {answer:?}");
    }
    assert!(jobs::load(&rig.root, "nimbus").is_none(), "no job was made from a group");
}

#[test]
fn a_routine_run_cannot_start_a_job() {
    let rig = rig("routine");
    rig.advance(61_000);
    let sent = rig.svc.send_as("nimbus", "do the nightly thing", Origin::Routine { title: "nightly", context: None }).unwrap();
    assert!(sent.delivered);
    let answer = tool(&rig, DM, "r-1", "job_start", json!({"goal": "run forever"}));
    assert_eq!(code(&answer), "TOOL_NOT_ALLOWED", "{answer:?}");
    assert!(jobs::load(&rig.root, "nimbus").is_none());
}

#[test]
fn a_job_call_from_a_chat_that_is_not_the_bots_own_private_chat_is_refused() {
    let rig = rig("elsewhere");
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    // the brain of nimbus names the private chat of mochi: it is dropped before any job is made
    rig.post(vec![json!({"v":1,"type":"sandbox_call","callId":"x-1","conversationId":"app:dm:mochi","tool":"job_start","args":{"goal":"x"},"image":false})]);
    assert!(jobs::load(&rig.root, "nimbus").is_none() && jobs::load(&rig.root, "mochi").is_none());
}

#[test]
fn only_one_job_at_a_time_and_a_new_one_after_it_ends() {
    let rig = rig("one");
    running(&rig);
    let again = tool(&rig, DM, "j-2", "job_start", json!({"goal": "another"}));
    assert_eq!(code(&again), "POTATO_JOB_EXISTS");
    tool(&rig, DM, "j-3", "job_done", json!({"summary": "ok"}));
    assert!(ok(&tool(&rig, DM, "j-4", "job_start", json!({"goal": "another"}))));
    assert_eq!(job(&rig).goal, "another");
}

// ---- a chat turn that ran out of steps becomes a job ----------------------------------------------------------------

/// The goal and the line the brain gives when a chat turn ran out of steps (packages/potato-brain promoteToJob).
const PROMOTED_GOAL: &str = "Finish the work in progress in this chat. The person last said: continue";
const PROMOTED_TEXT: &str = "Still working on this as a job \u{2014} I'll keep going. Press Stop to end it.";

fn promoted_reply(id: &str) -> Value {
    json!({"v":1,"type":"reply","conversationId":DM,"inReplyTo":id,"segments":[PROMOTED_TEXT]})
}

#[test]
fn a_chat_turn_that_ran_out_of_steps_starts_a_job_and_the_host_continues_it_by_itself() {
    let rig = rig("autojob");
    let sent = rig.svc.send("nimbus", "do the big thing").unwrap();
    // The brain ran out of steps in this normal turn: it calls job_start, then ends the turn with its short note.
    let started = tool(&rig, DM, "auto-start", "job_start", json!({"goal": PROMOTED_GOAL}));
    assert!(ok(&started), "{started:?}");
    assert_eq!(job(&rig).goal, PROMOTED_GOAL);
    assert_eq!(job(&rig).status, Status::Running);
    assert!(continuations(&rig).is_empty(), "nothing starts while the turn runs");
    rig.post(vec![promoted_reply(&sent.id), usage(10)]);
    let next = continuations(&rig);
    assert_eq!(next.len(), 1, "the host starts the next turn by itself: {next:?}");
    assert!(next[0]["context"].as_str().unwrap().contains(&format!("Goal: {PROMOTED_GOAL}")));
    assert!(rig.thread().iter().any(|m| m.text == PROMOTED_TEXT), "the person sees the one short note");
    assert!(!rig.thread().iter().any(|m| m.text.contains("Say continue")));
    // The job runs under the usual limits and ends the usual way.
    let second = last_continuation(&rig);
    let done = tool(&rig, DM, "auto-done", "job_done", json!({"summary": "Done."}));
    assert!(ok(&done), "{done:?}");
    end_turn(&rig, &second);
    assert_eq!(continuations(&rig).len(), 1, "no turn after job_done");
    assert_eq!(job(&rig).status, Status::Done);
}

#[test]
fn a_chat_turn_that_ran_out_of_steps_while_a_job_runs_is_refused_and_starts_no_second_chain() {
    let rig = rig("autojob-busy");
    let first = running(&rig);
    // A person's message in the middle; its turn runs out of steps and the brain tries to promote it.
    let second = rig.svc.send("nimbus", "do the big thing").unwrap();
    let refused = tool(&rig, DM, "auto-again", "job_start", json!({"goal": PROMOTED_GOAL}));
    assert!(!ok(&refused), "{refused:?}");
    assert!(!code(&refused).is_empty());
    assert_eq!(job(&rig).goal, "Tidy the project folder", "the running job is untouched");
    // The brain keeps today's ending for that turn; no second chain starts.
    end_turn(&rig, &first);
    let before = continuations(&rig).len();
    rig.post(vec![json!({"v":1,"type":"reply","conversationId":DM,"inReplyTo":second.id,"segments":["I stopped after 60 steps. Say continue and I will pick it up from where the files are."]}), usage(10)]);
    assert!(continuations(&rig).len() <= before + 1, "at most the one chain of the running job");
    assert_eq!(job(&rig).goal, "Tidy the project folder");
}
