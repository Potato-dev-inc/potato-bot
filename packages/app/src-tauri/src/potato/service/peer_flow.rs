//! One of the owner's bots writing to another of the owner's bots (`message_bot`). The owner asks bot A in their private chat
//! to write to bot B. B gets the message as a turn nobody trusted (no owner tools, no computer, no memory), both private chats
//! show a peer card, and B's answer is copied to A's chat as a third card.
//!
//! Who may: the owner, in `app:dm:<bot>` and nowhere else, never in a routine's own run, and never in a turn that has already read
//! outside text (a page, a file, a command, a tool, an old chat). A group, a stranger, or text a bot wrote cannot start it. B's
//! turn cannot start it again: it runs without the owner's trust, so the tool is not offered and the app would refuse the call.

use super::*;
use crate::potato::store::PeerCard;

/// The most characters of a message to another bot (the brain cuts at the same number).
const PEER_TEXT_MAX: usize = 600;

impl Service {
    /// The answer to `message_bot`.
    pub(super) fn message_bot_item(&self, ctx: &sandbox_calls::Context, call_id: &str, args: &Value) -> Value {
        let refuse = |code: &str, message: &str| protocol::sandbox_result_err(call_id, code, message, false);
        if ctx.conversation != store::app_conversation(&ctx.bot) {
            return refuse("TOOL_NOT_ALLOWED", "A bot writes to another bot only in the private chat with its owner, not in a group.");
        }
        if !ctx.trust.is_owner() {
            return refuse("TOOL_NOT_ALLOWED", "Only the owner can have this bot write to another bot.");
        }
        let (from_routine, tainted) = {
            let g = self.inner.lock();
            let rt = g.bots.get(&ctx.bot);
            (rt.is_some_and(|rt| rt.turn_routine.contains(&ctx.conversation)), rt.is_some_and(|rt| rt.turn_tainted.contains(&ctx.conversation)))
        };
        if from_routine {
            return refuse("TOOL_NOT_ALLOWED", "A routine cannot write to other bots. Only the owner can ask for that, in a message.");
        }
        if tainted {
            return refuse(
                "TOOL_NOT_ALLOWED",
                "This turn already read text from outside the chat (a page, a file, a command, a tool or an old chat), so it cannot write to another bot. If your owner wants that, they can ask again in a new message.",
            );
        }
        let to = args.get("to").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        let text: String = args.get("text").and_then(Value::as_str).unwrap_or("").trim().chars().take(PEER_TEXT_MAX).collect();
        if to.is_empty() || text.is_empty() {
            return refuse("BAD_ARGS", "Say which bot and what to write.");
        }
        let Ok(bots) = store::load_bots(&self.root) else {
            return refuse("PEER_NOT_SENT", "The bots could not be read. Try again.");
        };
        let named: Vec<(String, String)> = bots.into_iter().filter(|b| b.id != ctx.bot).map(|b| (b.id.clone(), self.name_of(&b.id))).collect();
        let Some((target, target_name)) = named.iter().find(|(_, name)| name.trim().to_lowercase() == to).cloned() else {
            let names: Vec<&str> = named.iter().map(|(_, n)| n.as_str()).collect();
            return refuse("NO_SUCH_BOT", &format!("There is no other bot with that name. Your owner's other bots: {}.", if names.is_empty() { "none".to_string() } else { names.join(", ") }));
        };
        let from_name = self.name_of(&ctx.bot);
        let outcome = match self.send_as(&target, &text, Origin::Peer { from_id: &ctx.bot, from_name: &from_name }) {
            Ok(outcome) => outcome,
            Err(e) => return refuse("PEER_NOT_SENT", &e.message),
        };
        let now = self.host.now_ms();
        let shown = crate::redact::text(&store::clean_text(&text, self.home_of(&ctx.bot).as_deref())).into_inner();
        let mut card = Message::new(now, FromKind::Note, Some("peer"), MsgKind::Note, &shown);
        card.peer = Some(PeerCard { dir: "out".into(), bot_id: target.clone(), bot_name: target_name.clone() });
        if let Err(e) = self.save(&ctx.bot, &ctx.conversation, card) {
            tracing::debug!(%e, "could not save a peer card");
        }
        self.host.changed(&ctx.bot);
        let note = if outcome.delivered {
            format!("Sent to {target_name}. It answers in its own chat and your owner sees the answer here as a card. Do not say it answered.")
        } else {
            format!("{target_name} cannot answer right now, so your message waits in its chat. Tell your owner.")
        };
        protocol::sandbox_result_ok(call_id, serde_json::json!({ "ok": true, "sentTo": target_name, "note": note }))
    }

    /// B answered a message another bot wrote to it: the answer is copied to that bot's chat as a card.
    pub(super) fn relay_peer_reply(&self, bot: &str, in_reply_to: &str, said: &str, now: i64) {
        let Some(asker) = self.inner.lock().bots.get_mut(bot).and_then(|rt| rt.peer_asks.remove(in_reply_to)) else { return };
        if self.require_bot(&asker).is_err() {
            return;
        }
        let shown = crate::redact::text(&store::clean_text(said, self.home_of(bot).as_deref())).into_inner();
        let mut card = Message::new(now, FromKind::Note, Some("peer"), MsgKind::Note, &shown);
        card.peer = Some(PeerCard { dir: "reply".into(), bot_id: bot.to_string(), bot_name: self.name_of(bot) });
        if let Err(e) = self.save(&asker, &store::app_conversation(&asker), card) {
            tracing::debug!(%e, "could not save a peer answer");
            return;
        }
        self.host.changed(&asker);
    }
}
