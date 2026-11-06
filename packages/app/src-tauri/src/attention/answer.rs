//! `POST /attention/{id}/answer`: a paired person answers one item of the attention list from the
//! phone card (or, later, the lock screen).
//!
//! One function, [`answer`], and an [`Env`] that carries everything outside it (the clock, the
//! snapshot, the live screen, the answer route), so the whole sequence is tested with a fake screen
//! and a counting writer. The sequence (design 4.2):
//!
//! 1. the caller holds the `answer` capability (403 otherwise);
//! 2. a retry with the same `answer_key` gets the first result and never presses twice (ledger);
//! 3. the item is looked up: a tombstone says who answered, a missing item is `gone`;
//! 4. an approval or trust gate is checked against what the card showed (prompt id, occurrence,
//!    fingerprint) and, UNDER the existing answer lock, against the live screen again. A mismatch is
//!    `changed` (a different prompt is up) or `gone` (none is), and nothing is typed;
//! 5. the key is typed through the current answer route, under an [`AnswerPermit`] (a person's
//!    decision about this prompt, refused when the pane printed since it was re-read), never a
//!    `GatePermit`;
//! 6. the result is recorded: the ledger, the item's tombstone ("Allowed once from Anna's iPhone"),
//!    the audit log.
//!
//! A chat answer (`Via::Telegram`) is refused for a trust gate, a consent card, a budget, a limit, a
//! person-only question card (an issue card) and any approval above low risk, right after the item is
//! looked up and before any key is typed (see `chat_refusal` and `person_only_question_refusal`).
//!
//! A question, a consent card, a budget or a usage-limit card goes to the questions store (no pane
//! write). This route is for paired people only: an agent answers through `answer_prompt`, where the
//! agent-authority consent rules apply.


/// "Allowed once", "Denied": the receipt wording; any other choice keeps its own label.
pub fn answered_label(choice_id: &str, label: &str) -> String {
    match choice_id {
        "allow_once" => "Allowed once".to_string(),
        "deny" => "Denied".to_string(),
        _ => label.trim().to_string(),
    }
}
