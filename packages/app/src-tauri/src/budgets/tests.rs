//! Budget behaviour: accounting from operation records, the pause at the turn boundary, the pause
//! card and "Add turns" / "Stop here", the default-off path, helpers drawing from their parent and
//! the depth limit.

use super::accounting::{standing, turns_used};
use super::adjust::{add_minutes, add_turns, stop};
use super::view::view;
use super::gate::check_next_turn_at;
use super::model::{BudgetRecord, Source, MAX_DEPTH};
use super::*;
use crate::operations::{self, DelegateWorkRequest, OperationStatus, ResultCapture, WorkerPersist};
use serde_json::json;
use uuid::Uuid;

const T0: u64 = 1_000_000;

fn project() -> String {
    std::env::temp_dir().join(format!("pm-budgets-{}", Uuid::new_v4())).to_string_lossy().into_owned()
}

fn handle() -> String {
    format!("h-{}", &Uuid::new_v4().to_string()[..8])
}

fn start_turn(project: &str, handle: &str, index: u32) -> operations::OperationSnapshot {
    operations::create_operation(DelegateWorkRequest {
        project_path: project.to_string(),
        task: format!("turn {index}"),
        agent_type: "codex".into(),
        pane_id: None,
        idempotency_key: Uuid::new_v4().to_string(),
        acceptance_criteria: None,
        task_id: None,
        exclusive: false,
        locks: vec![],
        timeout_ms: 900_000,
        read_only: false,
        keep_pane: false,
        placement: None,
        owner_session_id: Some("budget-test".into()),
        parent_pane_id: None,
        worker_has_mcp_tools: false,
        agent_run_id: Some(handle.into()),
        turn_index: index,
        worker: WorkerPersist::default(),
        context_policy: None,
        checks: Vec::new(),
    })
    .unwrap()
    .0
}

fn finish(project: &str, turn: &operations::OperationSnapshot) -> operations::OperationSnapshot {
    operations::run_operation_dispatch(project, &turn.operation_id, |_| Ok(())).unwrap();
    operations::mark_operation_finished(project, &turn.operation_id, Some("done".into()), ResultCapture::Authoritative, true).unwrap()
}

fn asked(turns: Option<u32>, minutes: Option<u32>) -> Asked {
    Asked { turns, minutes, parent: None }
}

fn helper_of(parent: &str, turns: Option<u32>) -> Asked {
    Asked { turns, minutes: None, parent: Some(parent.into()) }
}

fn root(project: &str, handle: &str, turns: u32) -> BudgetRecord {
    admit(project, handle, &asked(Some(turns), None), Defaults::default(), T0).unwrap().unwrap()
}

fn code(result: Result<(), crate::operations::OperationError>) -> String {
    result.expect_err("expected BUDGET_EXHAUSTED").code
}

// ---- the default is OFF: today's behaviour -------------------------------------------------

#[test]
fn a_default_budget_applies_to_a_new_run_and_arguments_win_per_limit() {
    let project = project();
    let (a, b) = (handle(), handle());
    let defaults = Defaults { turns: Some(20), minutes: Some(60) };
    let rec = admit(&project, &a, &Asked::default(), defaults, T0).unwrap().unwrap();
    assert_eq!((rec.turns, rec.minutes, rec.source), (Some(20), Some(60), Source::Default));
    let rec = admit(&project, &b, &asked(Some(3), None), defaults, T0).unwrap().unwrap();
    assert_eq!((rec.turns, rec.minutes, rec.source), (Some(3), Some(60), Source::Argument));
}

#[test]
fn bad_arguments_are_refused_before_anything_is_made() {
    let project = project();
    let h = handle();
    assert_eq!(admit(&project, &h, &asked(Some(0), None), Defaults::default(), T0).unwrap_err().code, "INVALID_ARGUMENT");
    assert_eq!(admit(&project, &h, &asked(None, Some(0)), Defaults::default(), T0).unwrap_err().code, "INVALID_ARGUMENT");
    assert!(record_of(&project, &h).is_none());
}

// ---- accounting from the operation records -------------------------------------------------

#[test]
fn turns_are_counted_from_the_operation_records() {
    let project = project();
    let h = handle();
    root(&project, &h, 5);
    assert_eq!(turns_used(&project, &h), 0);
    let first = start_turn(&project, &h, 0);
    finish(&project, &first);
    start_turn(&project, &h, 1);
    assert_eq!(turns_used(&project, &h), 2, "a finished turn and a running one both count");
    let shown = view(&project, &h).unwrap();
    assert_eq!(shown["turns"]["used"], 2);
    assert_eq!(shown["turns"]["limit"], 5);
    assert_eq!(shown["state"], "ok");
    start_turn(&project, &h, 2);
    start_turn(&project, &h, 3);
    assert_eq!(view(&project, &h).unwrap()["state"], "nearly", "4 of 5 is past 80%");
}

#[test]
fn a_turn_cancelled_before_it_started_used_nothing() {
    let project = project();
    let h = handle();
    root(&project, &h, 2);
    let turn = start_turn(&project, &h, 0);
    operations::cancel_operation(&project, &turn.operation_id).unwrap();
    assert_eq!(turns_used(&project, &h), 0);
}

// ---- pause at the turn boundary -------------------------------------------------------------

#[test]
fn the_run_pauses_before_the_next_turn_and_a_running_turn_is_never_touched() {
    let project = project();
    let h = handle();
    root(&project, &h, 1);
    check_next_turn_at(&project, &h, T0).expect("the first turn may start");
    let running = start_turn(&project, &h, 0);
    // Budget is now used, while the turn is still running: nothing is cancelled or interrupted.
    let before = operations::get_operation(&project, &running.operation_id).unwrap();
    let error = check_next_turn_at(&project, &h, T0).unwrap_err();
    let after = operations::get_operation(&project, &running.operation_id).unwrap();
    assert_eq!(error.code, "BUDGET_EXHAUSTED");
    assert_eq!(before.status, after.status);
    assert_ne!(after.status, OperationStatus::Cancelled);
    assert_eq!(after.revision, before.revision, "the running turn was not written to");
    // The caller is told what to do, in words.
    assert!(error.message.contains("paused, not killed"), "{}", error.message);
    assert!(error.message.contains("Do not retry in a loop"), "{}", error.message);
    assert!(error.recoverable);
    assert_eq!(error.context["reason"], "turns");
    assert_eq!(error.context["turns_used"], 1);
    assert_eq!(error.context["turns_limit"], 1);
}

// ---- Add turns resumes; Stop here ends it ---------------------------------------------------

#[test]
fn a_time_budget_pauses_after_its_minutes_and_adding_minutes_resumes() {
    let project = project();
    let h = handle();
    admit(&project, &h, &asked(None, Some(60)), Defaults::default(), T0).unwrap();
    check_next_turn_at(&project, &h, T0 + 59 * 60_000).expect("59 of 60 minutes");
    let error = check_next_turn_at(&project, &h, T0 + 60 * 60_000).unwrap_err();
    assert_eq!(error.code, "BUDGET_EXHAUSTED");
    assert_eq!(error.context["reason"], "time");
    add_minutes(&project, &h, 30).unwrap();
    assert_eq!(record_of(&project, &h).unwrap().minutes, Some(90));
    check_next_turn_at(&project, &h, T0 + 61 * 60_000).expect("30 more minutes");
}

#[test]
fn records_survive_a_restart() {
    let project = project();
    let h = handle();
    root(&project, &h, 5);
    add_turns(&project, &h, 2).unwrap();
    store::forget_cache(&project);
    assert_eq!(record_of(&project, &h).unwrap().turns, Some(7), "read back from budgets.json");
}

// ---- helpers draw from the parent, at most two levels deep ----------------------------------

#[test]
fn a_helper_takes_half_of_what_is_left_and_the_parent_accounts_for_it() {
    let project = project();
    let (parent, child) = (handle(), handle());
    root(&project, &parent, 10);
    finish(&project, &start_turn(&project, &parent, 0));
    finish(&project, &start_turn(&project, &parent, 1));
    let share = admit(&project, &child, &helper_of(&parent, None), Defaults::default(), T0).unwrap().unwrap();
    assert_eq!(share.turns, Some(4), "half of the 8 left");
    assert_eq!((share.depth, share.source, share.parent.as_deref()), (1, Source::Share, Some(parent.as_str())));
    let parent_view = view(&project, &parent).unwrap();
    assert_eq!(parent_view["turns"]["own"], 2);
    assert_eq!(parent_view["turns"]["held_for_helpers"], 4);
    assert_eq!(parent_view["turns"]["used"], 6);
    assert_eq!(parent_view["helpers"][0]["limit"], 4);
    // The helper's own turns are counted against its own share.
    finish(&project, &start_turn(&project, &child, 0));
    assert_eq!(view(&project, &child).unwrap()["turns"]["used"], 1);
    assert_eq!(view(&project, &parent).unwrap()["helpers"][0]["used"], 1);
    assert_eq!(view(&project, &parent).unwrap()["turns"]["used"], 6, "its share is held, not the turns it used");
}

#[test]
fn a_helper_can_never_get_more_than_its_parent_has_left() {
    let project = project();
    let (parent, child) = (handle(), handle());
    root(&project, &parent, 5);
    let error = admit(&project, &child, &helper_of(&parent, Some(6)), Defaults::default(), T0).unwrap_err();
    assert_eq!(error.code, "BUDGET_EXHAUSTED");
    assert_eq!(error.context["reason"], "more_than_left");
    assert_eq!(error.context["left"], 5);
    assert!(error.message.contains("Couldn't start a helper"));
    assert!(record_of(&project, &child).is_none(), "nothing was made");
    let ok = admit(&project, &child, &helper_of(&parent, Some(5)), Defaults::default(), T0).unwrap().unwrap();
    assert_eq!(ok.turns, Some(5));
    // The parent has handed everything out: it is paused for its own next turn, and a second helper gets nothing.
    assert_eq!(code(check_next_turn_at(&project, &parent, T0)), "BUDGET_EXHAUSTED");
    let second = admit(&project, &handle(), &helper_of(&parent, None), Defaults::default(), T0).unwrap_err();
    assert_eq!(second.code, "BUDGET_EXHAUSTED");
    assert!(second.message.starts_with("Couldn't start a helper: budget used up"), "{}", second.message);
}

#[test]
fn a_helper_shares_the_parents_clock_and_cannot_outlive_it() {
    let project = project();
    let (parent, child) = (handle(), handle());
    admit(&project, &parent, &asked(Some(10), Some(60)), Defaults::default(), T0).unwrap();
    let later = T0 + 20 * 60_000;
    let share = admit(&project, &child, &helper_of(&parent, None), Defaults::default(), later).unwrap().unwrap();
    assert_eq!(share.minutes, Some(40), "the 40 minutes the parent has left");
    let too_long = admit(&project, &handle(), &Asked { turns: None, minutes: Some(50), parent: Some(parent.clone()) }, Defaults::default(), later);
    assert_eq!(too_long.unwrap_err().context["reason"], "more_than_left");
}

#[test]
fn helpers_may_go_two_levels_deep_and_no_further() {
    let project = project();
    let (a, b, c, d) = (handle(), handle(), handle(), handle());
    root(&project, &a, 40);
    let level1 = admit(&project, &b, &helper_of(&a, Some(20)), Defaults::default(), T0).unwrap().unwrap();
    let level2 = admit(&project, &c, &helper_of(&b, Some(10)), Defaults::default(), T0).unwrap().unwrap();
    assert_eq!((level1.depth, level2.depth), (1, 2));
    let error = admit(&project, &d, &helper_of(&c, Some(1)), Defaults::default(), T0).unwrap_err();
    assert_eq!(error.code, "BUDGET_EXHAUSTED");
    assert_eq!(error.context["reason"], "depth_limit");
    assert_eq!(error.context["max_depth"], MAX_DEPTH);
    assert!(record_of(&project, &d).is_none());
}

#[test]
fn a_parent_without_a_budget_is_no_parent() {
    let project = project();
    let child = handle();
    // The caller names a run that holds no budget: the new run is a root, so no default and no
    // arguments means no budget (today's behaviour).
    assert_eq!(admit(&project, &child, &helper_of("no-such-run", None), Defaults::default(), T0).unwrap(), None);
    let own = admit(&project, &handle(), &helper_of("no-such-run", Some(3)), Defaults::default(), T0).unwrap().unwrap();
    assert_eq!((own.depth, own.parent), (0, None));
}

#[test]
fn adding_turns_to_a_helper_raises_every_run_above_it() {
    let project = project();
    let (a, b, c) = (handle(), handle(), handle());
    root(&project, &a, 40);
    admit(&project, &b, &helper_of(&a, Some(20)), Defaults::default(), T0).unwrap();
    admit(&project, &c, &helper_of(&b, Some(10)), Defaults::default(), T0).unwrap();
    add_turns(&project, &c, 5).unwrap();
    assert_eq!(record_of(&project, &c).unwrap().turns, Some(15));
    assert_eq!(record_of(&project, &b).unwrap().turns, Some(25));
    assert_eq!(record_of(&project, &a).unwrap().turns, Some(45));
    // The invariant holds: a helper's share never exceeds what its parent had to give.
    assert_eq!(standing(&record_of(&project, &a).unwrap(), T0).turns_left(), Some(45 - 25));
}

#[test]
fn a_failed_start_gives_the_share_back() {
    let project = project();
    let (parent, child) = (handle(), handle());
    root(&project, &parent, 10);
    admit(&project, &child, &helper_of(&parent, Some(6)), Defaults::default(), T0).unwrap();
    assert_eq!(standing(&record_of(&project, &parent).unwrap(), T0).turns_left(), Some(4));
    discard(&project, &child); // the run never got an operation
    assert_eq!(standing(&record_of(&project, &parent).unwrap(), T0).turns_left(), Some(10));
}

#[test]
fn an_identical_retry_gets_the_same_budget_not_a_second_share() {
    let project = project();
    let (parent, child) = (handle(), handle());
    root(&project, &parent, 10);
    let first = admit(&project, &child, &helper_of(&parent, Some(4)), Defaults::default(), T0).unwrap().unwrap();
    let again = admit(&project, &child, &helper_of(&parent, Some(4)), Defaults::default(), T0 + 5).unwrap().unwrap();
    assert_eq!(first, again);
    assert_eq!(standing(&record_of(&project, &parent).unwrap(), T0).turns_reserved, 4);
}

// ---- the MCP surface: additive, optional ----------------------------------------------------

#[test]
fn run_agent_request_takes_the_budget_args_and_they_default_to_none() {
    use crate::agent_runs::AgentRunRequest;
    let plain: AgentRunRequest = serde_json::from_value(json!({"task": "t"})).unwrap();
    assert_eq!((plain.budget_turns, plain.budget_minutes, plain.budget_parent), (None, None, None));
    let with: AgentRunRequest =
        serde_json::from_value(json!({"task": "t", "budget_turns": 5, "budget_minutes": 30, "budget_parent": "h1"})).unwrap();
    assert_eq!((with.budget_turns, with.budget_minutes, with.budget_parent.as_deref()), (Some(5), Some(30), Some("h1")));
}

#[test]
fn run_agent_schema_gains_only_optional_budget_args() {
    let tools = crate::tool_registry::tools();
    let run = tools.iter().find(|tool| tool.name == "run_agent").expect("run_agent");
    let props = run.input_schema["properties"].as_object().unwrap();
    for key in ["budget_turns", "budget_minutes", "budget_parent"] {
        assert!(props.contains_key(key), "{key}");
    }
    assert_eq!(run.input_schema["required"], json!(["task"]), "nothing new is required");
    assert_eq!(props["budget_turns"]["minimum"], 1);
    // The other tools are untouched: followup_task takes no budget args.
    let followup = tools.iter().find(|tool| tool.name == "followup_task").unwrap();
    assert!(followup.input_schema["properties"].get("budget_turns").is_none());
}

// ---- a stopped helper refunds what it did not spend (missions hand off to the next agent) ---------

/// A root that is not a run handle (a mission key) shares with helpers; when the first helper is
/// stopped ("Stop here", or a mission handing off), only the turns it spent stay counted and the
/// next helper can get the rest. Before: a stopped helper kept its whole share reserved, so the
/// root looked used up and the next agent could not be admitted.
#[test]
fn a_stopped_helper_gives_back_the_turns_it_did_not_use() {
    let project = project();
    let mission = format!("mission-{}", &Uuid::new_v4().to_string()[..8]);
    root(&project, &mission, 10);
    let first = handle();
    let share = admit(&project, &first, &helper_of(&mission, Some(10)), Defaults::default(), T0).unwrap().unwrap();
    assert_eq!(share.turns, Some(10));
    finish(&project, &start_turn(&project, &first, 0));
    finish(&project, &start_turn(&project, &first, 1));
    let parent = super::store::get(&project, &mission).unwrap();
    assert_eq!(standing(&parent, T0).turns_left(), Some(0), "the active helper holds the whole share");

    stop(&project, &first).unwrap();
    let parent = super::store::get(&project, &mission).unwrap();
    assert_eq!(standing(&parent, T0).turns_left(), Some(8), "only the 2 turns it spent stay counted");
    let second = handle();
    let next = admit(&project, &second, &helper_of(&mission, Some(8)), Defaults::default(), T0).unwrap().unwrap();
    assert_eq!(next.turns, Some(8));
}

// ---- a companion's helpers -----------------------------------------------------------------

#[test]
fn a_helper_started_by_a_companion_gets_twenty_turns_unless_the_person_set_a_limit() {
    let project = project();
    let (a, b, c, d, e) = (handle(), handle(), handle(), handle(), handle());
    // Nothing set anywhere: 20 turns, as a default (not as something the caller asked for).
    let rec = admit(&project, &a, &Asked::default(), with_companion_default(Defaults::default(), true), T0).unwrap().unwrap();
    assert_eq!((rec.turns, rec.minutes, rec.source), (Some(20), None, Source::Default));
    assert_eq!(rec.turns, Some(crate::companion::delegate::HELPER_TURNS));
    // An agent that is not a companion's keeps today's behaviour: no budget.
    assert!(admit(&project, &b, &Asked::default(), with_companion_default(Defaults::default(), false), T0).unwrap().is_none());
    // The person's own Settings default wins.
    let own = Defaults { turns: Some(5), minutes: None };
    let rec = admit(&project, &c, &Asked::default(), with_companion_default(own, true), T0).unwrap().unwrap();
    assert_eq!(rec.turns, Some(5));
    // So does a limit on the run itself.
    let rec = admit(&project, &d, &asked(Some(7), None), with_companion_default(Defaults::default(), true), T0).unwrap().unwrap();
    assert_eq!((rec.turns, rec.source), (Some(7), Source::Argument));
    // A time limit in Settings stays, and the turn limit is still filled in.
    let minutes = Defaults { turns: None, minutes: Some(60) };
    let rec = admit(&project, &e, &Asked::default(), with_companion_default(minutes, true), T0).unwrap().unwrap();
    assert_eq!((rec.turns, rec.minutes), (Some(20), Some(60)));
}

#[test]
fn the_companions_twenty_turn_helper_is_paused_before_turn_twenty_one() {
    let project = project();
    let h = handle();
    admit(&project, &h, &Asked::default(), with_companion_default(Defaults::default(), true), T0).unwrap().unwrap();
    for index in 0..20 {
        check_next_turn_at(&project, &h, T0).unwrap_or_else(|e| panic!("turn {index} may start: {}", e.message));
        let turn = start_turn(&project, &h, index);
        finish(&project, &turn);
    }
    let error = check_next_turn_at(&project, &h, T0).unwrap_err();
    assert_eq!(error.code, "BUDGET_EXHAUSTED");
    assert_eq!(error.context["turns_limit"], 20);
}
