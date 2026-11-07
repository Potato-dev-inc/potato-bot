//! Abuse limits. Entry limits use the author's own timestamps so every PC decides the same way.
//! Nothing here looks at an address: the key is the author's device (entries) or the stream (frames).
use super::ids::DeviceKey;
use super::limits::{RATE_BURST, RATE_BURST_WINDOW_MS, RATE_HOUR};
use super::store::SpaceStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over { No, Burst, Hour }

const HOUR_MS: u64 = 60 * 60 * 1000;

/// Counts this author's stored entries in (created_ms - window, created_ms]. Fish (kind 7), reactions, deletes and
/// Leave count too, and so do entries whose text was dropped later (`gone`): deleting does not buy more sends.
pub fn author_over_limit(s: &SpaceStore, device: &DeviceKey, created_ms: u64) -> rusqlite::Result<Over> {
    let count = |window: u64| -> rusqlite::Result<u32> {
        s.conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE device = ?1 AND created_ms > ?2 AND created_ms <= ?3",
            rusqlite::params![&device.0[..], created_ms.saturating_sub(window) as i64, created_ms as i64],
            |r| r.get::<_, u32>(0),
        )
    };
    if count(RATE_BURST_WINDOW_MS)? >= RATE_BURST { return Ok(Over::Burst); }
    if count(HOUR_MS)? >= RATE_HOUR { return Ok(Over::Hour); }
    Ok(Over::No)
}

pub const LINK_FRAMES_PER_SEC: u32 = 200;
pub const LINK_OVER_SECONDS: u32 = 3;

/// Frames per second on one stream. Three seconds in a row over the cap = close.
pub struct LinkMeter { second: u64, count: u32, over_in_row: u32 }

#[derive(Debug, PartialEq, Eq)]
pub enum Meter { Ok, Close }

impl LinkMeter {
    pub fn new() -> Self { Self { second: 0, count: 0, over_in_row: 0 } }

    pub fn frame(&mut self, now_ms: u64) -> Meter {
        let s = now_ms / 1000;
        if s != self.second {
            if self.count > LINK_FRAMES_PER_SEC { self.over_in_row += 1; } else { self.over_in_row = 0; }
            self.second = s;
            self.count = 0;
        }
        self.count += 1;
        if self.over_in_row >= LINK_OVER_SECONDS { Meter::Close } else { Meter::Ok }
    }
}

impl Default for LinkMeter {
    fn default() -> Self { Self::new() }
}
