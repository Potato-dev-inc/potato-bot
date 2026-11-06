//! The diagnostic file ("Save a diagnostic file" in Settings > General).
//!
//! A zip of: a plain README, app and system versions, the settings with secrets removed, the tail of
//! the app log, and the newest frozen flight recordings of failed runs. Every file is made of
//! [`Redacted`] text (strict mode: secrets, long opaque runs and the user's folder name), so the
//! zip writer cannot be handed raw text. Nothing is uploaded: the file goes where the person saves it.
//!
//! `preview` lists what a save would contain; the person can leave any file out (`exclude`).


