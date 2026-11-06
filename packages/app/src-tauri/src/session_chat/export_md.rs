//! "Save as Markdown": the same chat as `export` makes, as one Markdown file. Same pipeline as the page
//! (`export_text`): every text is redacted whole, the home folder becomes `~`, folder paths shrink to their last name,
//! and only then is the text shaped as Markdown. Tool calls are one line, an edit is a ```diff block, tool output is
//! at most `MAX_LINES` lines. Markdown has no markup escaping, so raw HTML is neutralised here: outside code a `<`
//! becomes `&lt;` (no tag can start), and an image opener `![` is written `!&#91;` (no image is loaded).


