//! A bot's skills in the app (the bot panel's Skills tab): the written ways of working its owner taught it. The brain
//! keeps them as one Markdown file each in `<notes>/skills/<slug>.md` (`potato-brain/src/persona/skills.ts` is the
//! writer and the reader of the brain's side; the layout here is the same: a `# name` line, a `Description:` line, a blank
//! line, then the steps). The brain reads the folder on every owner turn, so a change here is seen at once.
//!
//! The page can list, read (the list carries the text), edit and delete. It cannot make a skill (the bot does, when its
//! owner asks) and it cannot rename one: the name is the file. Edit keeps the brain's limits (4096 characters a file, a
//! one-line description of 200). Delete moves the file to `skills/.removed/`; nothing is erased.
//!
//! Every file name comes from the list and is checked again: letters, digits and dashes only, no part of a path.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::store::{self, StoreError, StoreResult};

pub const SKILL_CAP: usize = 4096;
pub const DESCRIPTION_MAX: usize = 200;
pub const NAME_MAX: usize = 60;
const SLUG_MAX: usize = 40;
const LIST_LIMIT: usize = 100;
const REMOVED_DIR: &str = ".removed";
const NOT_FOUND: &str = "POTATO_SKILL_NOT_FOUND";
const INVALID: &str = "POTATO_SKILL_INVALID";

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// The file name without `.md`.
    pub slug: String,
    pub name: String,
    pub description: String,
    pub body: String,
}

fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::new(INVALID, message)
}

/// `<state>/notes/skills`, where the brain keeps them.
pub fn dir(root: &Path, bot: &str) -> StoreResult<PathBuf> {
    store::require_bot(root, bot)?;
    Ok(store::state_dir(root, bot)?.join("notes").join("skills"))
}

fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.chars().count() <= SLUG_MAX && slug.chars().all(|c| c.is_alphanumeric() || c == '-')
}

fn one_line(raw: &str, max: usize) -> String {
    let flat: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect::<String>().trim().to_string()
}

/// The file text, as the brain writes it.
fn render(name: &str, description: &str, body: &str) -> String {
    format!("# {name}\nDescription: {description}\n\n{body}\n")
}

fn parse(slug: &str, text: &str) -> Option<Skill> {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.split('\n');
    let head = lines.next()?;
    let name_raw = head.strip_prefix("# ")?;
    let name = one_line(name_raw, NAME_MAX);
    let rest: Vec<&str> = lines.collect();
    let (description, skip) = match rest.first().and_then(|l| l.strip_prefix("Description:")) {
        Some(d) => (d.trim().to_string(), 2),
        None => (String::new(), 1),
    };
    let body = rest.iter().skip(skip).copied().collect::<Vec<_>>().join("\n").trim().to_string();
    Some(Skill { slug: slug.to_string(), name: if name.is_empty() { slug.to_string() } else { name }, description, body })
}

fn read_one(folder: &Path, slug: &str) -> Option<Skill> {
    parse(slug, &fs::read_to_string(folder.join(format!("{slug}.md"))).ok()?)
}

/// Every skill of the bot, by name.
pub fn list(root: &Path, bot: &str) -> StoreResult<Vec<Skill>> {
    let folder = dir(root, bot)?;
    let Ok(entries) = fs::read_dir(&folder) else { return Ok(Vec::new()) };
    let mut slugs: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".md").map(str::to_string))
        .filter(|s| valid_slug(s))
        .collect();
    slugs.sort();
    slugs.truncate(LIST_LIMIT);
    Ok(slugs.iter().filter_map(|s| read_one(&folder, s)).collect())
}

/// Changes the description and the steps of one skill. The name, and so the file, stays.
pub fn save(root: &Path, bot: &str, slug: &str, description: &str, body: &str) -> StoreResult<Skill> {
    let _files = store::files_lock();
    let folder = dir(root, bot)?;
    if !valid_slug(slug) {
        return Err(StoreError::new(NOT_FOUND, "That skill is gone."));
    }
    let current = read_one(&folder, slug).ok_or_else(|| StoreError::new(NOT_FOUND, "That skill is gone."))?;
    let description = one_line(description, DESCRIPTION_MAX);
    let body = body.replace("\r\n", "\n").trim().to_string();
    if body.is_empty() {
        return Err(invalid("A skill needs some steps. Nothing was saved."));
    }
    let text = render(&current.name, &description, &body);
    if text.chars().count() > SKILL_CAP {
        return Err(invalid(format!("A skill can have at most {SKILL_CAP} characters. Nothing was saved.")));
    }
    crate::secure_file::write_private(&folder.join(format!("{slug}.md")), &text).map_err(|e| StoreError::new("POTATO_STORE_INVALID", e))?;
    Ok(Skill { slug: current.slug, name: current.name, description, body })
}

/// Moves one skill's file to `skills/.removed/` (a taken name there gets `-2`, `-3`, ...).
pub fn delete(root: &Path, bot: &str, slug: &str) -> StoreResult<Skill> {
    let _files = store::files_lock();
    let folder = dir(root, bot)?;
    if !valid_slug(slug) {
        return Err(StoreError::new(NOT_FOUND, "That skill is gone."));
    }
    let current = read_one(&folder, slug).ok_or_else(|| StoreError::new(NOT_FOUND, "That skill is gone."))?;
    let removed = folder.join(REMOVED_DIR);
    fs::create_dir_all(&removed).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not make the removed folder: {e}.")))?;
    let mut to = removed.join(format!("{slug}.md"));
    let mut n = 2;
    while to.exists() {
        to = removed.join(format!("{slug}-{n}.md"));
        n += 1;
    }
    fs::rename(folder.join(format!("{slug}.md")), &to).map_err(|e| StoreError::new("POTATO_STORE_INVALID", format!("Could not remove the skill: {e}.")))?;
    Ok(current)
}
