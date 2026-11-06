//! A job's file changes in the engine (`file_undo.rs` keeps the old content): the list the job shows, and Restore. The old
//! content is kept by `sandbox_calls` just before the bot's `write_file` or `edit_file`; this is only the owner's side.

use super::*;
use crate::potato::file_undo;

impl Service {
    /// The files the bot's current job changed, with what Restore will do. `None`: the bot has no job.
    pub fn job_files(&self, bot: &str) -> Result<Option<file_undo::View>> {
        self.require_bot(bot)?;
        let Some(job) = jobs::load(&self.root, bot) else { return Ok(None) };
        Ok(Some(file_undo::view(&self.root, bot, &job.id)?))
    }

    /// Restore on one file of a job: puts the old content back in the bot's computer (or removes a file the job made).
    /// The owner's click only; the bot has no tool for it. Needs the bot's computer to be set up.
    pub fn restore_job_file(&self, bot: &str, job_id: &str, file_id: &str) -> Result<file_undo::Entry> {
        self.require_bot(bot)?;
        let port = self.port.lock().clone().ok_or_else(|| err("SANDBOX_NOT_SET_UP", "The bot's computer is not set up yet, so the file cannot be restored."))?;
        let entry = file_undo::restore(&self.root, bot, job_id, file_id, &*port.sandbox)?;
        self.host.changed(bot);
        Ok(entry)
    }
}
