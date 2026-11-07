//! Every Spaces limit in one place. Changing a number here is a product decision.
//!
//! Bot caps are named `BOT_*` (not tied to one bot product): per the later public/private change, a cap is per
//! bot, counted on and paid by that bot's owner's computer. The BOT_* caps are enforced by the bot side (the
//! owner's PC before it answers); the log does not count them when history is received, because every PC must
//! reach the same verdict on the same bytes.

pub const MAX_MEMBERS: usize = 50;
pub const MAX_CHANNELS: usize = 20;
pub const MAX_CHANNELS_EVER: usize = 100;
pub const KEEP_MS: u64 = 90 * 24 * 60 * 60 * 1000;
pub const KEEP_BYTES: u64 = 200 * 1024 * 1024;
pub const TRIM_TO_BYTES: u64 = 180 * 1024 * 1024;
/// No one author may hold more than this in a space. Over it, their own oldest entries go first, whatever
/// timestamps they put on them. Member count is capped at 50, so 60 MB each cannot all be reached together.
pub const AUTHOR_KEEP_BYTES: u64 = 60 * 1024 * 1024;
pub const INVITE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
pub const INVITE_MAX_STRIKES: u32 = 5;
/// After a wrong try on an invite, the next try on that invite is refused (Busy) for this long, whoever sends it.
/// Counted by invite, not by node: fresh node keys do not get around it, and five wrong tries need 2 minutes.
pub const INVITE_TRY_GAP_MS: u64 = 30_000;
/// A stranger's first frame (Hello or JoinRequest) may not be bigger than this.
pub const FIRST_FRAME_MAX_BYTES: usize = 16 * 1024;
pub const MAX_OPEN_INVITES: usize = 3;
pub const JOIN_LOCKOUT_STRIKES: u32 = 3;
pub const JOIN_LOCKOUT_MS: u64 = 600_000;
pub const JOIN_WAIT_MS: u64 = 30 * 60 * 1000;
pub const JOINS_PER_MINUTE: u32 = 20;
pub const NAME_MAX_CHARS: usize = 40;
pub const CHANNEL_NAME_MAX_CHARS: usize = 32;
pub const MAX_TEXT_CHARS: usize = 4000;
pub const MAX_ENTRY_BYTES: usize = 65_536;
pub const MAX_FRAME_BYTES: usize = 1_048_576;
pub const MAX_LINKS: usize = 12;
pub const RATE_BURST: u32 = 10;
pub const RATE_BURST_WINDOW_MS: u64 = 10_000;
pub const RATE_HOUR: u32 = 600;
pub const MAX_WANT_PER_ROUND: usize = 500;
pub const FILE_MAX_BYTES: u64 = 25 * 1024 * 1024;
pub const FILE_CHUNK: usize = 65_536;
/// Answers one bot may give per day (its owner can raise it).
pub const BOT_DAY_CAP: u32 = 30;
pub const BOT_PERSON_HOUR_CAP: u32 = 5;
pub const BOT_UNPROMPTED_CAP: u32 = 4;
pub const BOT_UNPROMPTED_WINDOW_MS: u64 = 30 * 60 * 1000;
/// Bots each person may bring into a private space: the owner sets 0 to 10, new spaces start at 1.
pub const BOTS_PER_PERSON_MAX: u32 = 10;
pub const BOTS_PER_PERSON_DEFAULT: u32 = 1;
