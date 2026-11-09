//! My name in Spaces: one per PC, kept in `settings.json`, used when I create or join a space and sent to every
//! space I am already in as a signed rename (`SetName`).
use super::*;
use super::bots::CLAIM_WAIT_MS;
use crate::spaces::roster::new_name_claim;

impl SpacesService {
    /// The name this PC uses in every space, or None until the person picks one.
    pub fn display_name(&self) -> Option<String> { lock(&self.display_name).clone() }

    /// Create and join need a name first: the screen asks for it when it hears `needs_display_name`.
    pub(super) fn require_display_name(&self) -> Result<String, String> {
        self.display_name().ok_or_else(|| NEEDS_NAME.to_string())
    }

    /// `spaces_set_display_name`: clean the name, keep it, and rename me in every space I am in. In a space I own the
    /// rename is published at once; in another it goes to the owner's PC as a hidden \`NameClaim\` and shows up when
    /// the owner's PC publishes it.
    pub fn set_display_name(&self, raw: &str) -> Result<String, String> {
        let name = clean_name(raw);
        if name == "Someone" { return Err("Type the name people should see.".into()); }
        presence::write_setting(&self.dir, "displayName", json!(name))?;
        *lock(&self.display_name) = Some(name.clone());
        self.rename_me_everywhere(&name);
        self.changed();
        Ok(name)
    }

    fn rename_me_everywhere(&self, name: &str) {
        let now = self.now();
        let mut ops: Vec<(SpaceId, ControlOp)> = Vec::new();
        let mut claims: Vec<(SpaceId, ChannelId, Body)> = Vec::new();
        {
            let mut spaces = lock(&self.spaces);
            for (id, sp) in spaces.iter_mut() {
                if space_state(sp) != "active" { continue; }
                let me = sp.me.member_key();
                if sp.roster.members.get(&me).map(|m| m.name.as_str()) == Some(name) { continue; }
                let body = new_name_claim(&sp.me, id, name);
                if sp.is_owner() {
                    if let Ok(op) = owner_op(sp, body, now) { ops.push((*id, op)); }
                } else if let OpBody::SetName { sig, .. } = body {
                    claims.push((*id, sp.roster.channels[0].id, Body::NameClaim { name: name.to_string(), sig }));
                }
            }
        }
        for (id, op) in &ops {
            self.push_control(id, op);
            self.messages_changed(id, None);
        }
        for (id, ch, body) in claims { let _ = self.send_now(&id, &ch, body, None, None); }
    }

    /// The owner's PC publishes the renames members asked for. The roster checks the member's own signature.
    pub(crate) fn owner_name_asks(&self, space: &SpaceId, mut asks: Vec<(DeviceKey, u64, String, [u8; 64])>) {
        let now = self.now();
        asks.sort_by_key(|a| a.1);
        let mut ops = Vec::new();
        {
            let mut spaces = lock(&self.spaces);
            let Some(sp) = spaces.get_mut(space) else { return };
            if !sp.is_owner() || space_state(sp) != "active" { return; }
            for (device, at, name, sig) in asks {
                let Some((member, None)) = sp.roster.author(&device) else { continue };
                if member == sp.me.member_key() || now.saturating_sub(at) > CLAIM_WAIT_MS { continue; }
                if sp.roster.members.get(&member).is_some_and(|m| m.name == name) { continue; }
                if let Ok(op) = owner_op(sp, OpBody::SetName { member, name, sig }, now) { ops.push(op); }
            }
        }
        if ops.is_empty() { return; }
        for op in &ops { self.push_control(space, op); }
        self.messages_changed(space, None);
        self.changed();
    }
}
