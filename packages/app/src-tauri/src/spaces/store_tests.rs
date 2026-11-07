use super::identity::SpaceIdentity;
use super::ids::{ChannelId, InviteId, NodeKey, SpaceId};
use super::invite::{InviteRecord, InviteState};
use super::roster::*;
use super::store::SpaceStore;

const NOW: u64 = 1_000_000;

fn space() -> SpaceId { SpaceId([7; 16]) }
fn owner() -> SpaceIdentity { SpaceIdentity::for_test(1) }

fn genesis() -> (Roster, ControlOp) {
    Roster::genesis(&owner(), space(), "Club", "Owner", ChannelId([1; 8]), &[9; 32], NOW).unwrap()
}

fn temp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pm-spaces-{tag}-{}", std::process::id()))
}

#[test]
fn fresh_store_has_schema_v2() {
    let s = SpaceStore::open_memory().unwrap();
    assert_eq!(s.schema_version().unwrap(), 2);
    let n: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 15);
}

#[test]
fn reopen_keeps_data_and_does_not_rerun_schema() {
    let dir = temp("reopen");
    let path = dir.join("a").join("space.sqlite");
    {
        let s = SpaceStore::open(&path).unwrap();
        s.meta_set("role", b"owner").unwrap();
    }
    {
        let s = SpaceStore::open(&path).unwrap();
        assert_eq!(s.meta_get("role").unwrap(), Some(b"owner".to_vec()));
        assert_eq!(s.schema_version().unwrap(), 2);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A store made before bots (user_version 1) gets the two bot tables when it is opened, and keeps what it had.
#[test]
fn a_version_1_store_is_upgraded_and_keeps_its_data() {
    let dir = temp("upgrade");
    let path = dir.join("a").join("space.sqlite");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(super::store::SCHEMA).unwrap();
        c.execute("INSERT INTO meta (key, value) VALUES ('role', x'6f')", []).unwrap();
        c.execute("INSERT INTO bot_usage (at_ms, bot, person, kind) VALUES (5, x'0102030405060708', NULL, 'called')", []).unwrap();
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
    }
    let s = SpaceStore::open(&path).unwrap();
    assert_eq!(s.schema_version().unwrap(), 2);
    assert_eq!(s.meta_get("role").unwrap(), Some(vec![0x6f]));
    let used: i64 = s.conn.query_row("SELECT COUNT(*) FROM bot_usage", [], |r| r.get(0)).unwrap();
    assert_eq!(used, 1);
    for table in ["bot_local", "bot_handled"] {
        let n: i64 = s.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "{table}");
    }
    drop(s);
    // a second open does nothing more
    assert_eq!(SpaceStore::open(&path).unwrap().schema_version().unwrap(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_vacuum_is_incremental() {
    let dir = temp("vac");
    {
        let s = SpaceStore::open(&dir.join("space.sqlite")).unwrap();
        let v: i64 = s.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 2);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn meta_u64_round_trip() {
    let s = SpaceStore::open_memory().unwrap();
    assert_eq!(s.meta_u64("lamport").unwrap(), 0);
    s.meta_set_u64("lamport", 77).unwrap();
    assert_eq!(s.meta_u64("lamport").unwrap(), 77);
    s.meta_delete("lamport").unwrap();
    assert_eq!(s.meta_u64("lamport").unwrap(), 0);
}

#[test]
fn control_append_needs_next_seq() {
    let s = SpaceStore::open_memory().unwrap();
    let (_, g) = genesis();
    let mut skip = g.clone();
    skip.seq = 3;
    assert!(s.control_append(&g).unwrap());
    assert!(!s.control_append(&skip).unwrap());
    assert_eq!(s.control_ops().unwrap().len(), 1);
    assert_eq!(s.control_hash_at(1).unwrap(), Some(g.hash()));
    assert_eq!(s.control_hash_at(2).unwrap(), None);
}

#[test]
fn control_ops_round_trip_and_replay() {
    let s = SpaceStore::open_memory().unwrap();
    let (mut r, g) = genesis();
    assert!(s.control_append(&g).unwrap());
    let op = r.next_op(&owner(), OpBody::AddChannel { channel: ChannelId([2; 8]), name: "ideas".into() }, NOW).unwrap();
    r.apply(&op).unwrap();
    assert!(s.control_append(&op).unwrap());
    let back = Roster::replay(space(), &s.control_ops().unwrap()).unwrap();
    assert_eq!(back.channels.len(), 2);
    assert_eq!(s.control_ops_after(1).unwrap().len(), 1);
}

#[test]
fn invite_round_trip_all_states() {
    let s = SpaceStore::open_memory().unwrap();
    let states = [
        InviteState::Open,
        InviteState::Pending(NodeKey([5; 32])),
        InviteState::Used,
        InviteState::Dead,
        InviteState::Cancelled,
    ];
    for (i, st) in states.iter().enumerate() {
        let mut r = InviteRecord::new(InviteId([i as u8 + 1; 8]), 100 + i as u64);
        r.state = *st;
        r.strikes = i as u32;
        s.invite_put(&r).unwrap();
        assert_eq!(s.invite_get(&r.id).unwrap(), Some(r));
    }
    assert_eq!(s.invites().unwrap().len(), 5);
    assert_eq!(s.invite_get(&InviteId([99; 8])).unwrap(), None);
}

#[test]
fn join_request_lifecycle() {
    let s = SpaceStore::open_memory().unwrap();
    let id = s.join_request_add(&InviteId([1; 8]), &NodeKey([2; 32]), b"cert", "Sam", 10, 20).unwrap();
    assert_eq!(s.join_requests_waiting().unwrap().len(), 1);
    s.join_request_set(id, "let_in").unwrap();
    assert!(s.join_requests_waiting().unwrap().is_empty());
    assert_eq!(s.join_request(id).unwrap().unwrap().state, "let_in");
}

#[test]
fn bad_invite_state_text_is_rejected_by_sql() {
    let s = SpaceStore::open_memory().unwrap();
    let r = s.conn.execute(
        "INSERT INTO invites (id, created_ms, expires_ms, state) VALUES (x'01', 1, 2, 'weird')",
        [],
    );
    assert!(r.is_err());
}

#[test]
fn schema_has_bot_usage_and_no_fish() {
    let s = SpaceStore::open_memory().unwrap();
    let has = |t: &str| -> bool {
        s.conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = ?1", [t], |r| r.get::<_, i64>(0))
            .unwrap()
            > 0
    };
    assert!(has("bot_usage"));
    assert!(!has("fish_usage"));
}

#[test]
fn a_gone_placeholder_does_not_raise_high() {
    let s = SpaceStore::open_memory().unwrap();
    let d = super::ids::DeviceKey([4; 32]);
    s.insert_gone_placeholder(&d, 1u64 << 40).unwrap();
    assert_eq!(s.next_seq(&d).unwrap(), 1);
    assert!(s.heads().unwrap().iter().all(|h| h.2 < 1 << 40));
}

#[test]
fn the_lamport_clock_does_not_overflow() {
    let s = SpaceStore::open_memory().unwrap();
    s.meta_set_u64("lamport", u64::MAX).unwrap();
    assert_eq!(s.next_lamport().unwrap(), u64::MAX);
}
