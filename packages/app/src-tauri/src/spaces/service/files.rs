//! Files for the service: offer a file, get one on request, save a copy. Nothing is fetched until a person asks.
use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::spaces::files::{self, FileOffer, Fetch};

/// What the UI shows for a file that is being fetched, or whose last fetch failed.
#[derive(Clone, Copy, Debug)]
pub(super) struct FileJob { pub percent: u8, pub failed: bool }

pub(super) type FileJobs = BTreeMap<(SpaceId, [u8; 32]), FileJob>;

const NO_ONE: &str = "No one online has this file right now. Try again later.";
const BROKEN: &str = "This file didn't check out and was thrown away.";

/// `SpaceMessage.file` for a message row, or Null.
pub(super) fn file_value(space_dir: &Path, sp: &OpenSpace, row: &MessageRow, jobs: &FileJobs) -> Value {
    let Some(raw) = row.file_json.as_deref().and_then(files::hex_bytes) else { return Value::Null };
    let Ok(offer) = FileOffer::from_body(&raw) else { return Value::Null };
    let cached = files::cache_path(space_dir, &offer.sha).is_file();
    let have_row: bool = sp
        .store
        .conn
        .query_row("SELECT have FROM files WHERE sha = ?1", params![offer.sha.to_vec()], |r| r.get::<_, i64>(0))
        .map(|h| h == 1)
        .unwrap_or(false);
    let have = cached && have_row;
    let job = jobs.get(&(sp.id, offer.sha));
    let state = match job {
        Some(j) if j.failed => "failed",
        Some(_) => "getting",
        None => "idle",
    };
    json!({
        "sha": to_hex(&offer.sha), "name": offer.name, "size": offer.size, "have": have, "state": state,
        "percent": job.map(|j| j.percent).unwrap_or(0),
    })
}

impl SpacesService {
    fn space_dir(&self, id: &SpaceId) -> std::path::PathBuf { self.dir.join(id.to_hex()) }

    /// Send a local file to a channel. The bytes are cached here and served to members who ask.
    pub async fn offer_file(&self, space: &str, channel: &str, path: &str) -> Result<Value, String> {
        let id = space_of(space)?;
        let ch = channel_of(channel)?;
        self.with_open(&id, |_| ()).ok_or("That space is not known.")?;
        let (offer, data) = files::offer_from_path(Path::new(path))?;
        let cache = files::cache_path(&self.space_dir(&id), &offer.sha);
        let existed = cache.is_file();
        if !existed {
            let dir = cache.parent().ok_or("Can't read that file.")?;
            tokio::fs::create_dir_all(dir).await.map_err(|_| "Can't save that file.".to_string())?;
            let part = dir.join(format!("{}.part", to_hex(&offer.sha)));
            tokio::fs::write(&part, &data).await.map_err(|_| "Can't save that file.".to_string())?;
            tokio::fs::rename(&part, &cache).await.map_err(|_| "Can't save that file.".to_string())?;
        }
        let entry = match self.send(&id, &ch, Body::File(offer.to_body()), None).await {
            Ok(e) => e,
            Err(e) => {
                if !existed { let _ = tokio::fs::remove_file(&cache).await; }
                return Err(e);
            }
        };
        let now = self.now() as i64;
        self.with_open(&id, |sp| {
            let _ = sp.store.conn.execute(
                "UPDATE files SET have = 1, cached_ms = ?2, size = ?3 WHERE sha = ?1",
                params![offer.sha.to_vec(), now, offer.size as i64],
            );
        });
        self.messages_changed(&id, Some(&ch));
        self.message_value(&id, entry)
    }

    /// Fetch a file from the author, then from any other live member link. Checked chunk by chunk.
    pub async fn get_file(&self, space: &str, sha: &str) -> Result<(), String> {
        let id = space_of(space)?;
        let sha: [u8; 32] = from_hex::<32>(sha).ok_or("That file is not known.")?;
        struct Plan { offer: FileOffer, have: bool, author: Option<NodeKey>, members: Vec<NodeKey>, me: NodeKey }
        let plan = self
            .with_open(&id, |sp| -> Result<Plan, String> {
                if space_state(sp) != "active" { return Err("You can't get files in this space any more.".into()); }
                let (have, offer_id): (i64, Vec<u8>) = sp
                    .store
                    .conn
                    .query_row("SELECT have, offer FROM files WHERE sha = ?1", params![sha.to_vec()], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(|_| "That file is not known.".to_string())?;
                let offer_id = EntryId(<[u8; 32]>::try_from(offer_id.as_slice()).map_err(|_| "That file is not known.".to_string())?);
                let row = sp.store.message(&offer_id).ok().flatten().ok_or("That file is not known.")?;
                if row.deleted_by.is_some() { return Err("That file is gone.".into()); }
                let offer = row
                    .file_json
                    .as_deref()
                    .and_then(files::hex_bytes)
                    .and_then(|b| FileOffer::from_body(&b).ok())
                    .ok_or("That file is not known.")?;
                let author = sp.roster.device(&row.device).map(|(_, c)| c.node);
                Ok(Plan {
                    offer,
                    have: have == 1 && files::cache_path(&self.space_dir(&id), &sha).is_file(),
                    author,
                    members: sp.roster.member_nodes(),
                    me: sp.me.node(),
                })
            })
            .ok_or("That space is not known.")??;
        if plan.have { return Ok(()); }
        let _permit = self
            .downloads
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Too many files are coming in. Try again in a moment.".to_string())?;
        {
            let mut jobs = lock(&self.file_jobs);
            if matches!(jobs.get(&(id, sha)), Some(j) if !j.failed) { return Err("Already getting this file.".into()); }
            jobs.insert((id, sha), FileJob { percent: 0, failed: false });
        }
        self.messages_changed(&id, None);

        let live = lock(&self.links).peers();
        let mut order: Vec<NodeKey> = Vec::new();
        if let Some(a) = plan.author { order.push(a); }
        for n in live {
            if plan.members.contains(&n) && !order.contains(&n) { order.push(n); }
        }
        order.retain(|n| *n != plan.me);

        let last_percent = std::sync::atomic::AtomicU8::new(0);
        let progress = |done: u16, total: u16| {
            let pct = (done as u32 * 100 / total.max(1) as u32) as u8;
            if last_percent.swap(pct, Ordering::SeqCst) != pct {
                if let Some(j) = lock(&self.file_jobs).get_mut(&(id, sha)) { j.percent = pct; }
                self.messages_changed(&id, None);
            }
        };
        let dir = self.space_dir(&id);
        let (mut done, mut broken) = (false, false);
        for node in order {
            let Some(link) = lock(&self.links).link(&node) else { continue };
            let opened = tokio::time::timeout(files::IO_WAIT, link.open()).await;
            let Ok(Ok(mut stream)) = opened else { continue };
            match files::fetch_file(&mut stream, &id, &plan.offer, &dir, &progress).await {
                Fetch::Done => { done = true; break; }
                Fetch::Broken => broken = true,
                Fetch::None => {}
            }
            last_percent.store(0, Ordering::SeqCst);
        }
        if done {
            let now = self.now() as i64;
            self.with_open(&id, |sp| {
                let _ = sp.store.conn.execute(
                    "UPDATE files SET have = 1, cached_ms = ?2, size = ?3 WHERE sha = ?1",
                    params![sha.to_vec(), now, plan.offer.size as i64],
                );
            });
            lock(&self.file_jobs).remove(&(id, sha));
            self.messages_changed(&id, None);
            return Ok(());
        }
        if let Some(j) = lock(&self.file_jobs).get_mut(&(id, sha)) { j.failed = true; j.percent = 0; }
        self.messages_changed(&id, None);
        Err(if broken { BROKEN } else { NO_ONE }.to_string())
    }

    /// Copy a cached file into `downloads` under its cleaned name. Returns `{ "name": final name }`.
    pub fn save_file(&self, space: &str, sha: &str, downloads: &Path) -> Result<Value, String> {
        let id = space_of(space)?;
        let sha: [u8; 32] = from_hex::<32>(sha).ok_or("That file is not known.")?;
        let name = self
            .with_open(&id, |sp| {
                sp.store
                    .conn
                    .query_row("SELECT name FROM files WHERE sha = ?1 AND have = 1", params![sha.to_vec()], |r| r.get::<_, String>(0))
                    .ok()
            })
            .ok_or("That space is not known.")?
            .ok_or("That file is not here yet.")?;
        let cached = files::cache_path(&self.space_dir(&id), &sha);
        if !cached.is_file() { return Err("That file is not here any more.".into()); }
        let saved = files::save_copy(&cached, downloads, &name)?;
        Ok(json!({ "name": saved }))
    }

    /// Server side of a `FileWant` stream from `peer`: only for a current member of the space, and only a file that
    /// one of its live offers names. At most `MAX_UPLOADS` at a time; the rest get `FileNone`.
    pub(super) async fn serve_file_stream(self: Arc<Self>, mut stream: SpaceStream, space: SpaceId, sha: [u8; 32], first: u16) {
        let Ok(_permit) = self.uploads.clone().try_acquire_owned() else {
            let _ = write_frame(&mut stream.send, files::FILE_NONE, &[]).await;
            return;
        };
        let listed = self
            .with_open(&space, |sp| {
                sp.store
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM files WHERE sha = ?1 AND offer IN (SELECT id FROM entries WHERE gone = 0)",
                        params![sha.to_vec()],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(0)
                    > 0
            })
            .unwrap_or(false);
        let dir = self.space_dir(&space);
        let _ = files::serve_file(&mut stream, &dir, listed, &sha, first).await;
        let _ = stream.send.shutdown().await;
    }
}
