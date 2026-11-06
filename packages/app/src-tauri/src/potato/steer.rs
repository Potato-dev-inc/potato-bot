//! Live steering, the app's side. A person's message that arrives while a turn runs is kept in the thread with a quiet
//! "Queued" label until the bot has picked it up (the brain reads it between its tool rounds). Here: which words mean
//! "how is it going?", and the app's own note for a status question when one tool call has been running a long time.
//! That note is built from the saved job record and the live step only. It is never a model call and it is marked as the
//! app's note, so it is never mistaken for the bot's own words.

use super::jobs::Job;
use super::work::WorkRun;

/// A tool call that has run this long is "slow": a status question gets the app's note without waiting for the bot.
pub const SLOW_STEP_MS: i64 = 20_000;
/// At most one such note per this long, so repeated "status?" does not fill the thread.
pub const NOTE_EVERY_MS: i64 = 30_000;

/// True for a short message that asks how the work is going.
pub fn is_status_question(text: &str) -> bool {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if flat.is_empty() || flat.chars().count() > 60 {
        return false;
    }
    const WORDS: [&str; 16] = [
        "status", "progress", "how is it going", "how's it going", "hows it going", "any update", "update?", "where are you", "are you still", "still working",
        "still there", "eta", "done yet", "how long", "what are you doing", "what's happening",
    ];
    WORDS.iter().any(|w| flat.contains(w))
}

fn minutes_in(ms: i64) -> String {
    let secs = (ms.max(0) / 1000) as u64;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// The app's note, or `None` when no tool call has run long enough. Built only from the record the app keeps.
pub fn slow_step_note(name: &str, run: &WorkRun, job: Option<&Job>, now: i64) -> Option<String> {
    let step = run.steps.iter().rev().find(|s| s.status == "running")?;
    if now - step.started_ms < SLOW_STEP_MS {
        return None;
    }
    let what = step.detail.clone().unwrap_or_else(|| step.tool.replace('_', " "));
    let mut line = match job {
        Some(job) => format!(
            "Working on: {} \u{b7} step {} \u{b7} {} in. Right now: {} (running {}).",
            job.goal,
            job.steps_total + 1,
            minutes_in(job.elapsed_ms(now)),
            what,
            minutes_in(now - step.started_ms)
        ),
        None => format!("Working on a long step: {} (running {}).", what, minutes_in(now - step.started_ms)),
    };
    line.push_str(&format!(" This is a note from the app, not from {name}: your message is queued and {name} reads it between steps."));
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::potato::protocol::ProgressPhase;

    #[test]
    fn status_words_are_found_and_long_messages_are_not() {
        for yes in ["status?", "Status", "how is it going?", "any update", "still working?", "what are you doing", "eta?"] {
            assert!(is_status_question(yes), "{yes}");
        }
        for no in ["", "please rename the folder", "can you write a long report about progress in the field of cooking and then email it to me later", "hello"] {
            assert!(!is_status_question(no), "{no}");
        }
    }

    #[test]
    fn the_note_needs_a_step_that_ran_long_and_names_the_job() {
        let mut run = WorkRun::new(0);
        run.apply("s1", "run_terminal", ProgressPhase::Start, true, None, 1_000);
        assert_eq!(slow_step_note("Nimbus", &run, None, 10_000), None, "9 seconds is not slow");
        let job = Job { goal: "Tidy the project folder".into(), steps_total: 11, started_ms: 0, turn_started_ms: Some(30_000), ..Job::default() };
        let note = slow_step_note("Nimbus", &run, Some(&job), 330_000).expect("a note");
        assert!(note.contains("Working on: Tidy the project folder"), "{note}");
        assert!(note.contains("step 12") && note.contains("5m"), "{note}");
        assert!(note.contains("note from the app, not from Nimbus"), "{note}");
        run.apply("s1", "run_terminal", ProgressPhase::End, true, None, 331_000);
        assert_eq!(slow_step_note("Nimbus", &run, Some(&job), 332_000), None, "nothing is running now");
    }
}
