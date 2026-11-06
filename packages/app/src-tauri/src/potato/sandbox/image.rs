//! The prebuilt sandbox image: a ready Ubuntu with the desktop already in it, so the install does not download
//! Ubuntu from Canonical and build the desktop on the person's PC.
//!
//! The manifest is `images.json` next to the guest files (`{"schema":1,"base_url":"","images":{"light":{...}}}`).
//! An empty `base_url` or a missing entry means there is no image, and the install uses today's plan.
//! An image is used only when its `guest_digest` equals the digest of the guest files this build ships.
//!
//! Download: `<cache>/<file>.part` with HTTP `Range` resume, real bytes reported, cancel honoured, sha256 checked,
//! then renamed to `<cache>/<file>`. A cached file that already has the right sha256 is used as is. Only https.
//! The archive is `.tar.zst` (decoded as a stream with `ruzstd`) or `.tar`; `wsl --import` takes the tar.
//!
//! Dev override (debug builds only): `PM_SBX_IMAGE_FILE` is a local `.tar.zst` or `.tar`, and `PM_SBX_IMAGE_SHA256`
//! is its sha256. Nothing is downloaded then. Release builds ignore both variables.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::install::Desktop;

/// The manifest's file name, next to the guest files.
pub const MANIFEST_FILE: &str = "images.json";
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK: usize = 256 * 1024;
/// How often the unpack reports its progress (about four times a second).
const UNPACK_REPORT_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ImageEntry {
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub guest_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub base_url: String,
    pub images: BTreeMap<String, ImageEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    Cancelled,
    NotHttps,
    Http(String),
    Io(String),
    HashMismatch,
    Unreadable(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Cancelled => f.write_str("the download was stopped"),
            ImageError::NotHttps => f.write_str("the image address is not https"),
            ImageError::Http(why) => write!(f, "the download failed ({why})"),
            ImageError::Io(why) => write!(f, "a file could not be written ({why})"),
            ImageError::HashMismatch => f.write_str("the image file does not match its checksum"),
            ImageError::Unreadable(why) => write!(f, "the image file could not be read ({why})"),
        }
    }
}

/// Reads the manifest text. `None` for a manifest this build does not understand (schema 1 only).
pub fn parse_manifest(text: &str) -> Option<Manifest> {
    let manifest: Manifest = serde_json::from_str(text).ok()?;
    (manifest.schema == 1).then_some(manifest)
}

/// The manifest that ships with the guest files, if there is one.
pub fn load_manifest(guest_root: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(guest_root.join(MANIFEST_FILE)).ok()?;
    parse_manifest(&text)
}

impl Manifest {
    /// The entry for `key`, or `None` when there is no image: an empty `base_url` turns every image off.
    pub fn entry(&self, key: &str) -> Option<&ImageEntry> {
        if self.base_url.trim().is_empty() {
            return None;
        }
        self.images.get(key)
    }
}

/// Which image a set up uses. `desktop` is the desktop recipe (`Desktop::recipe_id`), `recipes` every recipe chosen.
/// Light, Mint, and XFCE with the Zorin look have an image; plain XFCE has none.
pub fn image_key(desktop: &str, recipes: &[&str]) -> Option<&'static str> {
    match desktop {
        "light" => Some("light"),
        "mint" => Some("mint"),
        "xfce" if recipes.contains(&"zorin-look") => Some("zorin"),
        _ => None,
    }
}

/// The image key for a wizard desktop choice.
pub fn image_key_for(desktop: Desktop) -> Option<&'static str> {
    image_key(desktop.recipe_id(), &desktop.recipe_ids())
}

/// The dev override from the two variables. Debug builds only: a release build never reads them.
pub fn dev_override(file: Option<String>, sha256: Option<String>) -> Option<(PathBuf, String)> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let file = file.filter(|f| !f.trim().is_empty())?;
    let sha256 = sha256.filter(|s| !s.trim().is_empty())?;
    Some((PathBuf::from(file), sha256.trim().to_ascii_lowercase()))
}

/// The dev override read from the environment.
pub fn dev_override_from_env() -> Option<(PathBuf, String)> {
    dev_override(std::env::var("PM_SBX_IMAGE_FILE").ok(), std::env::var("PM_SBX_IMAGE_SHA256").ok())
}

/// The sha256 of a file, lowercase hex, streamed.
pub fn sha256_of(path: &Path) -> Result<String, ImageError> {
    let mut file = File::open(path).map_err(|e| ImageError::Unreadable(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).map_err(|e| ImageError::Unreadable(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Where the image is kept: `<cache>/<file>`. The cache folder is made if needed.
pub fn cached_path(cache: &Path, entry: &ImageEntry) -> PathBuf {
    cache.join(&entry.file)
}

/// Gets the image file for `entry` and returns its path, verified. Order: dev override, then a valid cached file, then
/// a (resumed) download. `progress` gets `(bytes done, bytes total)` as the download moves.
pub fn fetch(
    manifest: &Manifest,
    entry: &ImageEntry,
    cache: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, ImageError> {
    if let Some((path, sha)) = dev_override_from_env() {
        let got = sha256_of(&path)?;
        if got != sha {
            return Err(ImageError::HashMismatch);
        }
        progress(entry.size, entry.size);
        return Ok(path);
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(ImageError::Cancelled);
    }
    std::fs::create_dir_all(cache).map_err(|e| ImageError::Io(e.to_string()))?;
    let final_path = cached_path(cache, entry);
    if final_path.is_file() && sha256_of(&final_path).ok().as_deref() == Some(entry.sha256.as_str()) {
        progress(entry.size, entry.size);
        return Ok(final_path);
    }
    let url = format!("{}/{}", manifest.base_url.trim_end_matches('/'), entry.file);
    if !url.starts_with("https://") {
        return Err(ImageError::NotHttps);
    }
    let part = cache.join(format!("{}.part", entry.file));
    download(&url, &part, entry.size, cancel, progress)?;
    if sha256_of(&part)? != entry.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(ImageError::HashMismatch);
    }
    std::fs::rename(&part, &final_path).map_err(|e| ImageError::Io(e.to_string()))?;
    Ok(final_path)
}

/// Downloads `url` into `part`, resuming from what is already there (HTTP `Range`). A server that ignores the range
/// starts the file again.
fn download(url: &str, part: &Path, total: u64, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64)) -> Result<(), ImageError> {
    let mut have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have > total {
        let _ = std::fs::remove_file(part);
        have = 0;
    }
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(READ_TIMEOUT).build();
    let mut request = agent.get(url);
    if have > 0 {
        request = request.set("Range", &format!("bytes={have}-"));
    }
    let response = request.call().map_err(|e| ImageError::Http(e.to_string()))?;
    let resumed = response.status() == 206;
    if !resumed {
        have = 0;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resumed)
        .open(part)
        .map_err(|e| ImageError::Io(e.to_string()))?;
    if resumed {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::End(0)).map_err(|e| ImageError::Io(e.to_string()))?;
    }
    let mut reader = response.into_reader();
    let mut buf = vec![0u8; CHUNK];
    let mut done = have;
    progress(done, total);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(ImageError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| ImageError::Http(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| ImageError::Io(e.to_string()))?;
        done += n as u64;
        progress(done.min(total), total);
    }
    file.flush().map_err(|e| ImageError::Io(e.to_string()))?;
    Ok(())
}

/// Counts the bytes that pass through it, so the unpack can say how far into the compressed file it is.
struct CountingReader<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Turns a `.tar.zst` into a plain `.tar` at `out`, as a stream (the archive is never held whole in memory). A `.tar`
/// is copied as it is. `progress` gets `(compressed bytes read, compressed file size)`, about four times a second and at the end.
pub fn unpack_to_tar(image: &Path, out: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64)) -> Result<(), ImageError> {
    let is_zst = image.extension().is_some_and(|e| e == "zst");
    let total = std::fs::metadata(image).map(|m| m.len()).map_err(|e| ImageError::Unreadable(e.to_string()))?;
    let input = File::open(image).map_err(|e| ImageError::Unreadable(e.to_string()))?;
    let count = Arc::new(AtomicU64::new(0));
    let input = CountingReader { inner: input, count: Arc::clone(&count) };
    let tmp = out.with_extension("tar.part");
    let mut output = File::create(&tmp).map_err(|e| ImageError::Io(e.to_string()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut reader: Box<dyn Read> = if is_zst {
        Box::new(ruzstd::decoding::StreamingDecoder::new(input).map_err(|e| ImageError::Unreadable(e.to_string()))?)
    } else {
        Box::new(input)
    };
    let mut last_report = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&tmp);
            return Err(ImageError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| ImageError::Unreadable(e.to_string()))?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n]).map_err(|e| ImageError::Io(e.to_string()))?;
        if last_report.elapsed() >= UNPACK_REPORT_EVERY {
            progress(count.load(Ordering::Relaxed).min(total), total);
            last_report = std::time::Instant::now();
        }
    }
    progress(total, total);
    output.flush().map_err(|e| ImageError::Io(e.to_string()))?;
    drop(output);
    std::fs::rename(&tmp, out).map_err(|e| ImageError::Io(e.to_string()))?;
    Ok(())
}
