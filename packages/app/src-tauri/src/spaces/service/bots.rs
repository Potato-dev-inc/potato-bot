//! Bots for the service: hand a stored text line to this PC's bots, take their answers, keep the space diary, and the
//! commands the bot panel uses. Everything here is dead until a `BotPort` is set (Potato Bot up and Labs
//! `spaces_enabled` on); without one, no line is handed on, no diary is written and no bot can be added.
use std::sync::Arc;

use super::*;
use super::super::bot_bridge as bb;
use bb::{BotLocal, BotSink, Candidate, Mode, Planned, Refusal, Reply};
use crate::spaces::diary::{self, DiaryHome, JournalLine};
use crate::spaces::leak::PrivateMarks;
use crate::spaces::log::KIND_TEXT;
use crate::spaces::roster::{bot_name_key, bot_name_ok, new_bot_claim, BotChannels, BotRules};

/// What runs this PC's bots and where their vaults are. Set by the Potato side when both are up.
pub trait BotPort: BotSink + DiaryHome {}
impl<T: BotSink + DiaryHome> BotPort for T {}

/// A line handed to a bot that has not been answered yet. The answer must name its id, and what it may do is what
/// is written here, never what the answer says.
pub(super) struct Pending {
    space: SpaceId,
    channel: ChannelId,
    bot: BotId,
    potato: String,
    line: EntryId,
    called_by: Option<MemberKey>,
    at: u64,
}

/// The words the screen switches on for a refused delivery.
fn refusal_code(r: Refusal) -> &'static str {
    match r {
        Refusal::Unavailable => "unavailable",
        Refusal::Limit => "limit",
        Refusal::Busy => "busy",
        Refusal::NotAllowed => "not_allowed",
    }
}

const NOT_YOURS_REMOVE: &str = "Only the owner of the space or the bot's own owner can remove a bot.";
/// A claim waits this long for the space owner's PC (and is forgotten after it).
const CLAIM_WAIT_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const NO_POTATO: &str = "Potato Bot isn't running, so there is no bot to bring in.";

/// The roster's own words for a refused bot op, as one plain sentence.
fn bot_op_error(e: String) -> String {
    match e.as_str() {
        "bot name taken" => "That name is already used here. Names that only look alike count as the same name.".to_string(),
        "too many bots" => "You already have as many bots here as this space allows.".to_string(),
        "bad bot name" => "Use a plain name, with no special characters.".to_string(),
        "members have more bots" => "Some people already have more bots than that. Remove a bot first.".to_string(),
        "bad bot rules" => "Bots per person must be 0 to 10.".to_string(),
        "no such bot" => "That bot is already gone.".to_string(),
        "not a member" => "You are not a member of this space.".to_string(),
        _ => e,
    }
}

impl SpacesService {
    /// Connects (or disconnects, with `None`) whatever runs this PC's bots.
    pub fn set_bot_port(&self, port: Option<Arc<dyn BotPort>>) {
        let connected = port.is_some();
        *lock(&self.bots_port) = port;
        if connected {
            self.prune_diaries();
        } else {
            lock(&self.bot_pending).clear();
            lock(&self.bot_refused).clear();
        }
    }

    fn port(&self) -> Option<Arc<dyn BotPort>> { lock(&self.bots_port).clone() }

    fn new_delivery_id(&self) -> String { format!("d{}", to_hex(&random_bytes::<12>())) }

    // ---- after a text line (or a delete, or a bot's answer) is stored -------------------------------------------

    /// `lines`: stored text and bot lines. `deletes`: targets of stored deletes. Writes the diary for this PC's bots
    /// and hands each text line to the bots that should answer it. The sink is called with no lock held.
    pub(super) fn after_stored(&self, space: &SpaceId, lines: &[EntryId], deletes: &[EntryId]) {
        if self.stopped.load(Ordering::SeqCst) || (lines.is_empty() && deletes.is_empty()) { return; }
        let Some(port) = self.port() else { return };
        let now = self.now();
        let mut planned: Vec<Planned> = Vec::new();
        {
            let spaces = lock(&self.spaces);
            let Some(sp) = spaces.get(space) else { return };
            let me = sp.me.member_key();
            let ctx = bb::Ctx { store: &sp.store, roster: &sp.roster, me: &me, active: space_state(sp) == "active", now };
            for id in lines {
                let Ok(Some(row)) = sp.store.message(id) else { continue };
                self.journal_row(port.as_ref(), sp, &row);
                planned.extend(bb::plan(&ctx, &row, &mut || self.new_delivery_id()));
            }
            for target in deletes {
                if matches!(sp.store.message(target), Ok(Some(row)) if row.deleted_by.is_some()) {
                    self.journal_forget(port.as_ref(), sp, target);
                }
            }
        }
        let mut refusal_changed = false;
        for p in planned {
            match port.deliver(&p.delivery) {
                Ok(()) => {
                    refusal_changed |= lock(&self.bot_refused).remove(&p.delivery.bot).is_some();
                    let line = EntryId::from_hex(&p.delivery.line.id);
                    let mut pend = lock(&self.bot_pending);
                    pend.retain(|_, x| now.saturating_sub(x.at) < bb::PENDING_MS);
                    if let Some(line) = line {
                        pend.insert(
                            p.delivery.id.clone(),
                            Pending {
                                space: *space, channel: p.delivery.channel, bot: p.delivery.bot, potato: p.delivery.bot_local.clone(),
                                line, called_by: p.called_by, at: now,
                            },
                        );
                    }
                }
                Err(why) => {
                    // The bot could not take it (off, paused, over its money limit): the hold on the caps goes back, and the
                    // reason is kept so the screen can say why nobody answered.
                    if let Some(sp) = lock(&self.spaces).get(space) { bb::usage_release(&sp.store, p.usage); }
                    refusal_changed |= lock(&self.bot_refused).insert(p.delivery.bot, refusal_code(why)) != Some(refusal_code(why));
                }
            }
        }
        if refusal_changed { self.changed(); }
    }

    /// One shown row into the diary of each of my bots that may be in its channel and has memory on.
    fn journal_row(&self, port: &dyn BotPort, sp: &OpenSpace, row: &MessageRow) {
        if !(row.kind == KIND_TEXT || row.kind == KIND_BOT) || row.deleted_by.is_some() { return; }
        let me = sp.me.member_key();
        if row.member != me && sp.store.is_blocked(&row.member) { return; }
        let Ok(locals) = bb::local_list(&sp.store) else { return };
        let who = match (row.kind, row.bot) {
            (KIND_BOT, Some(b)) => sp.roster.bots.get(&b).map(|x| x.name.clone()).unwrap_or_else(|| "a bot".to_string()),
            _ => member_name(sp, &row.member),
        };
        let channel = sp.roster.channel(&row.channel).map(|c| c.name.clone()).unwrap_or_default();
        for l in locals {
            let Some(potato) = &l.potato else { continue };
            if !l.memory || l.silenced || l.muted { continue; }
            if sp.roster.bot_owners.get(&l.bot) != Some(&me) || !sp.roster.bot_may_speak(&l.bot, &row.channel) { continue; }
            let Some(vault) = port.vault_of(potato) else { continue };
            let line = JournalLine {
                id: row.id.to_hex(), at: row.created_ms, who: who.clone(), via: diary::VIA_SPACE.to_string(),
                channel: channel.clone(), text: row.text.clone(),
            };
            let _ = diary::append(&vault, &sp.id, &line);
        }
    }

    /// A deleted message leaves every diary of mine (memory on or off: a diary may have been written before).
    fn journal_forget(&self, port: &dyn BotPort, sp: &OpenSpace, target: &EntryId) {
        let Ok(locals) = bb::local_list(&sp.store) else { return };
        for l in locals {
            let Some(vault) = l.potato.as_deref().and_then(|p| port.vault_of(p)) else { continue };
            let _ = diary::remove(&vault, &sp.id, &target.to_hex());
        }
    }

    /// Caps and handled-line marks that can no longer matter.
    pub(super) fn bots_prune(&self, sp: &OpenSpace, now: u64) {
        bb::prune(&sp.store, now);
        // A claim the owner's PC never answered is forgotten after a week.
        for l in bb::local_list(&sp.store).unwrap_or_default() {
            if l.claimed_ms > 0 && !sp.roster.bot_owners.contains_key(&l.bot) && now.saturating_sub(l.claimed_ms) >= CLAIM_WAIT_MS {
                let _ = bb::local_delete(&sp.store, &l.bot);
            }
        }
    }

    /// Old day files go (at start of the port, and when a bot leaves a space).
    pub(super) fn prune_diaries(&self) {
        let Some(port) = self.port() else { return };
        let now = self.now();
        for sp in lock(&self.spaces).values() {
            for l in bb::local_list(&sp.store).unwrap_or_default() {
                if let Some(vault) = l.potato.as_deref().and_then(|p| port.vault_of(p)) { diary::prune(&vault, &sp.id, now); }
            }
            bb::prune(&sp.store, now);
        }
    }

    /// Housekeeping for one space after its roster changed: a bot that left takes old diary files with it.
    pub(super) fn bots_roster_changed(&self, space: &SpaceId) {
        let Some(port) = self.port() else { return };
        let now = self.now();
        if let Some(sp) = lock(&self.spaces).get(space) {
            for l in bb::local_list(&sp.store).unwrap_or_default() {
                if sp.roster.bots.contains_key(&l.bot) { continue; }
                if let Some(vault) = l.potato.as_deref().and_then(|p| port.vault_of(p)) { diary::prune(&vault, &sp.id, now); }
            }
        }
    }

    // ---- the bot's answer ---------------------------------------------------------------------------------------

    /// A bot of mine answered a delivery. Only the delivery's own space and channel can be posted to, whatever the
    /// answer says. Held back (looked private) or empty: nothing is posted. Returns an error text for the caller's log.
    pub fn bot_reply(&self, potato: &str, delivery: &str, conversation: &str, text: &str) -> Result<(), String> {
        let p = {
            let mut pend = lock(&self.bot_pending);
            match pend.get(delivery) {
                None => return Err("no such delivery".into()),
                Some(p) if p.potato != potato || bb::conversation_of(&p.space, &p.channel) != conversation => {
                    return Err("that answer is not for this space".into());
                }
                Some(_) => pend.remove(delivery).ok_or("no such delivery")?,
            }
        };
        let now = self.now();
        {
            let spaces = lock(&self.spaces);
            let sp = spaces.get(&p.space).ok_or("That space is not known.")?;
            if let Ok(Some(l)) = bb::local_get(&sp.store, &p.bot) {
                if l.silenced || l.muted { return Err("that bot is stopped here".into()); }
            }
        }
        match bb::clean_reply(text, &PrivateMarks::for_this_pc()) {
            Reply::Empty => Ok(()),
            Reply::Held(why) => {
                if let Some(sp) = lock(&self.spaces).get(&p.space) { let _ = bb::held_bump(&sp.store, &p.bot, now); }
                Err(format!("held back: {why}"))
            }
            Reply::Post(clean) => {
                self.send_now(&p.space, &p.channel, Body::BotText { text: clean, called_by: p.called_by, hops: 0 }, Some(p.line), Some(p.bot))
                    .map(|_| ())
            }
        }
    }

    /// The turn ended without an answer to post (a pass, a failure): the delivery is forgotten.
    pub fn bot_ended(&self, potato: &str, delivery: &str) {
        let mut pend = lock(&self.bot_pending);
        if pend.get(delivery).is_some_and(|p| p.potato == potato) { pend.remove(delivery); }
    }

    /// Deliveries handed on and not yet answered (tests count them).
    #[cfg(test)]
    pub(crate) fn bot_pending_count(&self) -> usize { lock(&self.bot_pending).len() }

    // ---- what the screen shows ----------------------------------------------------------------------------------

    /// `[{id, name, ownerName, mine, muted}]` and the rules, for `detail`.
    pub(super) fn bots_in_detail(&self, sp: &OpenSpace) -> (Vec<Value>, Value) {
        let me = sp.me.member_key();
        let locals: BTreeMap<BotId, BotLocal> = bb::local_list(&sp.store).unwrap_or_default().into_iter().map(|l| (l.bot, l)).collect();
        let now = self.now();
        let refused = lock(&self.bot_refused).clone();
        let bots = sp.roster.bots.values().map(|b| {
            let l = locals.get(&b.id);
            let mine = b.member == me && l.is_some_and(|l| l.potato.is_some());
            let own = if mine { l } else { None };
            json!({
                "id": b.id.to_hex(), "name": b.name, "ownerName": member_name(sp, &b.member), "mine": mine,
                "muted": l.is_some_and(|l| l.muted),
                // Mine only: what decides whether a call is answered (the screen explains a silent call from these).
                "silenced": own.map(|l| l.silenced), "dayCap": own.map(|l| l.day_cap),
                "usedToday": if mine { bb::used_in_day(&sp.store, &b.id, now) } else { 0 },
                "refused": if mine { refused.get(&b.id).map(|c| json!(c)).unwrap_or(Value::Null) } else { Value::Null },
            })
        });
        (bots.collect(), bot_rules_json(&sp.roster.bot_rules))
    }

    /// The ids of the bots this viewer hid on this PC.
    pub(super) fn muted_bots(&self, sp: &OpenSpace) -> Vec<BotId> {
        bb::local_list(&sp.store).unwrap_or_default().into_iter().filter(|l| l.muted).map(|l| l.bot).collect()
    }

    /// `space_bot_list`.
    pub fn bot_list(&self, space: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let now = self.now();
        let port_up = self.port().is_some();
        let refused = lock(&self.bot_refused).clone();
        self.with_open(&id, |sp| {
            let me = sp.me.member_key();
            let locals: BTreeMap<BotId, BotLocal> = bb::local_list(&sp.store).unwrap_or_default().into_iter().map(|l| (l.bot, l)).collect();
            let mut mine_count = 0;
            let bots: Vec<Value> = sp.roster.bots.values().map(|b| {
                let l = locals.get(&b.id);
                let mine = b.member == me && l.is_some_and(|l| l.potato.is_some());
                if mine { mine_count += 1; }
                let own = if mine { l } else { None };
                json!({
                    "id": b.id.to_hex(), "name": b.name, "ownerName": member_name(sp, &b.member), "ownerMember": b.member.to_hex(),
                    "mine": mine, "muted": l.is_some_and(|l| l.muted),
                    "silenced": own.map(|l| l.silenced), "mode": own.map(|l| l.mode.as_str()), "memory": own.map(|l| l.memory),
                    "dayCap": own.map(|l| l.day_cap), "potato": own.and_then(|l| l.potato.clone()),
                    "usedToday": if mine { bb::used_in_day(&sp.store, &b.id, now) } else { 0 },
                    "heldBackToday": if mine { bb::held_today(&sp.store, &b.id, now) } else { 0 },
                    "refused": if mine { refused.get(&b.id).map(|c| json!(c)).unwrap_or(Value::Null) } else { Value::Null },
                })
            }).collect();
            let blocked = if !port_up { Some(NO_POTATO) } else { None };
            json!({
                "space": id.to_hex(), "isOwner": sp.is_owner(), "rules": bot_rules_json(&sp.roster.bot_rules),
                "mineCount": mine_count, "addBlocked": blocked, "bots": bots, "pendingAdd": pending_names(sp, &locals, now),
            })
        })
        .ok_or_else(|| "That space is not known.".to_string())
    }

    /// `space_bot_candidates`: my Potato bots that can join and are not in this space yet, each with the name the
    /// sheet starts with (`<me>的<bot>`).
    pub fn bot_candidates(&self, space: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let port = self.port().ok_or(NO_POTATO)?;
        let all = port.candidates();
        self.with_open(&id, |sp| {
            let taken: Vec<String> = bb::local_list(&sp.store)
                .unwrap_or_default()
                .into_iter()
                .filter(|l| sp.roster.bots.contains_key(&l.bot) || (l.claimed_ms > 0 && !sp.roster.bot_owners.contains_key(&l.bot)))
                .filter_map(|l| l.potato)
                .collect();
            let mine = member_name(sp, &sp.me.member_key());
            let list: Vec<Value> = all
                .iter()
                .filter(|c: &&Candidate| !taken.contains(&c.id))
                .map(|c| {
                    let suggested: String = format!("{mine}的{}", c.name).chars().take(NAME_MAX_CHARS).collect();
                    json!({ "id": c.id, "name": c.name, "suggestedName": suggested })
                })
                .collect();
            json!({ "space": id.to_hex(), "candidates": list })
        })
        .ok_or_else(|| "That space is not known.".to_string())
    }

    // ---- changes ------------------------------------------------------------------------------------------------

    /// `space_bot_add`: my member key signs "this bot is mine". On the space owner's PC the claim is published at once in an
    /// `AddBot` op. On a member's PC it goes out as a hidden `BotClaim` entry; the owner's PC publishes the `AddBot` when it
    /// stores the entry (also later, if it was offline), and the bot shows in `pendingAdd` until the roster lists it.
    pub fn bot_add(&self, space: &str, potato: &str, name: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let port = self.port().ok_or(NO_POTATO)?;
        if !port.candidates().iter().any(|c| c.id == potato) { return Err("That bot can't join a space.".into()); }
        let name = clean_name(name.trim());
        if !bot_name_ok(&name) { return Err("Give the bot a plain name.".into()); }
        let now = self.now();
        let bot = BotId::random();
        let (op, claim, channel) = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            if space_state(sp) != "active" { return Err("You can't change this space any more.".into()); }
            let me = sp.me.member_key();
            let locals = bb::local_list(&sp.store).unwrap_or_default();
            let fresh = |l: &BotLocal| l.claimed_ms > 0 && !sp.roster.bot_owners.contains_key(&l.bot) && now.saturating_sub(l.claimed_ms) < CLAIM_WAIT_MS;
            let already = locals.iter().any(|l| l.potato.as_deref() == Some(potato) && (sp.roster.bots.contains_key(&l.bot) || fresh(l)));
            if already { return Err("That bot is already in this space.".into()); }
            // The checks the roster will make, so a refusal is told at once and not left to the owner's PC.
            let key = bot_name_key(&name);
            if key.is_empty() || sp.roster.bots.values().any(|b| bot_name_key(&b.name) == key) || locals.iter().any(|l| fresh(l) && bot_name_key(&l.claim_name) == key) {
                return Err(bot_op_error("bot name taken".into()));
            }
            let pending = locals.iter().filter(|l| fresh(l)).count();
            if (sp.roster.bots_of(&me).len() + pending) as u32 >= sp.roster.bot_rules.per_person {
                return Err(bot_op_error("too many bots".into()));
            }
            let body = new_bot_claim(&sp.me, &id, bot, &name);
            if sp.is_owner() {
                let op = owner_op(sp, body, now).map_err(bot_op_error)?;
                bb::local_put(&sp.store, &BotLocal::new(bot, Some(potato.to_string()))).map_err(db)?;
                (Some(op), None, sp.roster.channels[0].id)
            } else {
                let OpBody::AddBot { sig, .. } = body else { return Err("Could not sign the claim.".into()) };
                (None, Some(sig), sp.roster.channels[0].id)
            }
        };
        if let Some(sig) = claim {
            self.send_now(&id, &channel, Body::BotClaim { bot, name: name.clone(), sig }, None, None)?;
            if let Some(sp) = lock(&self.spaces).get(&id) {
                let mut l = BotLocal::new(bot, Some(potato.to_string()));
                l.claimed_ms = now;
                l.claim_name = name;
                bb::local_put(&sp.store, &l).map_err(db)?;
            }
        }
        if let Some(op) = op { self.push_control(&id, &op); }
        self.changed();
        self.bot_list(space)
    }

    /// `space_bot_remove`: the space owner takes any bot off the roster; a member takes their own off with a hidden `BotDrop`
    /// entry (it stops here at once). A claim that is still pending is simply forgotten.
    pub fn bot_remove(&self, space: &str, bot: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let bot = BotId::from_hex(bot).ok_or("That bot is not here.")?;
        let now = self.now();
        enum Next { Op(ControlOp), Drop(ChannelId), Done }
        let next = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            let me = sp.me.member_key();
            let local = bb::local_get(&sp.store, &bot).map_err(db)?;
            match sp.roster.bots.get(&bot).map(|b| b.member) {
                Some(_) if sp.is_owner() => {
                    let op = owner_op(sp, OpBody::RemoveBot { bot }, now).map_err(bot_op_error)?;
                    let _ = bb::local_delete(&sp.store, &bot);
                    Next::Op(op)
                }
                Some(owner) if owner == me && local.as_ref().is_some_and(|l| l.potato.is_some()) => {
                    let mut l = local.unwrap_or_else(|| BotLocal::new(bot, None));
                    l.silenced = true;
                    bb::local_put(&sp.store, &l).map_err(db)?;
                    Next::Drop(sp.roster.channels[0].id)
                }
                Some(_) => return Err(NOT_YOURS_REMOVE.into()),
                None if local.as_ref().is_some_and(|l| l.claimed_ms > 0) => {
                    let _ = bb::local_delete(&sp.store, &bot);
                    Next::Done
                }
                None => return Err("That bot is already gone.".into()),
            }
        };
        match next {
            Next::Op(op) => self.push_control(&id, &op),
            Next::Drop(ch) => { self.send_now(&id, &ch, Body::BotDrop { bot }, None, None)?; }
            Next::Done => {}
        }
        self.bots_roster_changed(&id);
        self.messages_changed(&id, None);
        self.changed();
        self.bot_list(space)
    }

    /// The owner's PC acts on bot claims and drops it has just stored. Each goes through the roster's own rules (the number
    /// of bots, the name check, the claim's signature against the author's key); a claim the rules refuse only drops. A
    /// claim from a member who is gone, one older than a week, one for an id that was ever on the roster, and a drop of a
    /// bot that is not the author's are all dropped.
    pub(crate) fn owner_bot_asks(&self, space: &SpaceId, asks: Vec<(DeviceKey, u64, Body)>) {
        let now = self.now();
        let mut ops = Vec::new();
        {
            let mut spaces = lock(&self.spaces);
            let Some(sp) = spaces.get_mut(space) else { return };
            if !sp.is_owner() || space_state(sp) != "active" { return; }
            for (device, at, body) in asks {
                let Some((member, None)) = sp.roster.author(&device) else { continue };
                if member == sp.me.member_key() || now.saturating_sub(at) > CLAIM_WAIT_MS { continue; }
                let op_body = match body {
                    Body::BotClaim { bot, name, sig } if !sp.roster.bot_owners.contains_key(&bot) => OpBody::AddBot { bot, member, name, sig },
                    Body::BotDrop { bot } if sp.roster.bots.get(&bot).map(|b| b.member) == Some(member) => OpBody::RemoveBot { bot },
                    _ => continue,
                };
                if let Ok(op) = owner_op(sp, op_body, now) { ops.push(op); }
            }
        }
        if ops.is_empty() { return; }
        for op in &ops { self.push_control(space, op); }
        self.bots_roster_changed(space);
        self.messages_changed(space, None);
        self.changed();
    }

    /// `space_bot_rules`: the space owner sets bots per person (0 to 10) and the channels bots may speak in
    /// (`None`: all of them).
    pub fn bot_rules(&self, space: &str, per_person: u32, channels: Option<Vec<String>>) -> Result<Value, String> {
        let id = space_of(space)?;
        let now = self.now();
        let channels = match channels {
            None => BotChannels::All,
            Some(list) => {
                let mut ids: Vec<ChannelId> = list.iter().map(|c| channel_of(c)).collect::<Result<_, _>>()?;
                ids.sort();
                ids.dedup();
                BotChannels::Only(ids)
            }
        };
        let op = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(&id).ok_or("That space is not known.")?;
            if !sp.is_owner() { return Err("Only the owner can change the bot rules.".into()); }
            if let BotChannels::Only(list) = &channels {
                if list.iter().any(|c| sp.roster.channel(c).is_none()) { return Err("That channel is not known.".into()); }
            }
            owner_op(sp, OpBody::SetBotRules { rules: BotRules { per_person, channels } }, now).map_err(bot_op_error)?
        };
        self.push_control(&id, &op);
        self.changed();
        self.bot_list(space)
    }

    /// `space_bot_local`: this PC's own settings. `mode`, `memory`, `day_cap` and `silenced` are for a bot of mine;
    /// `muted` hides any bot's rows on this PC.
    pub fn bot_local(&self, space: &str, bot: &str, patch: &BotPatch) -> Result<Value, String> {
        let id = space_of(space)?;
        let bot = BotId::from_hex(bot).ok_or("That bot is not here.")?;
        let mode = patch.mode.as_deref().map(|m| Mode::parse(m).ok_or("Pick Only when called or When it helps.")).transpose()?;
        self.with_open(&id, |sp| -> Result<(), String> {
            if !sp.roster.bots.contains_key(&bot) { return Err("That bot is not here.".into()); }
            let me = sp.me.member_key();
            let mut l = bb::local_get(&sp.store, &bot).map_err(db)?.unwrap_or_else(|| BotLocal::new(bot, None));
            let mine = l.potato.is_some() && sp.roster.bot_owners.get(&bot) == Some(&me);
            if (mode.is_some() || patch.memory.is_some() || patch.day_cap.is_some() || patch.silenced.is_some()) && !mine {
                return Err("Only a bot's owner can change how it works.".into());
            }
            if let Some(m) = mode { l.mode = m; }
            if let Some(m) = patch.memory { l.memory = m; }
            if let Some(n) = patch.day_cap { l.day_cap = bb::clamp_day_cap(n); }
            if let Some(s) = patch.silenced { l.silenced = s; }
            if let Some(m) = patch.muted { l.muted = m; }
            bb::local_put(&sp.store, &l).map_err(db)
        })
        .ok_or("That space is not known.")??;
        self.messages_changed(&id, None);
        self.changed();
        self.bot_list(space)
    }

    /// Wipes the diaries of a space this PC forgets, for every bot of mine.
    pub(super) fn wipe_diaries(&self, sp: &OpenSpace) {
        let Some(port) = self.port() else { return };
        for l in bb::local_list(&sp.store).unwrap_or_default() {
            if let Some(vault) = l.potato.as_deref().and_then(|p| port.vault_of(p)) { diary::wipe(&vault, &sp.id); }
        }
    }
}

/// What `space_bot_local` may change. A field left out is left alone.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BotPatch {
    pub mode: Option<String>,
    pub memory: Option<bool>,
    pub day_cap: Option<u32>,
    pub silenced: Option<bool>,
    pub muted: Option<bool>,
}

/// Names of this PC's claims that the roster does not show yet (less than a week old).
fn pending_names(sp: &OpenSpace, locals: &BTreeMap<BotId, BotLocal>, now: u64) -> Vec<String> {
    let mut v: Vec<&BotLocal> = locals
        .values()
        .filter(|l| l.potato.is_some() && l.claimed_ms > 0 && !sp.roster.bot_owners.contains_key(&l.bot) && now.saturating_sub(l.claimed_ms) < CLAIM_WAIT_MS)
        .collect();
    v.sort_by_key(|l| l.claimed_ms);
    v.into_iter().map(|l| l.claim_name.clone()).collect()
}

fn bot_rules_json(r: &BotRules) -> Value {
    json!({
        "perPerson": r.per_person,
        "channels": match &r.channels { BotChannels::All => Value::Null, BotChannels::Only(list) => json!(list.iter().map(|c| c.to_hex()).collect::<Vec<_>>()) },
    })
}

#[cfg(test)]
impl SpacesService {
    /// Test only: the space owner's PC publishes a claim that `claimant` signed (there is no transport yet that
    /// carries a member's claim to the owner, so a test plays it by hand).
    pub(crate) fn test_publish_claim(&self, space: &SpaceId, claimant: &crate::spaces::identity::SpaceIdentity, name: &str) -> BotId {
        let bot = BotId::random();
        let now = self.now();
        let op = {
            let mut spaces = lock(&self.spaces);
            let sp = spaces.get_mut(space).expect("space");
            let body = new_bot_claim(claimant, space, bot, name);
            owner_op(sp, body, now).expect("claim accepted")
        };
        self.push_control(space, &op);
        bot
    }
}

#[cfg(test)]
impl SpacesService {
    pub(crate) fn now_for_test(&self) -> u64 { self.now() }
}
