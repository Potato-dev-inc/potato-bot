//! Files (D8.1): offer encoding, name cleaning, and transfers between services on the in-memory net.
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use rusqlite::params;

use super::files::*;
use super::frame::write_frame;
use super::ids::{to_hex, NodeKey, SpaceId};
use super::ingest::{ingest_entry, Ingest};
use super::limits::{FILE_CHUNK, FILE_MAX_BYTES, KEEP_BYTES, TRIM_TO_BYTES};
use super::log::{Body, Entry, EntryHeader, KIND_FILE};
use super::memnet::MemoryHub;
use super::net::{PeerHint, SpaceNet, SpacePermissions};
use super::retention;
use super::service_tests::{clock, eventually, general, node, pair, trio, Node, TempDir};
use super::store::SpaceStore;

fn bytes_of(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

fn write_temp(dir: &TempDir, name: &str, data: &[u8]) -> String {
    std::fs::create_dir_all(&dir.0).unwrap();
    let p = dir.0.join(name);
    std::fs::write(&p, data).unwrap();
    p.to_string_lossy().to_string()
}

fn hint(n: NodeKey) -> PeerHint { PeerHint { node: n, relay: String::new(), lan: Vec::new() } }

fn space_dir(n: &Node, space: &SpaceId) -> PathBuf { n.dir.0.join(space.to_hex()) }

fn offer_of(data: &[u8], name: &str) -> FileOffer {
    let (sha, chunks) = hash_chunks(data);
    FileOffer { name: name.into(), size: data.len() as u64, sha, chunks, mime: "application/octet-stream".into() }
}

/// The file message of the newest message in #general, as the UI would get it.
fn file_message(n: &Node, space: &SpaceId) -> Option<serde_json::Value> {
    let ch = general(n, space);
    let v = n.svc.messages(&space.to_hex(), &ch.to_hex(), None, 50).ok()?;
    v.as_array()?.iter().rev().find(|m| !m["file"].is_null()).cloned()
}

async fn offer(a: &Node, space: &SpaceId, tmp: &TempDir, name: &str, data: &[u8]) -> String {
    let path = write_temp(tmp, name, data);
    let ch = general(a, space);
    let m = a.svc.offer_file(&space.to_hex(), &ch.to_hex(), &path).await.unwrap();
    m["file"]["sha"].as_str().unwrap().to_string()
}

#[test]
fn offer_body_round_trip() {
    let data = bytes_of(200 * 1024, 1);
    let o = offer_of(&data, "notes.txt");
    assert_eq!(o.chunks.len(), 4);
    assert_eq!(FileOffer::from_body(&o.to_body()).unwrap(), o);
    assert_eq!(o.chunk_len(3), 200 * 1024 - 3 * FILE_CHUNK);
}

#[test]
fn offer_over_25_mb_is_refused() {
    let tmp = TempDir::new("big");
    std::fs::create_dir_all(&tmp.0).unwrap();
    let p = tmp.0.join("big.bin");
    let f = std::fs::File::create(&p).unwrap();
    f.set_len(FILE_MAX_BYTES + 1).unwrap();
    drop(f);
    assert_eq!(offer_from_path(&p).err().as_deref(), Some("Files can be up to 25 MB."));
    // A folder is not a file.
    assert_eq!(offer_from_path(&tmp.0).err().as_deref(), Some("Can't read that file."));
}

#[test]
fn offer_with_wrong_chunk_count_is_rejected() {
    let data = bytes_of(200 * 1024, 2);
    let mut o = offer_of(&data, "a.txt");
    o.chunks.pop();
    assert!(FileOffer::from_body(&o.to_body()).is_err());
    let mut o = offer_of(&data, "a.txt");
    o.chunks.push([0; 32]);
    assert!(FileOffer::from_body(&o.to_body()).is_err());
}

#[test]
fn offer_with_bad_size_or_name_is_rejected() {
    let data = bytes_of(1000, 3);
    let mut o = offer_of(&data, "a.txt");
    o.size = FILE_MAX_BYTES + 1;
    assert!(FileOffer::from_body(&o.to_body()).is_err());
    let mut o = offer_of(&data, "a.txt");
    o.size = 0;
    assert!(FileOffer::from_body(&o.to_body()).is_err());
    // A peer's name is never trusted: one that is not already clean is refused.
    for bad in ["..\\..\\evil.exe", "/etc/passwd", "a\u{202E}gpj.exe", "con.txt", ""] {
        let mut o = offer_of(&data, "a.txt");
        o.name = bad.into();
        assert!(FileOffer::from_body(&o.to_body()).is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn file_names_are_cleaned() {
    assert_eq!(clean_file_name("..\\..\\evil.exe"), "evil.exe");
    assert_eq!(clean_file_name("/etc/passwd"), "passwd");
    assert_eq!(clean_file_name("a\u{202E}gpj.exe"), "agpj.exe");
    assert_eq!(clean_file_name(""), "file");
    assert_eq!(clean_file_name("...."), "file");
    assert_eq!(clean_file_name("C:\\x\\y.txt"), "y.txt");
    assert_eq!(clean_file_name("a:b*c?.txt"), "abc.txt");
    assert_eq!(clean_file_name("NUL.txt"), "_NUL.txt");
    assert_eq!(clean_file_name("name.txt. . "), "name.txt");
    assert_eq!(clean_file_name(&"x".repeat(300)).chars().count(), 120);
    assert_eq!(clean_file_name("bad\u{0}\u{7}name"), "badname");
}

#[tokio::test]
async fn member_gets_a_file_from_the_author() {
    let (_hub, _clk, a, b, space) = pair().await;
    let tmp = TempDir::new("f1");
    let data = bytes_of(300 * 1000, 4);
    let sha = offer(&a, &space, &tmp, "report.bin", &data).await;
    eventually("b sees the offer", || file_message(&b, &space).is_some()).await;
    // Nothing downloads by itself.
    let m = file_message(&b, &space).unwrap();
    assert_eq!(m["file"]["have"], false);
    assert_eq!(m["file"]["state"], "idle");
    assert_eq!(m["text"], "report.bin");
    assert!(!cache_path(&space_dir(&b, &space), &offer_of(&data, "x").sha).exists());
    b.svc.get_file(&space.to_hex(), &sha).await.unwrap();
    let got = std::fs::read(cache_path(&space_dir(&b, &space), &offer_of(&data, "x").sha)).unwrap();
    assert_eq!(got, data);
    assert_eq!(file_message(&b, &space).unwrap()["file"]["have"], true);
    // The author has its own copy.
    assert_eq!(file_message(&a, &space).unwrap()["file"]["have"], true);
    // No leftover .part file.
    let leftovers = std::fs::read_dir(space_dir(&b, &space).join("files")).unwrap().count();
    assert_eq!(leftovers, 1);
}

#[tokio::test]
async fn third_member_gets_it_from_a_non_author() {
    let (hub, _clk, a, b, c, space) = trio().await;
    let tmp = TempDir::new("f2");
    let data = bytes_of(150 * 1000, 5);
    let sha = offer(&a, &space, &tmp, "pic.bin", &data).await;
    eventually("b and c see the offer", || file_message(&b, &space).is_some() && file_message(&c, &space).is_some()).await;
    b.svc.get_file(&space.to_hex(), &sha).await.unwrap();
    // The author goes away; c can only ask b.
    hub.cut(a.key, c.key, true);
    eventually("c lost the author", || c.svc.link_count() == 1).await;
    c.svc.get_file(&space.to_hex(), &sha).await.unwrap();
    let got = std::fs::read(cache_path(&space_dir(&c, &space), &offer_of(&data, "x").sha)).unwrap();
    assert_eq!(got, data);
}

#[tokio::test]
async fn bad_chunk_is_detected() {
    let (hub, _clk, a, b, c, space) = trio().await;
    let tmp = TempDir::new("f3");
    let data = bytes_of(200 * 1000, 6);
    let sha = offer(&a, &space, &tmp, "doc.bin", &data).await;
    eventually("b and c see the offer", || file_message(&b, &space).is_some() && file_message(&c, &space).is_some()).await;
    b.svc.get_file(&space.to_hex(), &sha).await.unwrap();
    // b's cached copy is damaged on disk; b serves it as it is.
    let cache = cache_path(&space_dir(&b, &space), &offer_of(&data, "x").sha);
    let mut bad = std::fs::read(&cache).unwrap();
    bad[70_000] ^= 0xff;
    std::fs::write(&cache, bad).unwrap();
    hub.cut(a.key, c.key, true);
    eventually("c lost the author", || c.svc.link_count() == 1).await;
    let err = c.svc.get_file(&space.to_hex(), &sha).await.unwrap_err();
    assert_eq!(err, "This file didn't check out and was thrown away.");
    let files = space_dir(&c, &space).join("files");
    let left: Vec<_> = std::fs::read_dir(&files).map(|d| d.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "no cache file and no .part may stay: {left:?}");
    assert_eq!(file_message(&c, &space).unwrap()["file"]["have"], false);
}

#[tokio::test]
async fn no_one_has_it() {
    let (hub, _clk, a, b, space) = pair().await;
    let tmp = TempDir::new("f4");
    let sha = offer(&a, &space, &tmp, "gone.bin", &bytes_of(70_000, 7)).await;
    eventually("b sees the offer", || file_message(&b, &space).is_some()).await;
    hub.cut(a.key, b.key, true);
    eventually("b is alone", || b.svc.link_count() == 0).await;
    let err = b.svc.get_file(&space.to_hex(), &sha).await.unwrap_err();
    assert_eq!(err, "No one online has this file right now. Try again later.");
    assert_eq!(file_message(&b, &space).unwrap()["file"]["state"], "failed");
}

#[tokio::test]
async fn stranger_cannot_fetch() {
    let (hub, clk) = (MemoryHub::new(), clock());
    let a = node(&hub, 1, &clk).await;
    let hex = a.svc.create_space("Book club").await.unwrap();
    let space = SpaceId::from_hex(&hex).unwrap();
    let tmp = TempDir::new("f5");
    let sha = offer(&a, &space, &tmp, "secret.bin", &bytes_of(70_000, 8)).await;
    a.svc.create_invite(&hex).await.unwrap();
    let d = hub.node(NodeKey([9; 32]), SpacePermissions::new());
    let link = d.dial(&hint(a.key)).await.expect("an open invite lets a stranger knock");
    let mut s = link.open().await.unwrap();
    let want = want_body(&space, &super::ids::from_hex::<32>(&sha).unwrap(), 0);
    write_frame(&mut s.send, FILE_WANT, &want).await.unwrap();
    let code = tokio::time::timeout(Duration::from_secs(2), link.closed()).await.unwrap();
    assert_eq!(code, 2);
}

#[tokio::test]
async fn member_cannot_fetch_a_file_that_is_not_in_the_space() {
    let (_hub, _clk, a, b, space) = pair().await;
    let tmp = TempDir::new("f6");
    let _ = offer(&a, &space, &tmp, "x.bin", &bytes_of(70_000, 9)).await;
    // b asks a for some hash that no offer in this space names: FileNone, nothing served.
    let unknown = [0xabu8; 32];
    let err = b.svc.get_file(&space.to_hex(), &to_hex(&unknown)).await.unwrap_err();
    assert_eq!(err, "That file is not known.");
}

#[test]
fn save_copy_adds_suffix() {
    let tmp = TempDir::new("save");
    let downloads = tmp.0.join("Downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("a.txt"), b"old").unwrap();
    let cached = tmp.0.join("cached");
    std::fs::write(&cached, b"new").unwrap();
    assert_eq!(save_copy(&cached, &downloads, "a.txt").unwrap(), "a (1).txt");
    assert_eq!(save_copy(&cached, &downloads, "a.txt").unwrap(), "a (2).txt");
    assert_eq!(std::fs::read(downloads.join("a.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(downloads.join("a (1).txt")).unwrap(), b"new");
    // A name with folders in it still lands directly in Downloads.
    assert_eq!(save_copy(&cached, &downloads, "..\\..\\evil.exe").unwrap(), "evil.exe");
    assert!(downloads.join("evil.exe").is_file());
    assert!(!tmp.0.join("evil.exe").exists());
}

#[tokio::test]
async fn save_file_copies_the_cached_file() {
    let (_hub, _clk, a, _b, space) = pair().await;
    let tmp = TempDir::new("f7");
    let data = bytes_of(90_000, 10);
    let sha = offer(&a, &space, &tmp, "mine.bin", &data).await;
    let downloads = tmp.0.join("dl");
    let r = a.svc.save_file(&space.to_hex(), &sha, &downloads).unwrap();
    assert_eq!(r["name"], "mine.bin");
    assert_eq!(std::fs::read(downloads.join("mine.bin")).unwrap(), data);
    let r = a.svc.save_file(&space.to_hex(), &sha, &downloads).unwrap();
    assert_eq!(r["name"], "mine (1).bin");
}

#[tokio::test]
async fn bad_offer_is_not_stored_by_the_receiver() {
    let (_hub, _clk, a, b, space) = pair().await;
    // Signed by a real member, sealed with the right key, but the offer says 0 bytes.
    let mut o = offer_of(&bytes_of(100, 11), "a.txt");
    o.size = 0;
    let ch = general(&a, &space);
    let entry: Entry = a
        .svc
        .with_open(&space, |sp| {
            let device = sp.me.device_key();
            let header = EntryHeader {
                space, channel: ch, device, seq: sp.store.next_seq(&device).unwrap(), lamport: 50, created_ms: 1_700_000_000_000,
                epoch: sp.roster.epoch, kind: KIND_FILE, refers: None, bot: None,
            };
            Entry::seal(&sp.me, header, &Body::File(o.to_body()), &sp.keys.0[&sp.roster.epoch]).unwrap()
        })
        .unwrap();
    let verdict = b
        .svc
        .with_open(&space, |sp| ingest_entry(&sp.store, &sp.roster, &sp.keys, &entry.to_bytes(), 1_700_000_000_000))
        .unwrap();
    assert!(matches!(verdict, Ingest::Rejected(_)), "{verdict:?}");
    assert!(file_message(&b, &space).is_none());
}

#[test]
fn files_count_toward_retention() {
    let s = SpaceStore::open_memory().unwrap();
    const MB: i64 = 1024 * 1024;
    let device = super::identity::SpaceIdentity::for_test(2).device_key();
    for seq in 1..=150u64 {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&seq.to_be_bytes());
        s.conn
            .execute(
                "INSERT INTO entries (id, device, seq, channel, lamport, created_ms, received_ms, kind, ref_id, epoch, bytes, size, gone) \
                 VALUES (?1, ?2, ?3, ?4, ?3, ?5, 0, 1, NULL, 1, x'', ?6, 0)",
                params![id.to_vec(), device.0.to_vec(), seq as i64, vec![1u8; 8], 9_999_000_000i64 + seq as i64, MB],
            )
            .unwrap();
    }
    s.conn
        .execute("INSERT INTO files (sha, name, size, have, offer, cached_ms) VALUES (?1, 'f', ?2, 1, ?3, 5)", params![vec![7u8; 32], 60 * MB, vec![7u8; 32]])
        .unwrap();
    assert!(retention::used_bytes(&s).unwrap() > KEEP_BYTES);
    let gone: RefCell<Vec<[u8; 32]>> = RefCell::new(Vec::new());
    let r = retention::prune(&s, 10_000_000_000, &|sha| gone.borrow_mut().push(*sha)).unwrap();
    assert_eq!(*gone.borrow(), vec![[7u8; 32]]);
    assert_eq!((r.files, r.size_entries), (1, 0));
    assert!(retention::used_bytes(&s).unwrap() <= TRIM_TO_BYTES);
}
