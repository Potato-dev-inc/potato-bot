//! Decision cards on files only (no engine). What it shows: one answer only; a card id that does not exist, or belongs to
//! another bot, is refused; a card, its answer and what it allowed survive a restart (a fresh read of the folder); a
//! cancelled card cannot be answered and its permissions are gone; "allow once" is for exactly one command, once;
//! "allow for this job" is for one class in one job; a card raised in a job puts the job to waiting and stops its clock,
//! and the answer is the only thing that flips it back.

use std::path::PathBuf;

use super::cards::*;
use super::jobs::{self, Status};
use super::store::{self, MsgKind};

const DM: &str = "app:dm:nimbus";

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-cards-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for bot in ["nimbus", "mochi"] {
        store::add_bot(&dir, bot, "fish", 1).unwrap();
        store::ensure_brain_dirs(&dir, bot).unwrap();
    }
    dir
}

fn approval(command: &str) -> New {
    New {
        kind: Kind::Approval,
        action: command.to_string(),
        why: "This command pushes to a remote.".to_string(),
        choices: Vec::new(),
        conversation: DM.to_string(),
        command_hash: Some(command_hash(command, None)),
        classes: vec!["git push".to_string()],
        keep_going: None,
        connector: None,
        login: None,
        mcp_add: None,
        speaker: "Nimbus".to_string(),
        home: None,
    }
}

fn question(text: &str, choices: &[&str]) -> New {
    New {
        kind: Kind::Question,
        action: text.to_string(),
        why: String::new(),
        choices: choices.iter().map(|c| c.to_string()).collect(),
        conversation: DM.to_string(),
        command_hash: None,
        classes: Vec::new(),
        keep_going: None,
        connector: None,
        login: None,
        mcp_add: None,
        speaker: "Nimbus".to_string(),
        home: None,
    }
}

fn raise_card(root: &PathBuf, new: New) -> Card {
    raise(root, "nimbus", new, 5_000).unwrap().card
}

fn answer_card(root: &PathBuf, id: &str, text: &str) -> Result<Answered, store::StoreError> {
    answer(root, "nimbus", id, text, None, 6_000)
}

#[test]
fn a_card_is_saved_open_with_an_id_and_a_row_in_the_thread() {
    let root = temp("save");
    let card = raise_card(&root, approval("git push origin main"));
    assert!(card.id.starts_with("card-") && card.id.len() == "card-".len() + 12, "{}", card.id);
    assert_eq!((card.kind, card.state), (Kind::Approval, State::Open));
    assert_eq!(open_cards(&root, "nimbus"), vec![card.clone()]);
    let rows = store::read_messages(&root, DM, 20, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, MsgKind::DecisionCard);
    assert_eq!(rows[0].request_id.as_deref(), Some(card.id.as_str()));
    assert_eq!(rows[0].text, "git push origin main");
    let wire = rows[0].card.as_ref().unwrap();
    assert_eq!(wire.kind, Kind::Approval);
    assert!(wire.choices.is_empty() && wire.answer.is_none() && !wire.keep_going);
}

#[test]
fn the_wire_format_of_a_card_row_and_its_result_row() {
    let root = temp("wire");
    let card = raise_card(&root, question("Which port?", &["3000", "8080"]));
    let first = serde_json::to_value(&store::read_messages(&root, DM, 20, None).unwrap()[0]).unwrap();
    assert_eq!(first["kind"], "decision_card");
    assert_eq!(first["requestId"], card.id.as_str());
    assert_eq!(first["text"], "Which port?");
    assert_eq!(first["card"], serde_json::json!({ "kind": "question", "why": "", "choices": ["3000", "8080"] }));
    answer_card(&root, &card.id, "1").unwrap();
    let rows = store::read_messages(&root, DM, 20, None).unwrap();
    let result = serde_json::to_value(&rows[1]).unwrap();
    assert_eq!(result["kind"], "decision_result");
    assert_eq!(result["requestId"], card.id.as_str());
    assert_eq!(result["text"], "You chose: 8080.");
    assert_eq!(result["card"]["answer"], "choice:1");
}

#[test]
fn one_answer_only_and_the_second_is_a_plain_error_that_changes_nothing() {
    let root = temp("once");
    let card = raise_card(&root, approval("git push"));
    let done = answer_card(&root, &card.id, "allow_once").unwrap();
    assert_eq!(done.answer, Answer::AllowOnce);
    for again in ["allow_once", "deny", "allow_job"] {
        let e = answer_card(&root, &card.id, again).unwrap_err();
        assert_eq!(e.code, "POTATO_CARD_ANSWERED", "{again}");
        assert_eq!(e.message, "That card was already answered.");
    }
    let kept = find(&root, "nimbus", &card.id).unwrap();
    assert_eq!((kept.state, kept.answer.as_deref()), (State::Answered, Some("allow_once")));
    // the deny that was refused did not forget the allow, and did not add a refusal
    let hash = command_hash("git push", None);
    assert!(has_once(&root, "nimbus", &hash, 6_000));
    assert!(!is_denied(&root, "nimbus", &hash, 6_000));
    assert_eq!(store::read_messages(&root, DM, 20, None).unwrap().iter().filter(|m| m.kind == MsgKind::DecisionResult).count(), 1, "one result row");
}

#[test]
fn a_forged_card_id_is_refused_and_so_is_another_bots_card() {
    let root = temp("forged");
    let card = raise_card(&root, approval("git push"));
    for forged in ["card-000000000000", "", "nope", "card-", &"x".repeat(500), "../cards", &card.id.to_uppercase()] {
        let e = answer(&root, "nimbus", forged, "allow_once", None, 6_000).unwrap_err();
        assert_eq!(e.code, "POTATO_CARD_NONE", "{forged:?}");
    }
    // the real id, but asked as another bot: that bot's file has no such card
    let e = answer(&root, "mochi", &card.id, "allow_once", None, 6_000).unwrap_err();
    assert_eq!(e.code, "POTATO_CARD_NONE");
    // an unknown bot
    let e = answer(&root, "ghost", &card.id, "allow_once", None, 6_000).unwrap_err();
    assert_eq!(e.code, "POTATO_BOT_NOT_FOUND");
    assert_eq!(find(&root, "nimbus", &card.id).unwrap().state, State::Open, "nothing was answered");
    assert!(!has_once(&root, "nimbus", &command_hash("git push", None), 6_000));
}

#[test]
fn an_answer_must_be_one_of_the_cards_own() {
    let root = temp("bad-answer");
    let approve = raise_card(&root, approval("git push"));
    for bad in ["", "maybe", "0", "1", "choice:0", "ALLOW_ONCE", "allow once"] {
        assert_eq!(answer_card(&root, &approve.id, bad).unwrap_err().code, "POTATO_CARD_BAD_ANSWER", "{bad:?}");
    }
    let ask = raise_card(&root, question("Which port?", &["3000", "8080"]));
    for bad in ["", "2", "-1", "allow_once", "deny", "x", "1.5", "99999999999999999999"] {
        assert_eq!(answer_card(&root, &ask.id, bad).unwrap_err().code, "POTATO_CARD_BAD_ANSWER", "{bad:?}");
    }
    assert_eq!(open_cards(&root, "nimbus").len(), 2, "a bad answer leaves both open");
    assert!(answer_card(&root, &ask.id, "0").is_ok());
    assert!(answer_card(&root, &approve.id, "deny").is_ok());
}

#[test]
fn a_card_its_answer_and_what_it_allowed_survive_a_restart() {
    let root = temp("restart");
    let open = raise_card(&root, approval("git push origin a"));
    let answered = raise_card(&root, approval("git push origin b"));
    answer_card(&root, &answered.id, "allow_once").unwrap();
    // a restart forgets every object in memory: only the folder is left
    let again = root.clone();
    assert_eq!(open_cards(&again, "nimbus"), vec![open.clone()]);
    assert!(has_once(&again, "nimbus", &command_hash("git push origin b", None), 7_000));
    assert_eq!(answer(&again, "nimbus", &answered.id, "deny", None, 7_000).unwrap_err().code, "POTATO_CARD_ANSWERED");
    // the open one can still be answered after the restart, once
    assert!(answer(&again, "nimbus", &open.id, "deny", None, 7_000).is_ok());
    assert_eq!(answer(&again, "nimbus", &open.id, "deny", None, 7_000).unwrap_err().code, "POTATO_CARD_ANSWERED");
    assert!(is_denied(&again, "nimbus", &command_hash("git push origin a", None), 7_000));
}

#[test]
fn a_damaged_cards_file_reads_as_nothing_open_and_nothing_allowed() {
    let root = temp("damaged");
    let card = raise_card(&root, approval("git push"));
    answer_card(&root, &card.id, "allow_once").unwrap();
    std::fs::write(store::state_dir(&root, "nimbus").unwrap().join(FILE), "{ not json").unwrap();
    assert!(open_cards(&root, "nimbus").is_empty());
    assert!(!has_once(&root, "nimbus", &command_hash("git push", None), 6_000), "a damaged file allows nothing");
    assert!(store::state_dir(&root, "nimbus").unwrap().join(ASIDE_FILE).exists(), "kept aside");
    assert_eq!(answer_card(&root, &card.id, "allow_once").unwrap_err().code, "POTATO_CARD_NONE");
}

#[test]
fn cancelling_closes_open_cards_and_forgets_what_was_allowed() {
    let root = temp("cancel");
    let open = raise_card(&root, approval("git push origin a"));
    let allowed = raise_card(&root, approval("git push origin b"));
    answer_card(&root, &allowed.id, "allow_once").unwrap();
    assert_eq!(cancel_all(&root, "nimbus", None, 8_000), 1);
    let e = answer_card(&root, &open.id, "allow_once").unwrap_err();
    assert_eq!(e.code, "POTATO_CARD_CANCELLED");
    assert!(e.message.contains("cancelled"));
    assert_eq!(find(&root, "nimbus", &open.id).unwrap().state, State::Cancelled);
    assert_eq!(find(&root, "nimbus", &allowed.id).unwrap().state, State::Answered, "an answered card is not rewritten");
    assert!(!has_once(&root, "nimbus", &command_hash("git push origin b", None), 8_000), "what was allowed is forgotten");
    assert!(!has_once(&root, "nimbus", &command_hash("git push origin a", None), 8_000));
    let rows = store::read_messages(&root, DM, 20, None).unwrap();
    let cancelled: Vec<_> = rows.iter().filter(|m| m.kind == MsgKind::DecisionResult && m.text == "Cancelled.").collect();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].card.as_ref().unwrap().answer.as_deref(), Some("cancelled"));
    assert_eq!(cancel_all(&root, "nimbus", None, 9_000), 0, "nothing left to cancel");
}

#[test]
fn allow_once_is_for_exactly_that_command_and_one_time() {
    let root = temp("exact");
    let card = raise_card(&root, approval("git push origin main"));
    answer_card(&root, &card.id, "allow_once").unwrap();
    let same = command_hash("git push origin main", None);
    assert!(has_once(&root, "nimbus", &same, 6_000));
    for other in ["git push origin dev", "git push origin main --force", "git push origin  main", "git  push origin main", "bash -c 'git push origin main'", "git push"] {
        assert!(!has_once(&root, "nimbus", &command_hash(other, None), 6_000), "{other}");
    }
    // the same text in another folder is another command
    assert!(!has_once(&root, "nimbus", &command_hash("git push origin main", Some("/home/pm/other")), 6_000));
    // surrounding space does not matter, the words do
    assert!(has_once(&root, "nimbus", &command_hash("  git push origin main \n", None), 6_000));
    assert!(consume_once(&root, "nimbus", &same, 6_000));
    assert!(!consume_once(&root, "nimbus", &same, 6_000), "used up");
    assert!(!has_once(&root, "nimbus", &same, 6_000));
}

#[test]
fn an_allow_once_that_was_never_used_runs_out() {
    let root = temp("expire");
    let card = raise_card(&root, approval("git push"));
    answer_card(&root, &card.id, "allow_once").unwrap();
    let hash = command_hash("git push", None);
    assert!(has_once(&root, "nimbus", &hash, 6_000 + ONCE_MEMORY_MS));
    assert!(!has_once(&root, "nimbus", &hash, 6_001 + ONCE_MEMORY_MS));
    assert!(!consume_once(&root, "nimbus", &hash, 6_001 + ONCE_MEMORY_MS));
}

#[test]
fn allow_for_this_job_covers_the_class_in_that_job_only() {
    let root = temp("jobgrant");
    let job = jobs::start(&root, "nimbus", DM, "Ship it", 1_000).unwrap();
    let card = raise_card(&root, approval("git push origin main"));
    assert_eq!(card.job.as_deref(), Some(job.id.as_str()));
    answer_card(&root, &card.id, "allow_job").unwrap();
    assert!(job_allows(&root, "nimbus", &job.id, &["git push"]));
    assert!(!job_allows(&root, "nimbus", &job.id, &["git push --force"]), "another class");
    assert!(!job_allows(&root, "nimbus", &job.id, &["git push", "publish"]), "every class of the command must be allowed");
    assert!(!job_allows(&root, "nimbus", "job-other", &["git push"]), "another job");
    assert!(!job_allows(&root, "nimbus", &job.id, &[]), "a command with no class is not covered");
    assert!(!job_allows(&root, "mochi", &job.id, &["git push"]), "another bot");
    // Stop forgets it
    cancel_all(&root, "nimbus", None, 9_000);
    assert!(!job_allows(&root, "nimbus", &job.id, &["git push"]));
}

#[test]
fn allow_for_this_job_without_a_job_is_only_an_allow_once() {
    let root = temp("jobgrant-nojob");
    let card = raise_card(&root, approval("git push"));
    assert!(card.job.is_none());
    answer_card(&root, &card.id, "allow_job").unwrap();
    assert!(has_once(&root, "nimbus", &command_hash("git push", None), 6_000));
    assert!(!job_allows(&root, "nimbus", "job-x", &["git push"]));
}

#[test]
fn deny_is_remembered_for_that_command_for_a_while() {
    let root = temp("deny");
    let card = raise_card(&root, approval("git push --force"));
    answer_card(&root, &card.id, "deny").unwrap();
    let hash = command_hash("git push --force", None);
    assert!(is_denied(&root, "nimbus", &hash, 6_000));
    assert!(!is_denied(&root, "nimbus", &command_hash("git push", None), 6_000));
    assert!(!is_denied(&root, "nimbus", &hash, 6_001 + DENY_MEMORY_MS));
    assert!(!has_once(&root, "nimbus", &hash, 6_000), "a no allows nothing");
}

#[test]
fn a_second_ask_for_the_same_command_finds_the_open_card() {
    let root = temp("dedupe");
    let first = raise_card(&root, approval("git push"));
    let second = raise_card(&root, approval("git push"));
    assert_eq!(first.id, second.id);
    assert_eq!(open_cards(&root, "nimbus").len(), 1);
    assert_eq!(store::read_messages(&root, DM, 20, None).unwrap().len(), 1, "one row in the thread");
    answer_card(&root, &first.id, "deny").unwrap();
    let third = raise_card(&root, approval("git push"));
    assert_ne!(first.id, third.id, "after the answer a new ask is a new card");
}

#[test]
fn there_is_a_limit_on_open_cards() {
    let root = temp("many");
    for n in 0..MAX_OPEN {
        raise_card(&root, approval(&format!("git push origin b{n}")));
    }
    let e = raise(&root, "nimbus", approval("git push origin one-more"), 5_000).err().unwrap();
    assert_eq!(e.code, "POTATO_CARD_TOO_MANY");
}

#[test]
fn a_question_needs_choices_and_an_approval_has_none() {
    let root = temp("shape");
    assert_eq!(raise(&root, "nimbus", question("Which?", &[]), 5_000).err().unwrap().code, "POTATO_CARD_INVALID");
    assert_eq!(raise(&root, "nimbus", question("Which?", &["", "  "]), 5_000).err().unwrap().code, "POTATO_CARD_INVALID");
    assert_eq!(raise(&root, "nimbus", approval("   "), 5_000).err().unwrap().code, "POTATO_CARD_INVALID");
    let mut with_choices = approval("git push");
    with_choices.choices = vec!["yes".into()];
    assert!(raise(&root, "nimbus", with_choices, 5_000).unwrap().card.choices.is_empty());
    let many = question("Which?", &["a", "b", "c", "d", "e", "f"]);
    assert_eq!(raise(&root, "nimbus", many, 5_000).unwrap().card.choices.len(), MAX_CHOICES);
}

#[test]
fn a_card_in_a_job_puts_the_job_to_waiting_and_stops_its_clock() {
    let root = temp("waiting");
    let job = jobs::start(&root, "nimbus", DM, "Ship it", 1_000).unwrap();
    assert_eq!(job.turn_started_ms, Some(1_000));
    let card = raise(&root, "nimbus", approval("git push"), 61_000).unwrap().card;
    let waiting = jobs::load(&root, "nimbus").unwrap();
    assert_eq!(waiting.status, Status::Waiting);
    assert_eq!(waiting.waiting.as_deref(), Some(card.id.as_str()));
    assert_eq!(waiting.spent_ms, 60_000, "the working time up to the card is kept");
    assert_eq!(waiting.turn_started_ms, None);
    // waiting does not count: hours later the job has used the same minute
    assert_eq!(waiting.elapsed_ms(61_000 + 5 * 60 * 60 * 1000), 60_000);
    assert_eq!(waiting.limit_hit(61_000 + 5 * 60 * 60 * 1000), None);
}

#[test]
fn the_answer_flips_the_job_back_with_the_decision_in_its_record() {
    let root = temp("flip");
    jobs::start(&root, "nimbus", DM, "Ship it", 1_000).unwrap();
    let card = raise(&root, "nimbus", approval("git push"), 61_000).unwrap().card;
    let done = answer_card(&root, &card.id, "allow_once").unwrap();
    assert_eq!(done.job, JobEffect::Resumed);
    let job = jobs::load(&root, "nimbus").unwrap();
    assert_eq!((job.status, job.waiting.clone()), (Status::Running, None));
    let decision = job.decision.clone().unwrap();
    assert!(decision.contains("Allow once") && decision.contains("git push"), "{decision}");
    assert_eq!(done.decision, decision);
    // a refused answer does not touch the job
    let root2 = temp("flip2");
    jobs::start(&root2, "nimbus", DM, "Ship it", 1_000).unwrap();
    let card2 = raise(&root2, "nimbus", approval("git push"), 61_000).unwrap().card;
    answer(&root2, "nimbus", &card2.id, "bogus", None, 62_000).unwrap_err();
    answer(&root2, "nimbus", "card-000000000000", "allow_once", None, 62_000).unwrap_err();
    assert_eq!(jobs::load(&root2, "nimbus").unwrap().status, Status::Waiting);
}

#[test]
fn a_card_with_no_job_leaves_job_effect_none() {
    let root = temp("nojob");
    let card = raise_card(&root, approval("git push"));
    assert_eq!(answer_card(&root, &card.id, "deny").unwrap().job, JobEffect::None);
}

#[test]
fn keep_going_adds_one_more_period_of_the_same_size_and_stop_stops() {
    for (why, check) in [
        (jobs::reason::TIME_LIMIT, 0),
        (jobs::reason::MONEY_LIMIT, 1),
        (jobs::reason::NO_PROGRESS, 2),
    ] {
        let root = temp(&format!("keep-{why}"));
        jobs::start(&root, "nimbus", DM, "Ship it", 1_000).unwrap();
        let mut job = jobs::load(&root, "nimbus").unwrap();
        job.no_progress = 3;
        jobs::save(&root, "nimbus", &job).unwrap();
        let mut new = question("The job reached a limit.", &[KEEP_GOING, STOP]);
        new.keep_going = Some(why.to_string());
        let card = raise(&root, "nimbus", new, 2_000).unwrap().card;
        answer_card(&root, &card.id, "0").unwrap();
        let after = jobs::load(&root, "nimbus").unwrap();
        assert_eq!(after.status, Status::Running, "{why}");
        match check {
            0 => assert_eq!((after.extra_ms, after.max_ms()), (jobs::MAX_JOB_MS, 2 * jobs::MAX_JOB_MS)),
            1 => assert_eq!((after.extra_nano, after.max_nano()), (jobs::MAX_JOB_NANO, 2 * jobs::MAX_JOB_NANO)),
            _ => assert_eq!(after.no_progress, 0),
        }
    }
    let root = temp("keep-stop");
    jobs::start(&root, "nimbus", DM, "Ship it", 1_000).unwrap();
    let mut new = question("The job reached a limit.", &[KEEP_GOING, STOP]);
    new.keep_going = Some(jobs::reason::TIME_LIMIT.to_string());
    let card = raise(&root, "nimbus", new, 2_000).unwrap().card;
    let done = answer_card(&root, &card.id, "1").unwrap();
    assert!(matches!(done.job, JobEffect::Stopped(_)));
    let job = jobs::load(&root, "nimbus").unwrap();
    assert_eq!((job.status, job.stop_reason.as_deref()), (Status::Stopped, Some(jobs::reason::TIME_LIMIT)));
    assert_eq!((job.extra_ms, job.extra_nano), (0, 0), "Stop grants nothing");
}
