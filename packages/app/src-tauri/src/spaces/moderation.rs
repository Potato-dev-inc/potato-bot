//! Moderation helpers. Pure: build the ops; the service applies, stores and pushes them.
use super::identity::SpaceIdentity;
use super::ids::MemberKey;
use super::roster::{ControlOp, OpBody, RemoveReason, Roster, RosterError};

/// RemoveMember + Rotate, ready to apply in this order. `new_key` is a fresh random 32-byte key.
/// The grants of the rotation are computed on the roster AFTER the removal, so the removed person (and their
/// bots, which the roster drops with them) get no copy of the new key.
pub fn remove_ops(roster: &Roster, owner: &SpaceIdentity, member: &MemberKey, reason: RemoveReason,
                  new_key: &[u8; 32], now_ms: u64) -> Result<[ControlOp; 2], RosterError> {
    let remove = roster.next_op(owner, OpBody::RemoveMember { member: *member, reason }, now_ms)?;
    let mut after = roster.clone();
    after.apply(&remove)?;
    let grants = after.grants_for_all(after.epoch + 1, new_key)?;
    let rotate = after.next_op(owner, OpBody::Rotate { epoch: after.epoch + 1, grants }, now_ms)?;
    Ok([remove, rotate])
}
