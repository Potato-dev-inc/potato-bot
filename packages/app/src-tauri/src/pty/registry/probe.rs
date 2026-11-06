//! A throwaway PTY for the canary (`crate::canary`): the CLI is started exactly like a pane (same
//! command line, same environment, same `vt100` emulator) but is never registered, never shown and
//! never announced to any window. The caller drives it and kills it.

