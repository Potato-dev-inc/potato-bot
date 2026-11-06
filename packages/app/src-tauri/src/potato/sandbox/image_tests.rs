use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::image::*;
use super::install::Desktop;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("pm-image-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn manifest(base_url: &str, sha: &str, size: u64) -> Manifest {
    let text = format!(
        r#"{{"schema":1,"base_url":"{base_url}","images":{{"light":{{"file":"light.tar","size":{size},"sha256":"{sha}","guest_digest":"abc"}}}}}}"#
    );
    parse_manifest(&text).expect("the manifest parses")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn an_empty_base_url_or_a_missing_entry_means_no_image() {
    let none = manifest("", "00", 1);
    assert!(none.entry("light").is_none(), "an empty base_url turns every image off");
    let some = manifest("https://images.example.test/v1", "00", 1);
    assert!(some.entry("light").is_some());
    assert!(some.entry("mint").is_none(), "no entry for mint");
}

#[test]
fn only_schema_one_is_read() {
    assert!(parse_manifest(r#"{"schema":2,"base_url":"","images":{}}"#).is_none());
    assert!(parse_manifest("not json").is_none());
    assert!(parse_manifest(r#"{"schema":1,"base_url":"","images":{}}"#).is_some());
}

#[test]
fn the_image_key_follows_the_desktop_and_the_look() {
    assert_eq!(image_key_for(Desktop::Light), Some("light"));
    assert_eq!(image_key_for(Desktop::Mint), Some("mint"));
    assert_eq!(image_key_for(Desktop::Zorin), Some("zorin"));
    assert_eq!(image_key_for(Desktop::Xfce), None, "plain XFCE has no image");
    assert_eq!(image_key("xfce", &["xfce", "zorin-look"]), Some("zorin"));
    assert_eq!(image_key("xfce", &["xfce"]), None);
}

#[test]
fn the_dev_override_is_read_only_when_both_values_are_given() {
    assert!(dev_override(None, Some("ab".into())).is_none());
    assert!(dev_override(Some("x.tar".into()), None).is_none());
    assert!(dev_override(Some("  ".into()), Some("ab".into())).is_none());
    // Tests run in a debug build, where the override is honoured.
    let (path, sha) = dev_override(Some("C:\\img\\light.tar.zst".into()), Some("ABCD".into())).unwrap();
    assert_eq!(path, PathBuf::from("C:\\img\\light.tar.zst"));
    assert_eq!(sha, "abcd");
}

#[test]
fn a_valid_cached_file_is_used_without_a_download() {
    let dir = Scratch::new("cached");
    let cache = dir.0.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let body = b"fake image bytes";
    write(&cache.join("light.tar"), body);
    let sha = sha256_of(&cache.join("light.tar")).unwrap();
    // The base_url is not reachable: a download would fail, so a success proves the cache was used.
    let m = manifest("https://nowhere.invalid/v1", &sha, body.len() as u64);
    let entry = m.entry("light").unwrap().clone();
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let got = fetch(&m, &entry, &cache, &cancel, &mut |done, total| seen.push((done, total))).unwrap();
    assert_eq!(got, cache.join("light.tar"));
    assert_eq!(seen.last(), Some(&(body.len() as u64, body.len() as u64)));
}

#[test]
fn a_download_is_only_ever_https() {
    let dir = Scratch::new("http");
    let m = manifest("http://images.example.test/v1", "00", 10);
    let entry = m.entry("light").unwrap().clone();
    let cancel = AtomicBool::new(false);
    let err = fetch(&m, &entry, &dir.0, &cancel, &mut |_, _| {}).unwrap_err();
    assert_eq!(err, ImageError::NotHttps);
    assert!(!dir.0.join("light.tar.part").exists(), "nothing was started");
}

#[test]
fn a_cancel_before_the_download_stops_at_once() {
    let dir = Scratch::new("cancel");
    let m = manifest("https://nowhere.invalid/v1", "00", 10);
    let entry = m.entry("light").unwrap().clone();
    let cancel = AtomicBool::new(true);
    assert_eq!(fetch(&m, &entry, &dir.0, &cancel, &mut |_, _| {}).unwrap_err(), ImageError::Cancelled);
}

#[test]
fn a_cached_file_with_the_wrong_checksum_is_not_used() {
    let dir = Scratch::new("badcache");
    write(&dir.0.join("light.tar"), b"not the image");
    let m = manifest("https://nowhere.invalid/v1", "0000", 10);
    let entry = m.entry("light").unwrap().clone();
    let cancel = AtomicBool::new(false);
    // It is not used, so the (unreachable) download is tried and fails: never a success.
    assert!(fetch(&m, &entry, &dir.0, &cancel, &mut |_, _| {}).is_err());
}

#[test]
fn a_plain_tar_is_copied_as_it_is() {
    let dir = Scratch::new("tar");
    let src = dir.0.join("light.tar");
    let body: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    write(&src, &body);
    let out = dir.0.join("base.tar");
    unpack_to_tar(&src, &out, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert!(!dir.0.join("base.tar.part").exists());
}

#[test]
fn a_zst_that_is_not_zstd_is_refused_and_leaves_no_output() {
    let dir = Scratch::new("badzst");
    let src = dir.0.join("light.tar.zst");
    write(&src, b"this is not zstd data at all");
    let out = dir.0.join("base.tar");
    let err = unpack_to_tar(&src, &out, &AtomicBool::new(false), &mut |_, _| {}).unwrap_err();
    assert!(matches!(err, ImageError::Unreadable(_)), "{err:?}");
    assert!(!out.exists());
}

#[test]
fn the_errors_read_as_plain_sentences() {
    for error in [ImageError::Cancelled, ImageError::NotHttps, ImageError::HashMismatch, ImageError::Http("timeout".into())] {
        assert!(error.to_string().len() > 10, "{error}");
    }
}

#[test]
fn unpack_reports_the_compressed_bytes_read_and_ends_at_the_file_size() {
    let dir = Scratch::new("progress");
    let src = dir.0.join("light.tar");
    let body: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    write(&src, &body);
    let out = dir.0.join("base.tar");
    let mut reports: Vec<(u64, u64)> = Vec::new();
    unpack_to_tar(&src, &out, &AtomicBool::new(false), &mut |done, total| reports.push((done, total))).unwrap();
    let last = *reports.last().expect("at least the final report");
    assert_eq!(last, (body.len() as u64, body.len() as u64));
    assert!(reports.iter().all(|&(done, total)| done <= total && total == body.len() as u64), "{reports:?}");
}
