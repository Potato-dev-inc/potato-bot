//! A refused typed write is never silent: the caller gets the refusal (the PWA shows a plain line)
//! and the app log gets ONE line per pane and channel.
//!
//! Live (VERIFY-R11): a loopback browser tab (no pairing token, no MCP session) typing into a pane a
//! `run_agent` run manages is refused 403 `AUTHORIZATION_DENIED` ("managed pane mutations require an
//! MCP session id", `mcp_sessions/authorize.rs`), and nothing on either side said so. A paired phone is
//! exempt (`router.rs`, `ws.rs`), so this only concerns the desktop's own browser tab, a dev server,
//! demos and filming. A held key repeats the refusal on every byte, hence once per pane and channel.

