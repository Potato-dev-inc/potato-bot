//! Budgets: a brake on how much an agent run may do before it must ask a person.
//!
//! - A run can hold a budget of turns and/or wall-clock minutes (`budget_turns` /
//!   `budget_minutes` on `run_agent`, or the defaults in Settings; both are OFF unless set).
//! - Helpers an agent starts with `run_agent` draw their share from the parent's remaining
//!   budget, at most two levels below the first budget.
//! - Turns are counted from the operation records. Provider usage and tokens are not read.
//! - When a budget is used up the run is PAUSED BEFORE ITS NEXT TURN, never stopped mid-turn:
//!   the caller gets `BUDGET_EXHAUSTED` and an Inbox card offers "Add 10 turns / Stop here".
//!
//! Layout: `model` (types), `store` (records on disk), `accounting` (what has been used),
//! `admit` (a new run's budget and the helper rules), `gate` (the pause), `card` (the Inbox
//! card), `adjust` (add / stop), `view` (the header chip), `errors`, `pane_usage` (turns and busy
//! time per helper pane, rolled up through `parent_pane_id`).
//!
//! Escalating a helper's questions to its parent first (roadmap T3, P2) is not built here.

mod accounting;
mod adjust;
mod admit;
mod card;
mod errors;
mod gate;
mod model;
mod store;
mod view;
#[cfg(test)]
mod tests;

pub use admit::{admit, discard, validate, with_companion_default};
pub use card::{on_answered, Outcome, KIND as CARD_KIND};
pub use gate::{check_next_turn, turn_ended};
pub use model::{Asked, Defaults, PARENT_ENV};
pub use store::get as record_of;

/// Hook for the routes that answer a question: applies the choice when the question was a pause
/// card. Returns the plain sentence to show the person ("Added 10 turns."), if any.
pub fn answer_applied(question: &crate::questions::Question) -> Option<String> {
    on_answered(question).map(|outcome| match outcome {
        Outcome::AddedTurns(n) => format!("Added {n} {}.", if n == 1 { "turn" } else { "turns" }),
        Outcome::AddedMinutes(n) => format!("Added {n} min."),
        Outcome::Stopped => "Stopped at its budget.".to_string(),
    })
}
