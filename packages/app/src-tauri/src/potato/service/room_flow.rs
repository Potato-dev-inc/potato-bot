//! Groups in the engine (plan 12, slice G7). The person writes in a group: the line is saved and goes to every member
//! bot's brain as a `room` item. A bot's reply is saved as a room line and goes on to the other bots, until the line
//! is 3 hops deep. The brain decides who speaks (its gate); nothing here runs a model.
//!
//! Rules kept here:
//! - The person's send goes through `rules::decide`, like a send to one bot.
//! - A line never goes back to its sender, never while bots are paused (`hold`), never at the hop limit.
//! - A bot that is off, stopped, blocked or over its day's limit is skipped; the others still get the line.
//! - A brain may answer only a line it was given and has not answered yet (`in_flight`), so a wrong or repeated
//!   answer cannot reset the hop count.
//! - A note (`note:true`) is saved to the room but not passed on. A crash, a stop or an error leaves one note line in
//!   the room, so the room never waits for a bot that is gone.
//! - Usage is charged to the bot that made it (`on_usage`), as for a send.

use super::*;
use crate::potato::rooms::fanout::{self, Skip, MAX_BOT_HOPS, NOTE_HOP};
use crate::potato::rooms::store::{self as rstore, Room, RoomError, RoomFrom, RoomLine, Speaker};

/// The person in every group of version 1: the owner at this PC.
pub const PERSON_ID: &str = "owner";
pub const PERSON_NAME: &str = "You";

/// A room error as a store error: the wire word and the plain sentence.
fn room_err(e: RoomError) -> StoreError {
    let wire = e.to_wire();
    let (code, message) = wire.split_once(": ").unwrap_or(("POTATO_ROOM_IO", "Could not save the group. Try again."));
    let code: &'static str = match code {
        "POTATO_ROOM_NAME" => "POTATO_ROOM_NAME",
        "POTATO_ROOM_BOTS" => "POTATO_ROOM_BOTS",
        "POTATO_ROOM_LIMIT" => "POTATO_ROOM_LIMIT",
        "POTATO_ROOM_GONE" => "POTATO_ROOM_GONE",
        "POTATO_ROOM_ID" => "POTATO_ROOM_ID",
        "POTATO_ROOM_FOLDER" => "POTATO_ROOM_FOLDER",
        "POTATO_ROOM_MODE" => "POTATO_ROOM_MODE",
        _ => "POTATO_ROOM_IO",
    };
    StoreError::new(code, message)
}

fn person() -> RoomFrom {
    RoomFrom { kind: Speaker::Person, id: PERSON_ID.into(), name: PERSON_NAME.into() }
}

impl Service {
    fn find_room(&self, room_id: &str) -> Result<Room> {
        if !store::valid_room_id(room_id) {
            return Err(room_err(RoomError::BadId));
        }
        rstore::load_rooms(&self.root).map_err(room_err)?.into_iter().find(|r| r.id == room_id).ok_or_else(|| room_err(RoomError::NotFound))
    }

    /// The newest `limit` lines of a group, oldest first, without the app's own notes about bots.
    pub fn room_lines(&self, room_id: &str, limit: usize) -> Result<Vec<RoomLine>> {
        self.find_room(room_id)?;
        Ok(rstore::read_lines(&self.root, room_id, limit))
    }

    /// The names of the bots now typing in a group.
    pub fn room_typing(&self, room_id: &str) -> Vec<String> {
        let conversation = store::app_room_conversation(room_id);
        let g = self.inner.lock();
        let mut ids: Vec<&String> = g.bots.iter().filter(|(_, rt)| rt.typing.contains_key(&conversation)).map(|(id, _)| id).collect();
        ids.sort();
        ids.into_iter().map(|id| self.name_of(id)).collect()
    }

    /// The step list of each bot working in a group, by the bot's name.
    pub fn room_work(&self, room_id: &str) -> Vec<(String, super::super::work::WorkRun)> {
        let conversation = store::app_room_conversation(room_id);
        let g = self.inner.lock();
        let mut ids: Vec<&String> = g.bots.keys().collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| g.bots.get(id).and_then(|rt| super::super::work::shown(&rt.work, &rt.typing, &conversation)).map(|w| (self.name_of(id), w)))
            .collect()
    }

    /// Moves a group to the top of the list; a line from a bot also counts as unread.
    fn touch_room(&self, room_id: &str, now: i64, unread: bool) {
        let Ok(mut rooms) = rstore::load_rooms(&self.root) else { return };
        let Some(room) = rooms.iter_mut().find(|r| r.id == room_id) else { return };
        room.last_ms = now;
        if unread {
            room.unread = room.unread.saturating_add(1);
        }
        if let Err(e) = rstore::save_rooms(&self.root, &rooms) {
            tracing::debug!(?e, "could not update a group");
            return;
        }
        self.host.room_changed(room_id);
    }

    /// A bot's name as a group shows it.
    pub fn bot_name(&self, bot: &str) -> String {
        self.name_of(bot)
    }

    /// Tells the windows that a group changed (the group commands save rooms themselves).
    pub fn notify_room(&self, room_id: &str) {
        self.host.room_changed(room_id);
    }

    fn bot_from(&self, bot: &str) -> RoomFrom {
        RoomFrom { kind: Speaker::Bot, id: bot.to_string(), name: self.name_of(bot) }
    }

    /// A note in a group about a bot (it stopped, it is out of budget). Saved, never passed on, never shown to a brain.
    fn room_note(&self, room_id: &str, bot: &str, now: i64, text: &str) {
        let line = RoomLine { id: rstore::new_id('l'), from: self.bot_from(bot), text: text.to_string(), at: now, hop: NOTE_HOP, in_reply_to: None, file: None, handoff: None };
        if let Err(e) = rstore::append_line(&self.root, room_id, line, self.home_of(bot).as_deref()) {
            tracing::debug!(?e, "could not save a group note");
            return;
        }
        self.touch_room(room_id, now, true);
    }

    /// A note row where the conversation is: the bot's own thread, or the group.
    pub(super) fn conversation_note(&self, bot: &str, conversation: &str, now: i64, text: &str) {
        // A Space conversation has no thread: a note about it is dropped.
        if store::app_space_of(conversation).is_some() {
            return;
        }
        match store::app_room_of(conversation) {
            Some(room) => self.room_note(room, bot, now, text),
            None => {
                let _ = self.save(bot, conversation, Message::new(now, FromKind::Note, None, MsgKind::Note, text));
            }
        }
    }

    /// One line from the person in a group. It is saved (hop 0) and goes to every member bot that can take it.
    pub fn send_room(&self, room_id: &str, text: &str) -> Result<RoomLine> {
        let room = self.find_room(room_id)?;
        let now = self.host.now_ms();
        let text = text.trim();
        let held = hold::is_held(&self.root);
        let sends_last_minute = {
            let mut g = self.inner.lock();
            room.bot_ids.iter().map(|b| g.bots.get_mut(b).map(|rt| Self::rate_ok(rt, now)).unwrap_or(0)).max().unwrap_or(0)
        };
        // The day's limit is judged per bot when the line is handed out, so one bot being over never stops the rest.
        let caps = Caps { disabled: !self.brains.enabled(), paused: held, over_limit: false, sends_last_minute };
        let facts = Facts {
            surface: "app",
            sender: Sender::App,
            channel: Channel::NotAChannel,
            mentioned: true,
            replied_to_bot: false,
            from_bot: false,
            age_ms: 0,
            text_is_empty: text.is_empty(),
        };
        let tools = match rules::decide(&facts, &rules::Config::default(), &caps) {
            Decision::Send { tools, .. } => tools,
            Decision::Drop(rules::Why::Disabled) => return Err(err("POTATO_OFF", "Potato Bot is switched off.")),
            Decision::Drop(rules::Why::Empty) => return Err(err("POTATO_EMPTY", "Write a message first.")),
            Decision::Drop(rules::Why::Paused) => return Err(err("POTATO_PAUSED", "Bots are paused. Press Start bots again to resume.")),
            Decision::Drop(rules::Why::RateLimited) => return Err(err("POTATO_RATE", "That is too fast. Wait a moment, then try again.")),
            Decision::Drop(_) | Decision::Pending => return Err(err("POTATO_REFUSED", "That message was not sent.")),
        };
        let line = RoomLine {
            id: rstore::new_id('l'),
            from: person(),
            text: crate::redact::text(&rstore::clean_line(text, None)).into_inner(),
            at: now,
            hop: 0,
            in_reply_to: None,
            file: None,
            handoff: None,
        };
        rstore::append_line(&self.root, room_id, line.clone(), None).map_err(room_err)?;
        self.touch_room(room_id, now, false);
        self.fan_out(&room, &line, tools);
        Ok(line)
    }

    /// The lines before `line`, oldest first, without notes. `inbox_item` keeps the newest 20.
    fn recent_before(&self, room_id: &str, line: &RoomLine) -> Vec<RoomLine> {
        let all = rstore::read_lines(&self.root, room_id, rstore::MAX_READ);
        let upto = all.iter().position(|l| l.id == line.id).unwrap_or(all.len());
        all[..upto].iter().filter(|l| l.hop != NOTE_HOP).cloned().collect()
    }

    /// Hands `line` to the member bots that may take it. Nothing is waited for: a bot that is not running is started
    /// and finds the line in its inbox.
    fn fan_out(&self, room: &Room, line: &RoomLine, tools: Tools) {
        self.fan_out_to(room, line, tools, None);
    }

    /// As `fan_out`, but with `only` set just that bot gets the line (a hand-off goes to the bot it names, not to the whole group).
    fn fan_out_to(&self, room: &Room, line: &RoomLine, tools: Tools, only: Option<&str>) {
        let now = self.host.now_ms();
        let held = hold::is_held(&self.root);
        let conversation = store::app_room_conversation(&room.id);
        let person_line = line.from.kind == Speaker::Person;
        // Cheap exits first: nothing below needs a file when nobody can get the line.
        if held || line.hop >= MAX_BOT_HOPS {
            return;
        }

        // Asked of the disk before the lock is taken (finding Node can take a moment).
        struct Pre {
            exists: bool,
            used: u64,
            limit_cents: Option<u32>,
            agents: bool,
            blocked: Option<Blocked>,
        }
        let day = self.host.local_day(now);
        let Ok(all_settings) = store::load_settings(&self.root) else { return };
        let mut pre: HashMap<String, Pre> = HashMap::new();
        for bot in &room.bot_ids {
            let exists = self.require_bot(bot).is_ok();
            let settings = self.settings_for(bot, &all_settings);
            let used = cost::day_of(&self.root, &day, bot).map(|d| d.cost_nano()).unwrap_or(0);
            let blocked = if exists && self.brains.enabled() && settings.brain != BrainKind::Agents { self.preflight(bot, &settings) } else { None };
            pre.insert(bot.clone(), Pre { exists, used, limit_cents: settings.daily_limit(), agents: settings.brain == BrainKind::Agents, blocked });
        }

        let roster: Vec<RoomFrom> = std::iter::once(person()).chain(room.bot_ids.iter().map(|b| self.bot_from(b))).collect();
        let recent = self.recent_before(&room.id, line);
        let sandbox_only = person_line && line.from.id != PERSON_ID;
        let trust = if sandbox_only { Trust::Others } else { Trust::OwnerPc };
        let sandbox = self.sandbox_offered() && rules::tool_access(trust).sandbox;

        let mut g = self.inner.lock();
        let mut over: HashMap<String, bool> = HashMap::new();
        let mut on: HashMap<String, bool> = HashMap::new();
        for bot in &room.bot_ids {
            let p = &pre[bot];
            let rt = g.bots.get(bot);
            let reserved = rt.map(|r| r.reserved_nano()).unwrap_or(0);
            let estimate = rt.map(|r| r.reserve_estimate()).unwrap_or(cost::RESERVE_FLOOR_NANO);
            let paused_for_limit = rt.is_some_and(|r| r.paused_for_limit.is_some());
            let reserve_over = cost::reserve_over(p.used.saturating_add(reserved).saturating_add(estimate), p.limit_cents);
            over.insert(bot.clone(), cost::budget_with(p.used, p.limit_cents) == Budget::Over || paused_for_limit || reserve_over);
            let stopped = rt.is_some_and(|r| r.keeper.is_stopped());
            on.insert(bot.clone(), p.exists && self.brains.enabled() && !p.agents && p.blocked.is_none() && !stopped);
        }
        let (mut go, skipped) = fanout::targets(room, line, &|b| on.get(b).copied().unwrap_or(false), &|b| over.get(b).copied().unwrap_or(false), held);
        if let Some(only) = only {
            go.retain(|b| b == only);
        }

        for bot in &go {
            let me = self.bot_from(bot);
            let item = fanout::inbox_item(&line.id, room, &me, &roster, &recent, line, tools, sandbox_only);
            let item = protocol::with_capabilities(item, sandbox);
            let rt = g.bots.entry(bot.clone()).or_insert_with(BotRt::new);
            if rt.session.is_none() {
                rt.chain_step = 0;
            }
            rt.blocked = None;
            let estimate = rt.reserve_estimate();
            rt.reserved.push_back(Reservation { id: line.id.clone(), nano: estimate });
            if person_line {
                rt.sends_ms.push_back(now);
            }
            rt.turn_trust.insert(conversation.clone(), trust);
            rt.turn_routine.remove(&conversation);
            self.deliver(rt, &line.id, &conversation, item);
            rt.keeper.want_run(now);
            let actions = rt.keeper.step(now);
            self.run_actions(&mut g, bot, actions);
        }
        drop(g);

        // The person is told why a bot did not get their line. A bot's line that goes unheard needs no note.
        if person_line {
            for (bot, why) in &skipped {
                let name = self.name_of(bot);
                let text = match why {
                    Skip::DailyLimit => {
                        let cents = pre.get(bot).and_then(|p| p.limit_cents).unwrap_or(0);
                        format!("{name} has used today's limit ({}). Raise it in Settings or wait until tomorrow.", cost::dollars(cents))
                    }
                    Skip::BotOff => match pre.get(bot).and_then(|p| p.blocked) {
                        Some(blocked) => self.blocked_note(bot, blocked),
                        None if pre.get(bot).is_some_and(|p| p.agents) && self.agent_brain_id(bot).is_some() => {
                            format!("{name} talks through your own agent, which cannot be limited, so it answers only you in your private chat, not in groups.")
                        }
                        None => format!("{name} is not running, so it did not get this line."),
                    },
                    Skip::Sender | Skip::HopLimit | Skip::Held | Skip::Muted => continue,
                };
                self.room_note(&room.id, bot, now, &text);
            }
        }
        for bot in go.iter().chain(skipped.iter().map(|(b, _)| b)) {
            self.host.changed(bot);
        }
    }

    /// A reply from a member bot. It is saved as a room line one hop deeper than the line it answers, and passed on to
    /// the other bots unless it is a note. Returns whether anything changed.
    pub(super) fn on_room_reply(&self, bot: &str, room_id: &str, in_reply_to: &str, segments: Vec<String>, kaomoji: Option<String>, note: bool, now: i64) -> bool {
        let conversation = store::app_room_conversation(room_id);
        let Ok(room) = self.find_room(room_id) else { return false };
        if !note {
            // The turn is over: the bot lets go of the screen (the person's hold is not touched).
            self.calls.end_run(self.port.lock().as_ref(), bot, &conversation);
        }
        {
            let mut g = self.inner.lock();
            let Some(rt) = g.bots.get_mut(bot) else { return false };
            // Only a line this brain was given and has not answered: a stray or repeated answer is ignored.
            if !rt.session.as_ref().is_some_and(|s| s.in_flight.contains_key(in_reply_to)) {
                return false;
            }
            rt.keeper.touch(now);
            if !note {
                rt.last_error = None;
                rt.typing.remove(&conversation);
                rt.reserved.retain(|r| r.id != in_reply_to);
                if let Some(session) = rt.session.as_mut() {
                    session.in_flight.remove(in_reply_to);
                }
            }
        }
        let trigger_hop = rstore::read_lines(&self.root, room_id, rstore::MAX_READ).into_iter().find(|l| l.id == in_reply_to).map(|l| l.hop);
        // A trigger that is gone from the file cannot be counted: the reply is kept but goes no further.
        let hop = trigger_hop.map(|h| h.saturating_add(1)).unwrap_or(MAX_BOT_HOPS).min(NOTE_HOP - 1);

        let said = crate::companion::view::scrub(&segments.join(" "), None);
        let mut text = segments.join("\n\n");
        if let Some(face) = kaomoji.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            text.push(' ');
            text.push_str(face);
        }
        let line = RoomLine { id: rstore::new_id('l'), from: self.bot_from(bot), text, at: now, hop, in_reply_to: Some(in_reply_to.to_string()), file: None, handoff: None };
        if let Err(e) = rstore::append_line(&self.root, room_id, line.clone(), self.home_of(bot).as_deref()) {
            tracing::debug!(?e, "could not save a group reply");
            return true;
        }
        self.touch_room(room_id, now, true);
        if !note {
            // The saved text is what the others read (secrets and the home folder are already out of it).
            let saved = rstore::read_lines(&self.root, room_id, 1).pop().filter(|l| l.id == line.id).unwrap_or(line);
            self.fan_out(&room, &saved, Tools::Full);
        }
        self.host.reply_pushed(bot, &self.name_of(bot), &said, super::super::notify::pushes_replies(&self.root, bot));
        true
    }

    /// The bot read a line and chose not to answer: the turn is over, nothing is saved.
    pub(super) fn on_room_pass(&self, bot: &str, conversation: &str, in_reply_to: &str, now: i64) -> bool {
        self.calls.end_run(self.port.lock().as_ref(), bot, conversation);
        let mut g = self.inner.lock();
        let Some(rt) = g.bots.get_mut(bot) else { return false };
        rt.keeper.touch(now);
        rt.typing.remove(conversation);
        rt.reserved.retain(|r| r.id != in_reply_to);
        if let Some(session) = rt.session.as_mut() {
            session.in_flight.remove(in_reply_to);
        }
        true
    }
}

/// The most characters of a hand-off task (the brain cuts at the same number).
const HANDOFF_TASK_MAX: usize = 600;

impl Service {
    /// The answer to `hand_off`: one bot gives another bot of the same group a task. Saved as a line with a hand-off card
    /// (who, to whom, what) one hop deeper than the line the bot is answering, and delivered to that bot alone. The hop limit is
    /// the group's: past it the card is saved as "not passed on" and the bot is told to say what is left itself.
    /// The line it answers must be one this bot was given and has not answered yet, so a made-up id cannot reset the count.
    pub(super) fn hand_off_item(&self, ctx: &sandbox_calls::Context, call_id: &str, args: &Value) -> Value {
        let refuse = |code: &str, message: &str| protocol::sandbox_result_err(call_id, code, message, false);
        let Some(room_id) = store::app_room_of(&ctx.conversation) else {
            return refuse("TOOL_NOT_ALLOWED", "Hand-offs work only inside a group.");
        };
        let Ok(room) = self.find_room(room_id) else {
            return refuse("ROOM_GONE", "This group no longer exists.");
        };
        if !room.bot_ids.iter().any(|b| b == &ctx.bot) || !room.bot_listens(&ctx.bot) {
            return refuse("TOOL_NOT_ALLOWED", "This bot is not taking part in this group.");
        }
        let to = args.get("to").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        let task: String = rstore::clean_line(args.get("task").and_then(Value::as_str).unwrap_or("").trim(), None).chars().take(HANDOFF_TASK_MAX).collect();
        let in_reply_to = args.get("inReplyTo").and_then(Value::as_str).unwrap_or("");
        if to.is_empty() || task.trim().is_empty() {
            return refuse("BAD_ARGS", "Say which bot and what it should do.");
        }
        let answering = self.inner.lock().bots.get(&ctx.bot).and_then(|rt| rt.session.as_ref()).is_some_and(|s| s.in_flight.get(in_reply_to).is_some_and(|c| c == &ctx.conversation));
        if !answering {
            return refuse("NOT_ANSWERING", "There is no message in this group for you to hand off from right now.");
        }
        let members: Vec<(String, String)> = room.bot_ids.iter().map(|b| (b.clone(), self.name_of(b))).collect();
        let Some((to_id, to_name)) = members.iter().find(|(id, name)| id != &ctx.bot && name.trim().to_lowercase() == to).cloned() else {
            let others: Vec<&str> = members.iter().filter(|(id, _)| id != &ctx.bot).map(|(_, n)| n.as_str()).collect();
            return refuse("NO_SUCH_BOT", &format!("No other bot with that name is in this group. The others are: {}.", others.join(", ")));
        };
        if !room.bot_listens(&to_id) {
            return refuse("BOT_OFF", &format!("{to_name} is switched off in this group, so it cannot be given a task."));
        }
        let now = self.host.now_ms();
        let trigger_hop = rstore::read_lines(&self.root, room_id, rstore::MAX_READ).into_iter().find(|l| l.id == in_reply_to).map(|l| l.hop);
        // A trigger that is gone from the file cannot be counted: the card is kept but goes no further.
        let hop = trigger_hop.map(|h| h.saturating_add(1)).unwrap_or(MAX_BOT_HOPS).min(NOTE_HOP - 1);
        let passed = hop < MAX_BOT_HOPS;
        let line = RoomLine {
            id: rstore::new_id('l'),
            from: self.bot_from(&ctx.bot),
            text: format!("@{to_name} {task}"),
            at: now,
            hop,
            in_reply_to: Some(in_reply_to.to_string()),
            file: None,
            handoff: Some(rstore::Handoff { to_id: to_id.clone(), to_name: to_name.clone(), task: task.clone(), passed }),
        };
        if rstore::append_line(&self.root, room_id, line.clone(), self.home_of(&ctx.bot).as_deref()).is_err() {
            return refuse("ROOM_SAVE_FAILED", "The hand-off could not be saved. Try again.");
        }
        self.touch_room(room_id, now, true);
        if !passed {
            return refuse("HOP_LIMIT", "This chat has reached its hand-off limit, so nothing was passed on. Tell the person what is left for them or another bot instead.");
        }
        let saved = rstore::read_lines(&self.root, room_id, 1).pop().filter(|l| l.id == line.id).unwrap_or(line);
        self.fan_out_to(&room, &saved, Tools::Full, Some(&to_id));
        protocol::sandbox_result_ok(call_id, serde_json::json!({ "ok": true, "handedTo": to_name, "note": format!("Passed on to {to_name}. The group sees a hand-off card. {to_name} answers in the group; do not say it is done.") }))
    }
}
