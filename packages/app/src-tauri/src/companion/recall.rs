//! `companion_recall`: the companion finds what its person said before.
//!
//! The tool is an MCP tool of the Puppet Master pack, so a companion's agent calls it like any other. The
//! bridge route (`bridge/routes/companion_mcp.rs`) lets it through only for a caller that runs inside a
//! companion's own pane (`caller::Verdict::verified`) and then asks this file, with that pane id.
//!
//! What it searches: the chat search index (`chat_index`), only inside the pane ids saved in the companion's
//! profile (`CompanionProfile::chat_panes`: its pane now and past ones), and only what was said (the person's
//! and the companion's own messages; never tool output). It never searches another agent's chats and never
//! another companion's. Snippets were redacted when they were indexed; they are redacted again here and the
//! home folder is taken out. When the person turned the index off, the answer is one plain sentence.
//!
//! `remember` is not here on purpose: it already works, by one line in `memory/pending.md` that the person
//! reviews (`memory.rs`). There is no second way to write a memory.


use super::service::{Service};

impl Service {
}
