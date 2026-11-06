//! Settings from chat in the engine (`self_settings.rs` has the rules and the journal). A bot may change its own name,
//! instructions, persona and model when its owner asks it to, in their private chat, and the thread shows a card with Undo.
//!
//! Who may: the owner, in `app:dm:<bot>` and nowhere else, never in a routine's own run, and never in a turn that has already
//! read outside text. Text that came from a page, a file, a command's output, a tool server or the chat history may carry
//! words meant for the model; a turn that read any of it cannot change a setting, so those words cannot unlock one.
//! A group is refused by its conversation, a stranger by their trust. A bot set to "Ask first" does not change itself
//! at all: its owner does it in the Advanced tab (the rule that a new write follows "When to ask").
//!
//! The brain restarts after the turn (not during it) for the changes it reads at start: instructions, persona, model.

use super::*;
use crate::potato::risk::Level;
use crate::potato::self_settings::{self, Field};

impl Service {
    /// The answer to `change_my_setting`.
    pub(super) fn self_setting_item(&self, ctx: &sandbox_calls::Context, call_id: &str, args: &Value) -> Value {
        let refuse = |code: &str, message: &str| protocol::sandbox_result_err(call_id, code, message, false);
        if ctx.conversation != store::app_conversation(&ctx.bot) {
            return refuse("TOOL_NOT_ALLOWED", "A bot changes its own settings only in the private chat with its owner, not in a group.");
        }
        if !ctx.trust.is_owner() {
            return refuse("TOOL_NOT_ALLOWED", "Only the owner can have this bot change its settings.");
        }
        let (from_routine, tainted) = {
            let g = self.inner.lock();
            let rt = g.bots.get(&ctx.bot);
            (rt.is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation)), rt.is_some_and(|rt| rt.turn_tainted.contains(&ctx.conversation)))
        };
        if from_routine {
            return refuse("TOOL_NOT_ALLOWED", "A routine cannot change the bot's settings. Only the owner can, in a message.");
        }
        if tainted {
            return refuse(
                "TOOL_NOT_ALLOWED",
                "This turn already read text from outside the chat (a page, a file, a command, a tool or an old chat), so it cannot change settings. If your owner wants a change, they can ask again in a new message.",
            );
        }
        if super::super::risk::level_for(&self.root, &ctx.bot) == Level::AskFirst {
            return refuse("TOOL_NOT_ALLOWED", "This bot is set to Ask first, so it does not change itself. Tell your owner what you would change; they can do it in the Advanced tab.");
        }
        let (field, value) = match self_settings::parse_request(args) {
            Ok(parsed) => parsed,
            Err((code, message)) => return refuse(code, &message),
        };
        match self.apply_self_setting(&ctx.bot, &ctx.conversation, field, &value) {
            Ok(None) => protocol::sandbox_result_ok(call_id, serde_json::json!({ "ok": true, "changed": false, "note": "It was already set like that. Nothing changed." })),
            Ok(Some(note)) => protocol::sandbox_result_ok(call_id, serde_json::json!({ "ok": true, "changed": true, "setting": field.word(), "note": note })),
            Err(e) => refuse(e.code, &e.message),
        }
    }

    /// Changes one field, writes the journal entry and the card. `Ok(None)`: it was the value already.
    fn apply_self_setting(&self, bot: &str, conversation: &str, field: Field, value: &str) -> Result<Option<String>> {
        let prior = self.set_field(bot, field, value)?;
        if prior == value {
            return Ok(None);
        }
        let now = self.host.now_ms();
        let entry = self_settings::record(&self.root, bot, field, &prior, value, now)?;
        let mut card = Message::new(now, FromKind::Note, None, MsgKind::Note, &self_settings::describe(field, &prior, value));
        card.change = Some(store::SettingChange { id: entry.id, field: field.word().to_string(), undone: false });
        self.save(bot, conversation, card)?;
        if field.restarts() {
            self.restart_soon(bot);
        }
        self.host.changed(bot);
        Ok(Some("Done. Your owner sees a card in the chat with an Undo button. It takes effect from your next turn.".to_string()))
    }

    /// Writes the value into the bot's own files and returns what was there before. Checks what the field needs (a model
    /// can be changed only on a bot that uses its own provider). No note, no restart: the callers do those.
    fn set_field(&self, bot: &str, field: Field, value: &str) -> Result<String> {
        self.require_bot(bot)?;
        match field {
            Field::Name => {
                let prior = self.name_of(bot);
                if prior != value {
                    self.host.rename(bot, value).map_err(|m| err("POTATO_SETTING_FAILED", &m))?;
                }
                Ok(prior)
            }
            Field::Persona => {
                let prior = self.require_bot(bot)?.talk_persona;
                if prior != value {
                    store::set_talk_persona(&self.root, bot, value)?;
                }
                Ok(prior)
            }
            Field::Instructions => {
                let now = self.host.now_ms();
                let mut prior = String::new();
                bot_settings::update(&self.root, bot, |s| {
                    prior = bot_settings::current_instructions(s).to_string();
                    bot_settings::save_instructions(s, value, now)?;
                    Ok(())
                })?;
                Ok(prior)
            }
            Field::Model => {
                let default = store::load_settings(&self.root)?.brain;
                let mut prior = String::new();
                bot_settings::update(&self.root, bot, |s| {
                    let uses_own = bot_settings::effective_brain(default, s) == store::BrainKind::Custom;
                    match s.custom.as_mut() {
                        Some(custom) if uses_own && !custom.base_url.trim().is_empty() => {
                            prior = custom.model.clone();
                            custom.model = value.to_string();
                            Ok(())
                        }
                        _ => Err(err("POTATO_SETTING_NO_PROVIDER", "The model can only be changed when this bot uses its own API provider (set in its Advanced tab). Your owner can switch models there.")),
                    }
                })?;
                Ok(prior)
            }
        }
    }

    /// Undo on a settings card: puts the value from before back. The owner's own click (an app command); the model has no
    /// tool for it. A change can be undone once.
    pub fn undo_setting(&self, bot: &str, change_id: &str) -> Result<()> {
        self.require_bot(bot)?;
        let entry = self_settings::find(&self.root, bot, change_id)?;
        if entry.undone {
            return Err(err("POTATO_SETTING_UNDONE", "That change was already undone."));
        }
        let field = Field::parse(&entry.field).ok_or_else(|| err("POTATO_SETTING_CHANGE_GONE", "That change is not in the record any more."))?;
        self.set_field(bot, field, &entry.prior)?;
        self_settings::mark_undone(&self.root, change_id)?;
        let now = self.host.now_ms();
        let line = match field {
            Field::Name => format!("Undone: its name is {} again.", entry.prior),
            Field::Instructions => "Undone: its instructions are back as they were.".to_string(),
            Field::Persona => format!("Undone: it talks as {} again.", self_settings::persona_name(&entry.prior)),
            Field::Model => format!("Undone: its model is {} again.", entry.prior),
        };
        self.save(bot, &store::app_conversation(bot), Message::new(now, FromKind::Note, None, MsgKind::Note, &line))?;
        if field.restarts() {
            self.restart_soon(bot);
        }
        self.host.changed(bot);
        Ok(())
    }

    /// Stops the brain so that it starts again with the new files: now when no turn is running, otherwise when the turn's
    /// usage comes in (`on_usage`), so a change made in a turn never cuts that turn short.
    pub(super) fn restart_soon(&self, bot: &str) {
        let busy = {
            let mut g = self.inner.lock();
            match g.bots.get_mut(bot) {
                Some(rt) if !rt.reserved.is_empty() || !rt.typing.is_empty() => {
                    rt.restart_after_turn = true;
                    true
                }
                _ => false,
            }
        };
        if !busy {
            self.stop_bot(bot);
        }
    }
}
