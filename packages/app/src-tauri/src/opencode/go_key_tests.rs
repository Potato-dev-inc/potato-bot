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

// ---- the key check, against a local fake server --------------------------------------------------------

mod check {
    use super::super::go_key::check_key_at;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    const FAKE: &str = "oc-fake-key-0123456789abcdef";

    /// One request, answered with `status`; the request head is sent back through the channel.
    fn serve(status: u16) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let body = format!("{{\"error\":\"echo {FAKE}\"}}");
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (url, rx)
    }

    #[test]
    fn an_accepted_key_works_and_goes_only_in_the_bearer_header() {
        let (url, rx) = serve(200);
        check_key_at(&url, FAKE).unwrap();
        let head = rx.recv().unwrap().to_lowercase();
        assert!(head.contains(&format!("authorization: bearer {}", FAKE.to_lowercase())));
        assert!(!head.lines().next().unwrap().contains(&FAKE.to_lowercase()), "never in the address");
    }

    #[test]
    fn a_rejected_key_gets_one_plain_sentence_without_the_key_or_the_server_text() {
        for status in [401u16, 403] {
            let (url, _rx) = serve(status);
            let e = check_key_at(&url, FAKE).unwrap_err();
            assert!(e.contains("did not accept that key"), "{e}");
            assert!(!e.contains(FAKE) && !e.contains("echo") && !e.contains("127.0.0.1"), "{e}");
        }
    }

    #[test]
    fn a_wrong_shape_is_told_before_anything_is_sent() {
        for bad in ["", "   ", "two words", "line\nbreak"] {
            let e = check_key_at("http://127.0.0.1:1/v1/models", bad).unwrap_err();
            assert!(e.contains("does not look like"), "{e}");
        }
    }

    #[test]
    fn busy_other_and_unreachable_are_plain_sentences() {
        let (url, _rx) = serve(429);
        assert!(check_key_at(&url, FAKE).unwrap_err().contains("busy"));
        let (url, _rx) = serve(500);
        assert!(check_key_at(&url, FAKE).unwrap_err().contains("could not answer"));
        let e = check_key_at("http://127.0.0.1:1/v1/models", FAKE).unwrap_err();
        assert!(e.contains("Could not reach") && !e.contains(FAKE), "{e}");
    }
}
