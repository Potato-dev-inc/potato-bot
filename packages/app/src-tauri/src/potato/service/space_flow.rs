//! Bots in Spaces, Potato's side. A line from a Space reaches this bot's brain as a `space` item; the brain's answer goes back
//! to the Space through `SpacePort`. Nothing of a space conversation is ever written to a thread:
//!
//! - no `save`, no `reply_pushed`, no phone, no `open_work`, no group file. A mid-turn note is dropped.
//! - the turn's trust is set here from the signature the Space checked (`owner_turn`), never from anything the brain says:
//!   `OwnerPc` for the owner's own line, `Others` for anyone else's. `Others` has no computer and no helpers.
//! - the same checks as a group line before the brain is asked: switched off, paused, stopped, no key, over the day's limit.
//!   A bot on "my own agents" is refused (an agent cannot be limited), as in groups.
//! - one kind of turn at a time per channel: a guest turn and an owner turn never run side by side in one conversation, because the
//!   brain names the conversation, not the turn, when it asks for a tool.

use super::*;
use crate::spaces::bot_bridge::{Candidate, Delivery, Line, Refusal};

/// Where a bot's answer goes. The Spaces side implements it; it decides what is posted.
pub trait SpacePort: Send + Sync {
    /// The bot answered delivery `delivery` of `conversation` with `text`.
    fn reply(&self, bot: &str, delivery: &str, conversation: &str, text: &str);
    /// The turn is over without an answer to post (a pass, a failure).
    fn ended(&self, bot: &str, delivery: &str);
}

fn line_json(l: &Line) -> Value {
    serde_json::json!({
        "id": l.id,
        "from": { "kind": l.from_kind, "id": l.from_id, "name": l.from_name },
        "text": crate::redact::text(&l.text).into_inner(),
        "at": l.at,
        "hop": 0,
        "inReplyTo": l.in_reply_to,
    })
}

impl Service {
    pub fn set_space_port(&self, port: Option<Arc<dyn SpacePort>>) {
        *self.space_port.lock() = port;
    }

    /// The bots of this PC that may be brought into a space: not the ones that talk through the person's own agents.
    pub fn space_candidates(&self) -> Vec<Candidate> {
        let Ok(settings) = store::load_settings(&self.root) else { return Vec::new() };
        let Ok(bots) = store::load_bots(&self.root) else { return Vec::new() };
        bots.into_iter()
            .filter(|b| self.settings_for(&b.id, &settings).brain != BrainKind::Agents)
            .map(|b| Candidate { name: self.name_of(&b.id), id: b.id })
            .collect()
    }

    /// The folder of a bot's memory (the Space diary is written below it), when the bot exists.
    pub fn space_vault(&self, bot: &str) -> Option<PathBuf> {
        self.require_bot(bot).ok()?;
        store::vault_dir(&self.root, bot).ok()
    }

    /// A conversation the brain may post into: a space channel with a delivery this bot was given and has not answered.
    pub(super) fn own_space_turn(&self, bot: &str, conversation: &str) -> bool {
        store::app_space_of(conversation).is_some()
            && self.inner.lock().bots.get(bot).and_then(|rt| rt.session.as_ref()).is_some_and(|s| s.in_flight.values().any(|c| c == conversation))
    }

    /// One line from a Space for one bot of this PC. Checked like a group line; nothing is saved anywhere.
    pub fn space_deliver(&self, d: &Delivery) -> std::result::Result<(), Refusal> {
        let bot = d.bot_local.as_str();
        self.require_bot(bot).map_err(|_| Refusal::Unavailable)?;
        let conversation = store::app_space_conversation(&d.space.to_hex(), &d.channel.to_hex());
        if store::app_space_of(&conversation).is_none() {
            return Err(Refusal::Unavailable);
        }
        let now = self.host.now_ms();
        let settings = self.settings_for(bot, &store::load_settings(&self.root).map_err(|_| Refusal::Unavailable)?);
        if settings.brain == BrainKind::Agents {
            return Err(Refusal::NotAllowed);
        }
        let held = hold::is_held(&self.root);
        let day = self.host.local_day(now);
        let used = cost::day_of(&self.root, &day, bot).map_err(|_| Refusal::Unavailable)?.cost_nano();
        let blocked = if self.brains.enabled() { self.preflight(bot, &settings) } else { None };
        // The trust is the signature's verdict, carried by the Space; the computer is offered to the owner's turn only.
        let trust = if d.owner_turn { Trust::OwnerPc } else { Trust::Others };
        let sandbox = self.sandbox_offered() && rules::tool_access(trust).sandbox;

        let mut g = self.inner.lock();
        let rt = g.bots.entry(bot.to_string()).or_insert_with(BotRt::new);
        // One kind of turn at a time in a channel (see the top of this file).
        let waiting = rt.queued.iter().any(|i| i.get("conversationId").and_then(Value::as_str) == Some(conversation.as_str()));
        let running = rt.session.as_ref().is_some_and(|s| s.in_flight.values().any(|c| *c == conversation));
        if (waiting || running) && rt.turn_trust.get(&conversation).copied() != Some(trust) {
            return Err(Refusal::Busy);
        }
        let caps = Caps { disabled: !self.brains.enabled(), paused: held, over_limit: false, sends_last_minute: Self::rate_ok(rt, now) };
        let facts = Facts {
            surface: "app",
            sender: Sender::App,
            channel: Channel::NotAChannel,
            mentioned: true,
            replied_to_bot: false,
            from_bot: false,
            age_ms: 0,
            text_is_empty: d.line.text.trim().is_empty(),
        };
        match rules::decide(&facts, &rules::Config::default(), &caps) {
            Decision::Send { .. } => {}
            Decision::Drop(rules::Why::RateLimited) => return Err(Refusal::Busy),
            Decision::Drop(_) | Decision::Pending => return Err(Refusal::Unavailable),
        }
        let estimate = rt.reserve_estimate();
        let reserved = rt.reserved_nano();
        if cost::budget_with(used, settings.daily_limit()) == Budget::Over
            || rt.paused_for_limit.is_some()
            || cost::reserve_over(used.saturating_add(reserved).saturating_add(estimate), settings.daily_limit())
        {
            return Err(Refusal::Limit);
        }
        if blocked.is_some() || rt.keeper.is_stopped() {
            return Err(Refusal::Unavailable);
        }

        let item = serde_json::json!({
            "v": 1,
            "type": "space",
            "id": d.id,
            "conversationId": conversation,
            "channelId": d.channel.to_hex(),
            "spaceId": d.space.to_hex(),
            "spaceName": d.space_name,
            "channelName": d.channel_name,
            "me": { "kind": "bot", "id": bot, "name": d.bot_name },
            "line": line_json(&d.line),
            "recent": d.recent.iter().map(line_json).collect::<Vec<_>>(),
            "turn": if d.owner_turn { "owner" } else { "guest" },
            "memory": d.memory,
            "capabilities": { "sandbox": sandbox },
        });

        if rt.session.is_none() {
            rt.chain_step = 0;
        }
        rt.blocked = None;
        rt.reserved.push_back(Reservation { id: d.id.clone(), nano: estimate });
        rt.sends_ms.push_back(now);
        if !running {
            rt.turn_tainted.remove(&conversation);
        }
        rt.turn_trust.insert(conversation.clone(), trust);
        rt.space_lines.insert(d.line.id.clone(), d.id.clone());
        rt.turn_routine.remove(&conversation);
        self.deliver(rt, &d.id, &conversation, item);
        rt.keeper.want_run(now);
        let actions = rt.keeper.step(now);
        self.run_actions(&mut g, bot, actions);
        drop(g);
        self.host.changed(bot);
        Ok(())
    }

    /// The delivery a brain event answers. The brain names the line it answers (`inReplyTo` = the line's entry id, as the
    /// wire says); the engine tracks deliveries by their own id. An id that is no known line is taken as a delivery id.
    fn space_delivery_of(&self, bot: &str, in_reply_to: &str) -> String {
        self.inner.lock().bots.get(bot).and_then(|rt| rt.space_lines.get(in_reply_to).cloned()).unwrap_or_else(|| in_reply_to.to_string())
    }

    /// The end of a space turn on the engine's side: the brain is no longer owed an answer for it, and the turn's trust goes
    /// when the last turn of that channel is over (a late tool call then finds no trust, which is `Others`).
    fn end_space_turn(&self, bot: &str, conversation: &str, delivery: &str, now: i64) -> bool {
        self.calls.end_run(self.port.lock().as_ref(), bot, conversation);
        let mut g = self.inner.lock();
        let Some(rt) = g.bots.get_mut(bot) else { return false };
        let owed = rt.session.as_ref().is_some_and(|s| s.in_flight.get(delivery).is_some_and(|c| c == conversation));
        if !owed {
            return false;
        }
        rt.space_lines.retain(|_, d| d != delivery);
        rt.keeper.touch(now);
        rt.last_error = None;
        rt.typing.remove(conversation);
        rt.work.remove(conversation);
        rt.reserved.retain(|r| r.id != delivery);
        if let Some(session) = rt.session.as_mut() {
            session.in_flight.remove(delivery);
            session.send_index.remove(delivery);
            if !session.in_flight.values().any(|c| c == conversation) {
                rt.turn_trust.remove(conversation);
                rt.turn_tainted.remove(conversation);
            }
        }
        true
    }

    /// The brain's answer to a space line. A mid-turn note is dropped; the final answer goes to the Space and nowhere else.
    pub(super) fn on_space_reply(&self, bot: &str, conversation: &str, in_reply_to: &str, segments: Vec<String>, kaomoji: Option<String>, note: bool, now: i64) -> bool {
        if note {
            return false;
        }
        let in_reply_to = self.space_delivery_of(bot, in_reply_to);
        let in_reply_to = in_reply_to.as_str();
        if !self.end_space_turn(bot, conversation, in_reply_to, now) {
            return false;
        }
        let mut text = segments.join("\n\n");
        if let Some(face) = kaomoji.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            text.push(' ');
            text.push_str(face);
        }
        if let Some(port) = self.space_port.lock().clone() {
            port.reply(bot, in_reply_to, conversation, &text);
        }
        true
    }

    /// The brain read the line and chose not to answer: nothing is posted.
    pub(super) fn on_space_pass(&self, bot: &str, conversation: &str, in_reply_to: &str, now: i64) -> bool {
        let in_reply_to = self.space_delivery_of(bot, in_reply_to);
        let in_reply_to = in_reply_to.as_str();
        let ended = self.end_space_turn(bot, conversation, in_reply_to, now);
        if ended {
            if let Some(port) = self.space_port.lock().clone() {
                port.ended(bot, in_reply_to);
            }
        }
        ended
    }

    /// The turn failed. Another brain of the bot's chain may try again; otherwise the turn is over and nothing is said anywhere.
    pub(super) fn on_space_error(&self, bot: &str, code: ErrorCode, conversation: &str, in_reply_to: Option<&str>, steps: u64, now: i64) -> bool {
        let delivery = in_reply_to.map(|l| self.space_delivery_of(bot, l));
        let in_reply_to = delivery.as_deref();
        if !job_stopped(code, steps) && self.fall_back(bot, now, code, in_reply_to) {
            return true;
        }
        if code == ErrorCode::ProviderAuth || code == ErrorCode::ProviderLimit {
            if let Some(rt) = self.inner.lock().bots.get_mut(bot) {
                rt.last_error = Some(code);
            }
        }
        let Some(id) = in_reply_to else { return false };
        let ended = self.end_space_turn(bot, conversation, id, now);
        if ended {
            if let Some(port) = self.space_port.lock().clone() {
                port.ended(bot, id);
            }
        }
        ended
    }

    /// What a space turn may ask of the computer. `None`: the normal path judges it. A guest turn gets no tool at all. An owner
    /// turn gets the computer's tools and nothing that writes into a chat, a job, a card or the bot's own settings.
    pub(super) fn space_tool_refusal(&self, ctx: &sandbox_calls::Context, call_id: &str, tool: &str) -> Option<Value> {
        let refuse = |message: &str| Some(protocol::sandbox_result_err(call_id, "TOOL_NOT_ALLOWED", message, false));
        if !ctx.trust.is_owner() {
            return refuse("Only the bot's owner can use tools, and this line is not from the owner.");
        }
        let computer = sandbox_calls::Tool::parse(tool).is_some() && !matches!(tool, "send_file" | "ask_person_for_screen");
        if computer {
            return None;
        }
        refuse("Do this in your private chat with me.")
    }
}
