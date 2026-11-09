//! Presence for the service: who is online, away or offline. Kept in memory only, never stored.
use std::path::Path;
use std::sync::atomic::Ordering;

use super::*;
use crate::spaces::presence::{self, Presence, PresenceTable};

const SETTINGS_FILE: &str = "settings.json";

/// "Show when I'm online" (all spaces on this computer). Default on.
pub(super) fn load_show_online(dir: &Path) -> bool {
    std::fs::read(dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["showOnline"].as_bool())
        .unwrap_or(true)
}

fn read_settings(dir: &Path) -> Value {
    std::fs::read(dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Writes one key of `settings.json` and keeps the others.
pub(super) fn write_setting(dir: &Path, key: &str, value: Value) -> Result<(), String> {
    let fail = || "Could not save that setting.".to_string();
    std::fs::create_dir_all(dir).map_err(|_| fail())?;
    let mut all = read_settings(dir);
    all[key] = value;
    std::fs::write(dir.join(SETTINGS_FILE), all.to_string()).map_err(|_| fail())
}

/// This PC's name in Spaces, if one was set.
pub(super) fn load_display_name(dir: &Path) -> Option<String> {
    read_settings(dir)["displayName"].as_str().map(str::to_string).filter(|n| !n.is_empty())
}

impl SpacesService {
    /// Report my state ("online" or "away"). Peers hear it at once.
    pub fn set_my_presence(&self, state: &str) -> Result<(), String> {
        let p = Presence::from_name(state).ok_or("That presence is not known.")?;
        *lock(&self.my_presence) = p;
        self.presence_tick();
        Ok(())
    }

    /// Turn "Show when I'm online" on or off and keep it in `settings.json`. Off: nothing about me is sent.
    pub fn set_show_online(&self, on: bool) -> Result<(), String> {
        write_setting(&self.dir, "showOnline", json!(on))?;
        self.show_online.store(on, Ordering::SeqCst);
        if on { self.presence_tick(); }
        self.changed();
        Ok(())
    }

    pub fn show_online(&self) -> bool { self.show_online.load(Ordering::SeqCst) }

    /// Tell every live link of every space my state and what I heard (fresh entries). Hidden: send nothing.
    pub fn presence_tick(&self) {
        if !self.show_online() { return; }
        let now = self.now();
        let mine = *lock(&self.my_presence);
        let spaces: Vec<(SpaceId, MemberKey, Vec<MemberKey>)> = lock(&self.spaces)
            .iter()
            .map(|(id, sp)| (*id, sp.me.member_key(), sp.roster.members.keys().copied().collect()))
            .collect();
        for (space, me, members) in spaces {
            let relay = {
                let mut tables = lock(&self.presence);
                let t = tables.entry(space).or_default();
                t.retain(now, |m| members.contains(m));
                t.relay_list(now).into_iter().filter(|(m, ..)| *m != me).collect::<Vec<_>>()
            };
            let body = presence::encode(mine, &relay);
            lock(&self.links).send_to_space(&space, None, sync::PRESENCE, &body);
        }
    }

    /// A Presence frame arrived on a sync stream from `peer`. Entries about people outside the space are ignored.
    pub(crate) fn on_presence(&self, space: &SpaceId, peer: NodeKey, body: &[u8]) {
        let Ok((theirs, relay)) = presence::decode(body) else { return };
        let now = self.now();
        let Some((sender, members, me)) = self.with_open(space, |sp| {
            (
                sp.roster.device_by_node(&peer).map(|(m, _)| m.member),
                sp.roster.members.keys().copied().collect::<Vec<_>>(),
                sp.me.member_key(),
            )
        }) else { return };
        let Some(sender) = sender else { return };
        {
            let mut tables = lock(&self.presence);
            let t = tables.entry(*space).or_insert_with(PresenceTable::new);
            t.heard(sender, theirs, now, now);
            for (m, s, at) in relay {
                if m != me && m != sender && members.contains(&m) { t.heard(m, s, at, now); }
            }
        }
        self.changed();
    }

    /// `online`, `away` or `offline` for one member of one space.
    pub(crate) fn presence_of(&self, space: &SpaceId, member: &MemberKey, now: u64) -> &'static str {
        lock(&self.presence).get(space).map(|t| t.state_of(member, now)).unwrap_or(Presence::Offline).name()
    }

    pub(super) async fn presence_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(presence::SEND_EVERY_MS));
        loop {
            tick.tick().await;
            self.presence_tick();
        }
    }
}
