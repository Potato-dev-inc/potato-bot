//! "Remember for all agents": one line added to the project memory from a selection in a chat.
//!
//! The selection is cleaned to one line (control characters out, whitespace collapsed), refused when it is
//! empty or holds a secret, capped at [`MAX_CHARS`], and added as `- <text>` unless the memory already has
//! that line (case-insensitive). [`forget_with`] removes that same line again (the toast's Undo).
//!
//! The error text starts with a stable code (`CODE: message`), the same shape the other commands use, so the
//! UI can explain each case.



