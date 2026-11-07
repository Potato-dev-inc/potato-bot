//! The leak check: what it holds back, what it lets through, and that no input can make it panic.
use super::leak::{leak_reason, PrivateMarks};

fn marks() -> PrivateMarks {
    PrivateMarks { home: Some("C:\\Users\\ren".into()), user: Some("ren".into()), extra: vec!["ren@example.com".into()] }
}

fn why(t: &str) -> Option<&'static str> { leak_reason(t, &marks()) }

#[test]
fn home_path_in_a_reply_is_blocked() {
    assert_eq!(why("look in C:\\Users\\ren\\Documents"), Some("home folder"));
    // JSON-escaped backslashes are the same folder
    assert_eq!(why("look in C:\\\\Users\\\\ren\\\\Documents"), Some("home folder"));
}

#[test]
fn forward_slash_home_is_blocked() {
    assert_eq!(why("c:/users/ren/x"), Some("home folder"));
    assert_eq!(why("C:/USERS/REN"), Some("home folder"));
}

#[test]
fn a_different_user_with_the_same_start_is_not_this_user() {
    assert_eq!(why("it is in C:\\Users\\renee\\Documents"), None);
    assert_eq!(why("/home/renata/a"), None);
}

#[test]
fn mac_and_linux_user_folders() {
    assert_eq!(why("/Users/ren/a"), Some("user folder"));
    assert_eq!(why("/home/ren/b"), Some("user folder"));
    assert_eq!(why("in \\home\\ren"), Some("user folder"));
}

#[test]
fn emails_are_blocked() {
    assert_eq!(why("mail sam@example.org"), Some("email"));
    assert_eq!(why("sam.o+tag@sub.example.co.uk."), Some("email"));
    assert_eq!(why("meet @sam at 6, or @ the cafe"), None);
    assert_eq!(why("a@b"), None);
}

#[test]
fn owner_extra_is_blocked_whatever_the_case() {
    assert_eq!(why("Ren@Example.com"), Some("owner detail"));
}

#[test]
fn tokens_are_blocked() {
    assert_eq!(why("key is sk-abcdefghijklmnopqrstu"), Some("token"));
    assert_eq!(why(&format!("ghp_{}", "a1B2".repeat(9))), Some("token"));
    assert_eq!(why("github_pat_11ABCDEFG0abcdefghijklmnop"), Some("token"));
    assert_eq!(why("-----BEGIN OPENSSH PRIVATE KEY-----"), Some("token"));
    assert_eq!(why("ssh-ed25519 AAAAC3Nza"), Some("token"));
    assert_eq!(why("AKIAIOSFODNN7EXAMPLE"), Some("token"));
    assert_eq!(why("token=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.abc"), Some("token"));
}

#[test]
fn long_hex_and_long_secrets_are_blocked() {
    assert_eq!(why(&"ab12".repeat(16)), Some("long hex"));
    assert_eq!(why(&"wxyz".repeat(11)), Some("long secret"));
    assert_eq!(why(&"Qw+/Zx=".repeat(7)), Some("long secret"));
}

#[test]
fn normal_chat_passes() {
    for t in [
        "Sure! The meetup is at 6pm on Friday.",
        "sk-8 is a bus line",
        "a task-based approach to disk-space problems",
        "Akiala Tanaka will bring snacks",
        "see https://example.com/docs/getting-started for more",
        "今晚七點在老地方見，記得帶書。",
        "#0f0f0f is the colour",
    ] {
        assert_eq!(why(t), None, "{t}");
    }
}

#[test]
fn no_home_known_still_runs_the_other_rules() {
    let m = PrivateMarks::default();
    assert_eq!(leak_reason("C:\\Users\\ren\\x", &m), None);
    assert_eq!(leak_reason("sam@example.org", &m), Some("email"));
}

/// A lowercased copy can be longer than the text (`İ` becomes two characters): offsets from it must never slice the
/// original. The scan works on the text itself, so these do not panic and still find the secret.
#[test]
fn multi_byte_text_before_a_secret_cannot_panic() {
    assert_eq!(why("İİİİİİİİİİ sk-abcdefghijklmnopqrstu"), Some("token"));
    assert_eq!(why("你好嗎 ghp_abcdefghijklmnopqrstuvwxyz0123456789"), Some("token"));
    assert_eq!(why("İİİ ssh-rsa AAAA"), Some("token"));
    assert_eq!(why("ẞẞ-----BEGIN PRIVATE KEY-----"), Some("token"));
    assert_eq!(why("İ-----BEGIN PRIVATE KEY-----"), Some("token"));
    assert_eq!(why("😀😀AKIAIOSFODNN7EXAMPLE"), Some("token"));
    assert_eq!(why("İİİİ eyJ"), None);
    // a prefix cut short at the end of the text
    for t in ["sk-", "ghp_", "AKIA", "eyJ", "a@", "@", "İ@İ.com"] {
        assert_eq!(why(t), None, "{t}");
    }
}

#[test]
fn marks_for_this_pc_do_not_panic() {
    let m = PrivateMarks::for_this_pc();
    assert!(m.extra.is_empty());
}
