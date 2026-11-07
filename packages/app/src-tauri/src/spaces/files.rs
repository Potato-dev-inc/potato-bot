//! Files in Spaces: the offer lives in the log, the bytes move only when someone clicks `Get file`, and every
//! chunk is checked. Nothing here opens or runs a file. Names a peer sent are never used as a path.
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::frame::{read_frame, write_frame};
use super::ids::{to_hex, SpaceId};
use super::limits::{FILE_CHUNK, FILE_MAX_BYTES};
use super::net::SpaceStream;
use super::wire::{Reader, WireError, Writer};

pub const FILE_WANT: u8 = 0x20;
pub const FILE_CHUNK_FRAME: u8 = 0x21;
pub const FILE_NONE: u8 = 0x22;
pub const FILE_DONE: u8 = 0x23;
pub const MAX_UPLOADS: usize = 2;
pub const MAX_DOWNLOADS: usize = 4;
pub const NAME_MAX: usize = 120;
pub const MIME_MAX: usize = 64;
pub const TOO_BIG: &str = "Files can be up to 25 MB.";
/// A silent peer frees its slot after this long without a frame.
pub const IO_WAIT: Duration = Duration::from_secs(30);

/// Device names Windows will not create as plain files.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOffer { pub name: String, pub size: u64, pub sha: [u8; 32], pub chunks: Vec<[u8; 32]>, pub mime: String }

pub fn chunk_count(size: u64) -> u64 { size.div_ceil(FILE_CHUNK as u64) }

impl FileOffer {
    pub fn to_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.str(&self.name).u64(self.size).fixed(&self.sha).u32(FILE_CHUNK as u32).u16(self.chunks.len() as u16);
        for c in &self.chunks { w.fixed(c); }
        w.str(&self.mime);
        w.finish()
    }

    /// Strict: size 1..=25 MB, chunk size 65 536, the chunk count that size needs, a name that is already clean.
    pub fn from_body(b: &[u8]) -> Result<Self, &'static str> {
        let wire = |e: WireError| e.0;
        let mut r = Reader::new(b);
        let name = r.str(NAME_MAX * 4).map_err(wire)?;
        let size = r.u64().map_err(wire)?;
        let sha = r.fixed::<32>().map_err(wire)?;
        let chunk = r.u32().map_err(wire)?;
        let n = r.u16().map_err(wire)? as u64;
        if size == 0 || size > FILE_MAX_BYTES { return Err("bad file size"); }
        if chunk as usize != FILE_CHUNK { return Err("bad chunk size"); }
        if n != chunk_count(size) { return Err("bad chunk count"); }
        let mut chunks = Vec::with_capacity(n as usize);
        for _ in 0..n { chunks.push(r.fixed::<32>().map_err(wire)?); }
        let mime = r.str(MIME_MAX).map_err(wire)?;
        r.done().map_err(wire)?;
        if name != clean_file_name(&name) { return Err("unclean file name"); }
        Ok(Self { name, size, sha, chunks, mime })
    }

    /// Length of chunk `i` (the last one is the remainder).
    pub fn chunk_len(&self, i: usize) -> usize {
        let start = i as u64 * FILE_CHUNK as u64;
        (self.size - start).min(FILE_CHUNK as u64) as usize
    }
}

fn is_bad_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        // Bidi controls, zero-width and BOM: they make `gpj.exe` look like `exe.jpg`.
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Strip folders (both slash kinds), control and bidi chars, characters Windows forbids, leading dots and trailing
/// dots/spaces; cap 120 chars; a Windows device name gets `_` in front; empty -> "file".
pub fn clean_file_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let kept: String = last.chars().filter(|c| !is_bad_char(*c)).collect();
    let kept = kept.trim_start_matches(['.', ' ']);
    let mut name: String = kept.chars().take(NAME_MAX).collect();
    while name.ends_with(['.', ' ']) { name.pop(); }
    if name.is_empty() { return "file".into(); }
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    if RESERVED.contains(&stem.as_str()) { name.insert(0, '_'); }
    name
}

pub fn hash_chunks(data: &[u8]) -> ([u8; 32], Vec<[u8; 32]>) {
    let whole: [u8; 32] = Sha256::digest(data).into();
    (whole, data.chunks(FILE_CHUNK).map(|c| Sha256::digest(c).into()).collect())
}

/// Build the offer for a local file (<= 25 MB, a regular file). The size is checked before the bytes are read.
pub fn offer_from_path(path: &Path) -> Result<(FileOffer, Vec<u8>), String> {
    let meta = std::fs::metadata(path).map_err(|_| "Can't read that file.".to_string())?;
    if !meta.is_file() { return Err("Can't read that file.".into()); }
    if meta.len() > FILE_MAX_BYTES { return Err(TOO_BIG.into()); }
    let data = std::fs::read(path).map_err(|_| "Can't read that file.".to_string())?;
    if data.len() as u64 > FILE_MAX_BYTES { return Err(TOO_BIG.into()); }
    if data.is_empty() { return Err("That file is empty.".into()); }
    let (sha, chunks) = hash_chunks(&data);
    let name = clean_file_name(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    Ok((FileOffer { name, size: data.len() as u64, sha, chunks, mime: "application/octet-stream".into() }, data))
}

/// `space_dir/files/<hex>`. Only a hash makes the name, never anything a peer typed.
pub fn cache_path(space_dir: &Path, sha: &[u8; 32]) -> PathBuf { space_dir.join("files").join(to_hex(sha)) }

fn part_path(cache: &Path) -> PathBuf {
    let mut p = cache.as_os_str().to_owned();
    p.push(".part");
    PathBuf::from(p)
}

pub fn want_body(space: &SpaceId, sha: &[u8; 32], first: u16) -> Vec<u8> {
    let mut w = Writer::new();
    w.fixed(&space.0).fixed(sha).u16(first);
    w.finish()
}

pub fn parse_want(body: &[u8]) -> Result<(SpaceId, [u8; 32], u16), WireError> {
    let mut r = Reader::new(body);
    let space = SpaceId(r.fixed::<16>()?);
    let sha = r.fixed::<32>()?;
    let first = r.u16()?;
    r.done()?;
    Ok((space, sha, first))
}

async fn none(s: &mut SpaceStream) -> Result<(), ()> {
    write_frame(&mut s.send, FILE_NONE, &[]).await.map_err(|_| ())
}

/// Server side: send the cached file from chunk `first`, then FileDone. FileNone if it is not cached here or is
/// not a file of a size the rules allow. The length comes from the cached file itself.
pub async fn serve_file(s: &mut SpaceStream, space_dir: &Path, have: bool, sha: &[u8; 32], first: u16) -> Result<(), ()> {
    let path = cache_path(space_dir, sha);
    let file = if have { tokio::fs::File::open(&path).await.ok() } else { None };
    let Some(mut file) = file else { return none(s).await };
    let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let n = chunk_count(size);
    if size == 0 || size > FILE_MAX_BYTES || first as u64 >= n { return none(s).await; }
    file.seek(std::io::SeekFrom::Start(first as u64 * FILE_CHUNK as u64)).await.map_err(|_| ())?;
    for i in first as u64..n {
        let len = (size - i * FILE_CHUNK as u64).min(FILE_CHUNK as u64) as usize;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf).await.map_err(|_| ())?;
        let mut w = Writer::new();
        w.u16(i as u16).blob(&buf);
        match tokio::time::timeout(IO_WAIT, write_frame(&mut s.send, FILE_CHUNK_FRAME, &w.finish())).await {
            Ok(Ok(())) => {}
            _ => return Err(()),
        }
    }
    write_frame(&mut s.send, FILE_DONE, &[]).await.map_err(|_| ())
}

/// Hex text to bytes (any length), None on a bad digit or odd length.
pub fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fetch { Done, None, Broken }

/// Getter side: ask for the file from chunk 0, write chunks to `<cache>.part`, check every chunk against the offer,
/// and on the last one check the whole file and rename. Any result but Done removes the `.part` file.
pub async fn fetch_file(s: &mut SpaceStream, space: &SpaceId, offer: &FileOffer, space_dir: &Path,
                        progress: &(dyn Fn(u16, u16) + Send + Sync)) -> Fetch
{
    let cache = cache_path(space_dir, &offer.sha);
    let part = part_path(&cache);
    let out = fetch_inner(s, space, offer, &cache, &part, progress).await;
    if out != Fetch::Done { let _ = tokio::fs::remove_file(&part).await; }
    out
}

async fn fetch_inner(s: &mut SpaceStream, space: &SpaceId, offer: &FileOffer, cache: &Path, part: &Path,
                     progress: &(dyn Fn(u16, u16) + Send + Sync)) -> Fetch
{
    let total = offer.chunks.len() as u16;
    if write_frame(&mut s.send, FILE_WANT, &want_body(space, &offer.sha, 0)).await.is_err() { return Fetch::None; }
    if tokio::fs::create_dir_all(cache.parent().unwrap_or(Path::new("."))).await.is_err() { return Fetch::None; }
    let Ok(mut file) = tokio::fs::File::create(part).await else { return Fetch::None };
    let mut whole = Sha256::new();
    let mut next = 0usize;
    loop {
        let frame = match tokio::time::timeout(IO_WAIT, read_frame(&mut s.recv)).await {
            Ok(Ok(f)) => f,
            _ => return Fetch::None,
        };
        match frame.kind {
            FILE_NONE => return Fetch::None,
            FILE_CHUNK_FRAME => {
                let mut r = Reader::new(&frame.body);
                let (Ok(index), Ok(data)) = (r.u16(), r.blob(FILE_CHUNK)) else { return Fetch::Broken };
                if r.done().is_err() || index as usize != next || next >= offer.chunks.len() { return Fetch::Broken; }
                if data.len() != offer.chunk_len(next) { return Fetch::Broken; }
                let digest: [u8; 32] = Sha256::digest(data).into();
                if digest != offer.chunks[next] { return Fetch::Broken; }
                if file.write_all(data).await.is_err() { return Fetch::None; }
                whole.update(data);
                next += 1;
                progress(next as u16, total);
            }
            FILE_DONE => {
                if next != offer.chunks.len() { return Fetch::Broken; }
                let digest: [u8; 32] = whole.finalize().into();
                if digest != offer.sha { return Fetch::Broken; }
                if file.flush().await.is_err() || file.sync_all().await.is_err() { return Fetch::None; }
                drop(file);
                return if tokio::fs::rename(part, cache).await.is_ok() { Fetch::Done } else { Fetch::None };
            }
            _ => return Fetch::Broken,
        }
    }
}

/// Copy to `downloads` under the cleaned name; " (1)", " (2)" go before the extension if the name is taken. An
/// existing file is never overwritten. Returns the final name.
pub fn save_copy(cached: &Path, downloads: &Path, name: &str) -> Result<String, String> {
    let name = clean_file_name(name);
    std::fs::create_dir_all(downloads).map_err(|_| "Can't save to Downloads.".to_string())?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    for n in 0..1000u32 {
        let candidate = if n == 0 { name.clone() } else { format!("{stem} ({n}){ext}") };
        let target = downloads.join(&candidate);
        // The cleaned name has no separators, so the file lands directly in `downloads`.
        if target.parent() != Some(downloads) { return Err("Can't save to Downloads.".into()); }
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(mut dst) => {
                let Ok(mut src) = std::fs::File::open(cached) else {
                    drop(dst);
                    let _ = std::fs::remove_file(&target);
                    return Err("That file is not here any more.".into());
                };
                if std::io::copy(&mut src, &mut dst).is_err() {
                    drop(dst);
                    let _ = std::fs::remove_file(&target);
                    return Err("Can't save to Downloads.".into());
                }
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Can't save to Downloads.".into()),
        }
    }
    Err("Can't save to Downloads.".into())
}
