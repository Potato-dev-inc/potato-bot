use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::Connection;

use super::go_key::*;

static N: AtomicUsize = AtomicUsize::new(0);

/// A database file removed when the test ends.
struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("db-shm"));
    }
}

fn db(rows: &[(&str, bool, &str)]) -> TempDb {
    let path = std::env::temp_dir().join(format!(
        "pm-opencode-key-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE credential (integration_id TEXT NOT NULL, label TEXT, active INTEGER NOT NULL, value TEXT NOT NULL);",
    )
    .unwrap();
    for (integration, active, value) in rows {
        conn.execute(
            "INSERT INTO credential (integration_id, label, active, value) VALUES (?1, NULL, ?2, ?3)",
            rusqlite::params![integration, i32::from(*active), value],
        )
        .unwrap();
    }
    TempDb(path)
}

fn read(db: &TempDb) -> Option<String> {
    read_at(&db.0)
}

#[test]
fn reads_the_active_api_key_for_the_go_integration() {
    let db = db(&[("opencode-go", true, r#"{"type":"api","key":"sk-go-123"}"#)]);
    assert_eq!(read(&db).as_deref(), Some("sk-go-123"));
}

#[test]
fn an_oauth_access_token_counts_as_the_key() {
    let db = db(&[("opencode", true, r#"{"type":"oauth","access":"access-456","refresh":"r","expires":0}"#)]);
    assert_eq!(read(&db).as_deref(), Some("access-456"));
}

#[test]
fn the_go_integration_wins_over_the_personal_one() {
    let db = db(&[
        ("opencode", true, r#"{"type":"api","key":"personal"}"#),
        ("opencode-go", true, r#"{"type":"api","key":"go-key"}"#),
    ]);
    assert_eq!(read(&db).as_deref(), Some("go-key"));
}

#[test]
fn an_inactive_row_is_not_a_key() {
    let db = db(&[("opencode-go", false, r#"{"type":"api","key":"inactive"}"#)]);
    assert_eq!(read(&db), None);
}

#[test]
fn a_missing_or_malformed_database_is_no_key() {
    assert_eq!(read_at(Path::new("this-file-does-not-exist.db")), None);

    let no_table = std::env::temp_dir().join(format!("pm-opencode-key-empty-{}-{}.db", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_file(&no_table);
    Connection::open(&no_table).unwrap().execute_batch("CREATE TABLE other (x TEXT);").unwrap();
    assert_eq!(read_at(&no_table), None);
    let _ = std::fs::remove_file(&no_table);
}

#[test]
fn a_value_without_a_key_or_access_is_no_key() {
    assert_eq!(key_from_value(r#"{"type":"api"}"#), None);
    assert_eq!(key_from_value(r#"{"type":"api","key":""}"#), None);
    assert_eq!(key_from_value("not json"), None);
}
