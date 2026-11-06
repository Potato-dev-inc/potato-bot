//! `wait_until` and `wait_cancel` in the engine (`waits.rs` has the rules and the timer).
//!
//! Who may: the owner, in their private chat with the bot (`app:dm:<bot>`), and not in a routine's own run (so a woken bot or a
//! scheduled run cannot start more waits by itself). A group is refused by its conversation, a stranger by their trust. The
//! check needs the tool it uses: a command needs the bot's Computer switch, an address its Web switch (and the owner's site
//! list), and the computer must be set up. The same checks run again every time the timer looks, so a switch turned off later
//! ends the wait with a plain note.

use super::*;
use crate::potato::waits;

impl Service {
    /// The answer to `wait_until` and `wait_cancel`.
    pub(super) fn wait_tool_item(&self, ctx: &sandbox_calls::Context, call_id: &str, tool: &str, args: &Value) -> Value {
        let refuse = |code: &str, message: &str| protocol::sandbox_result_err(call_id, code, message, false);
        if ctx.conversation != store::app_conversation(&ctx.bot) {
            return refuse("TOOL_NOT_ALLOWED", "A bot waits only in the private chat with its owner, not in a group.");
        }
        if !ctx.trust.is_owner() {
            return refuse("TOOL_NOT_ALLOWED", "Only the owner can have this bot wait for something.");
        }
        let from_routine = self.inner.lock().bots.get(&ctx.bot).is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation));
        if from_routine {
            return refuse("TOOL_NOT_ALLOWED", "A routine or a finished wait cannot start another wait. Tell the owner what you found; they can ask for a new one.");
        }
        let allow_local = self.calls.allows_local_pages();
        if tool == "wait_until" {
            if !self.brains.enabled() {
                return refuse("POTATO_OFF", "Potato Bot is switched off.");
            }
            if !self.sandbox_on.load(Ordering::SeqCst) {
                return refuse("SANDBOX_OFF", "The bot's computer is switched off. Turn on Sandbox in Settings > General > Labs.");
            }
            if self.port.lock().is_none() {
                return refuse("SANDBOX_NOT_SET_UP", "The bot's computer is not set up yet. Set it up in Settings > Potato Bot.");
            }
            // Read first, so the answer names the switch that is off before a wait is made.
            let made = match waits::make(&ctx.bot, args, allow_local, self.host.now_ms()) {
                Ok(wait) => wait,
                Err(e) => return refuse(e.code, &e.message),
            };
            let tool_needed = if made.check.kind == super::super::checks::CheckKind::Url { "web_fetch" } else { "run_terminal" };
            let tools = bot_settings::load(&self.root, &ctx.bot).unwrap_or_default().tools;
            if !bot_settings::sandbox_tool_on(&tools, tool_needed) {
                return refuse("TOOL_OFF", "The tool this check needs is off for this bot. Turn it on in its panel, under Advanced.");
            }
        }
        match waits::call(&self.root, &ctx.bot, tool, args, allow_local, self.host.now_ms()) {
            Ok(result) => protocol::sandbox_result_ok(call_id, result),
            Err(e) => refuse(e.code, &e.message),
        }
    }
}
