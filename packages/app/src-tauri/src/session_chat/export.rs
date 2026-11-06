//! "Save chat as page": one self-contained HTML file of a pane's chat, to keep or send. The page has inline CSS
//! and no script, and links to nothing (no `href`, `src`, font or image). Every string goes through `redact` first,
//! folder paths shrink to the folder name and text is escaped (`export_text`). Generic: every agent's items are
//! drawn the same way; only the agent's display name is shown.


