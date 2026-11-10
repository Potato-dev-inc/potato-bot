//! Attachments: what is kept, what is refused, and the one text message the brain gets.

use std::path::PathBuf;

use base64::Engine as _;

use super::attachments::*;
use super::store::MAX_MESSAGE_CHARS;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-potato-attach-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn text_file(name: &str, body: &str) -> Incoming {
    Incoming { name: name.into(), kind: "text".into(), data: b64(body.as_bytes()) }
}

const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 1];

fn png_file(name: &str) -> Incoming {
    Incoming { name: name.into(), kind: "image".into(), data: b64(&PNG) }
}

#[test]
fn a_name_loses_its_path_and_odd_characters_and_keeps_its_end() {
    assert_eq!(safe_name("C:\\Users\\me\\notes.txt"), "notes.txt");
    assert_eq!(safe_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_name(".env"), "env");
    assert_eq!(safe_name("a<b>:c?.md"), "a_b__c_.md");
    assert_eq!(safe_name(""), "file");
    assert_eq!(safe_name("///"), "file");
    let long = format!("{}.txt", "x".repeat(200));
    let cut = safe_name(&long);
    assert_eq!(cut.chars().count(), 60);
    assert!(cut.ends_with(".txt"));
}

#[test]
fn a_picture_is_known_by_its_first_bytes_not_by_its_name() {
    assert_eq!(image_ext(&PNG), Some("png"));
    assert_eq!(image_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
    assert_eq!(image_ext(b"GIF89a.."), Some("gif"));
    assert_eq!(image_ext(b"RIFF\x01\x00\x00\x00WEBPVP8 "), Some("webp"));
    assert_eq!(image_ext(b"hello"), None);
    assert_eq!(image_ext(b""), None);
}

#[test]
fn saves_files_in_the_bot_folder_and_names_them_there() {
    let root = temp("save");
    let saved = save_all(&root, "nimbus", &[text_file("notes.txt", "hello there"), png_file("shot.jpeg")], 1700).unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0].rel, "attachments/1700-1-notes.txt");
    assert_eq!(saved[0].kind, Kind::Text);
    assert_eq!(saved[0].text.as_deref(), Some("hello there"));
    // The picture takes its real extension, not the one the page claimed.
    assert_eq!(saved[1].name, "shot.png");
    assert_eq!(saved[1].kind, Kind::Image);
    let base = root.join("brain").join("nimbus");
    assert_eq!(std::fs::read_to_string(base.join(&saved[0].rel)).unwrap(), "hello there");
    assert_eq!(std::fs::read(base.join(&saved[1].rel)).unwrap(), PNG.to_vec());
}

#[test]
fn refuses_a_bad_bot_name_too_many_files_and_big_files_and_saves_nothing() {
    let root = temp("refuse");
    let one = [text_file("a.txt", "a")];
    assert_eq!(save_all(&root, "../evil", &one, 1).unwrap_err().code, "POTATO_BOT_NOT_FOUND");
    let five: Vec<Incoming> = (0..5).map(|i| text_file(&format!("{i}.txt"), "x")).collect();
    assert!(save_all(&root, "nimbus", &five, 1).unwrap_err().message.contains("at most 4"));
    let big = text_file("big.txt", &"x".repeat(MAX_FILE_BYTES + 1));
    assert!(save_all(&root, "nimbus", &[big], 1).unwrap_err().message.contains("bigger than 2.0 MB"));
    // Three 1.5 MB files are each fine and together too big.
    let part = "y".repeat(1_500_000);
    let three = [text_file("a.txt", &part), text_file("b.txt", &part), text_file("c.txt", &part)];
    assert!(save_all(&root, "nimbus", &three, 1).unwrap_err().message.contains("together"));
    // A good file next to a bad one: neither is kept.
    let mixed = [text_file("ok.txt", "fine"), Incoming { name: "x.bin".into(), kind: "text".into(), data: b64(&[1, 0, 2, 3]) }];
    assert!(save_all(&root, "nimbus", &mixed, 1).is_err());
    assert!(!root.join("brain").join("nimbus").join(FOLDER).exists());
}

#[test]
fn refuses_empty_files_bad_base64_binary_and_a_picture_that_is_not_one() {
    let root = temp("kinds");
    let empty = Incoming { name: "e.txt".into(), kind: "text".into(), data: String::new() };
    assert!(save_all(&root, "nimbus", &[empty], 1).unwrap_err().message.contains("empty"));
    let junk = Incoming { name: "j.txt".into(), kind: "text".into(), data: "***not base64***".into() };
    assert!(save_all(&root, "nimbus", &[junk], 1).unwrap_err().message.contains("could not be read"));
    let exe = Incoming { name: "run.txt".into(), kind: "text".into(), data: b64(b"MZ\x90\x00\x03\x00") };
    assert!(save_all(&root, "nimbus", &[exe], 1).unwrap_err().message.contains("not a text file"));
    let fake = Incoming { name: "cat.png".into(), kind: "image".into(), data: b64(b"just words") };
    assert!(save_all(&root, "nimbus", &[fake], 1).unwrap_err().message.contains("not a picture"));
    let latin1 = Incoming { name: "l.txt".into(), kind: "text".into(), data: b64(&[0x68, 0xE9, 0x6C, 0x6C, 0x6F]) };
    assert!(save_all(&root, "nimbus", &[latin1], 1).is_err());
}

#[test]
fn forget_removes_only_what_was_saved_there() {
    let root = temp("forget");
    let saved = save_all(&root, "nimbus", &[text_file("a.txt", "keep me?")], 5).unwrap();
    let path = root.join("brain").join("nimbus").join(&saved[0].rel);
    assert!(path.exists());
    forget(&root, "nimbus", &saved);
    assert!(!path.exists());
    // A name outside the folder is ignored.
    let outside = root.join("brain").join("nimbus").join("keep.txt");
    std::fs::write(&outside, "x").unwrap();
    let sneaky = Saved { name: "keep.txt".into(), rel: "../keep.txt".into(), kind: Kind::Text, bytes: 1, text: None, guest: Guest::Off };
    forget(&root, "nimbus", &[sneaky]);
    assert!(outside.exists());
}

#[test]
fn the_message_has_the_words_a_line_per_file_and_the_text_of_a_small_text_file() {
    let root = temp("compose");
    let saved = save_all(&root, "nimbus", &[text_file("todo.md", "- milk\n- eggs"), png_file("pic.png")], 9).unwrap();
    let message = compose("  look at these  ", &saved).unwrap();
    assert!(message.starts_with("look at these\n\n[Attached file: todo.md (13 B), saved at attachments/9-1-todo.md on the PC (your computer is off)]"));
    assert!(message.contains("[Attached image: pic.png (12 B), saved at attachments/9-2-pic.png on the PC (your computer is off)]"));
    assert!(message.contains("todo.md:\n```\n- milk\n- eggs\n```"));
    // The picture's bytes are never in the message.
    assert!(!message.contains("PNG"));
}

#[test]
fn only_files_and_no_words_still_makes_a_message() {
    let root = temp("only");
    let saved = save_all(&root, "nimbus", &[png_file("a.png")], 1).unwrap();
    let message = compose("", &saved).unwrap();
    assert!(message.starts_with("[Attached image: a.png"));
}

#[test]
fn a_long_text_file_is_cut_to_fit_and_says_so_and_the_message_never_passes_the_cap() {
    let root = temp("cut");
    let body = "line of text\n".repeat(900);
    let saved = save_all(&root, "nimbus", &[text_file("log.txt", &body)], 2).unwrap();
    let message = compose(&"w".repeat(500), &saved).unwrap();
    assert!(message.chars().count() <= MAX_MESSAGE_CHARS);
    assert!(message.contains("(Cut here. The whole file is saved.)"));
    assert!(message.contains("log.txt:\n```\nline of text"));
}

#[test]
fn a_file_that_leaves_no_room_is_named_but_not_quoted_and_too_many_words_are_refused() {
    let root = temp("room");
    let saved = save_all(&root, "nimbus", &[text_file("a.txt", &"z".repeat(5000))], 3).unwrap();
    let almost_full = "w".repeat(MAX_MESSAGE_CHARS - 120);
    let message = compose(&almost_full, &saved).unwrap();
    assert!(message.chars().count() <= MAX_MESSAGE_CHARS);
    assert!(message.contains("saved at attachments/3-1-a.txt on the PC (your computer is off)]"));
    assert!(!message.contains("```"));
    let err = compose(&"w".repeat(MAX_MESSAGE_CHARS), &saved).unwrap_err();
    assert_eq!(err.code, "POTATO_ATTACHMENT");
    assert!(err.message.contains("Shorten"));
}

#[test]
fn a_quote_that_contains_backticks_gets_a_longer_fence() {
    let root = temp("fence");
    let saved = save_all(&root, "nimbus", &[text_file("c.md", "before\n```js\nlet a = 1;\n```\nafter")], 4).unwrap();
    let message = compose("", &saved).unwrap();
    assert!(message.contains("c.md:\n````\nbefore"));
    assert!(message.trim_end().ends_with("after\n````"));
}

#[test]
fn size_words_are_short() {
    assert_eq!(size_word(300), "300 B");
    assert_eq!(size_word(2048), "2 KB");
    assert_eq!(size_word(1_572_864), "1.5 MB");
}

// ---- where the bot finds the file -----------------------------------------------------------------------------------

use super::sandbox::{FakeSandbox, Sandbox, SandboxError};

fn saved_one(root: &std::path::Path) -> Vec<Saved> {
    save_all(root, "nimbus", &[png_file("image.png")], 1794280014692).unwrap()
}

#[test]
fn the_line_names_the_path_inside_the_computer_when_it_was_copied() {
    let root = temp("line-on");
    let mut saved = saved_one(&root);
    saved[0].guest = Guest::At("/home/pm/attachments/1794280014692-1-image.png".into());
    let message = compose("see", &saved).unwrap();
    assert!(
        message.contains("[Attached image: image.png (12 B), saved on your computer at /home/pm/attachments/1794280014692-1-image.png]"),
        "{message}"
    );
    assert!(!message.contains("on the PC"));
}

#[test]
fn the_line_says_the_computer_is_off_or_that_the_copy_failed() {
    let root = temp("line-off");
    let mut saved = saved_one(&root);
    let off = compose("", &saved).unwrap();
    assert!(off.contains("saved at attachments/1794280014692-1-image.png on the PC (your computer is off)"), "{off}");
    saved[0].guest = Guest::Failed;
    let failed = compose("", &saved).unwrap();
    assert!(failed.contains("saved at attachments/1794280014692-1-image.png on the PC (it could not be copied to your computer)"), "{failed}");
}

#[test]
fn files_are_copied_into_a_running_computer_under_the_same_name() {
    let root = temp("copy");
    let computer = FakeSandbox::new();
    computer.use_file_system();
    computer.start().unwrap();
    assert!(is_running(&computer));
    let prepared = prepare(&root, "nimbus", "look", &[png_file("image.png"), text_file("n.txt", "hi there, a note")], 77, Some(&computer)).unwrap();
    assert_eq!(computer.file("/home/pm/attachments/77-1-image.png").as_deref(), Some(&PNG[..]));
    assert_eq!(computer.file("/home/pm/attachments/77-2-n.txt").as_deref(), Some(&b"hi there, a note"[..]));
    assert!(prepared.message.contains("saved on your computer at /home/pm/attachments/77-1-image.png]"));
    assert!(prepared.message.contains("saved on your computer at /home/pm/attachments/77-2-n.txt]"));
    // The file is still on the PC too.
    assert!(root.join("brain/nimbus/attachments/77-1-image.png").exists());
}

#[test]
fn a_computer_that_is_off_is_not_started_and_the_message_still_goes() {
    let root = temp("off");
    let computer = FakeSandbox::new();
    assert!(!is_running(&computer), "a stopped computer is not running");
    // No computer is handed over when it is not running; nothing is written to it.
    let prepared = prepare(&root, "nimbus", "look", &[png_file("image.png")], 5, None).unwrap();
    assert!(prepared.message.contains("saved at attachments/5-1-image.png on the PC (your computer is off)"));
    assert!(computer.calls().is_empty());
    assert!(!is_running(&computer));
}

#[test]
fn a_failed_copy_does_not_stop_the_message_and_the_pictures_still_go() {
    let root = temp("copy-fail");
    let computer = FakeSandbox::new();
    computer.use_file_system();
    computer.start().unwrap();
    computer.fail_next(SandboxError::new("SANDBOX_FILE_WRITE_FAILED", "nope"));
    let prepared = prepare(&root, "nimbus", "look", &[png_file("a.png"), png_file("b.png")], 6, Some(&computer)).unwrap();
    assert_eq!(prepared.message.matches("could not be copied to your computer").count(), 2, "{}", prepared.message);
    assert!(prepared.saved.iter().all(|f| f.guest == Guest::Failed));
    assert_eq!(prepared.images.len(), 2);
    assert!(root.join("brain/nimbus/attachments/6-1-a.png").exists());
}

#[test]
fn only_pictures_become_image_parts_at_most_four() {
    let root = temp("parts");
    let files: Vec<Incoming> = (0..4).map(|i| png_file(&format!("p{i}.png"))).collect();
    let saved = save_all(&root, "nimbus", &files, 8).unwrap();
    assert_eq!(images_of(&root, "nimbus", &saved).len(), 4);
    let mixed = save_all(&root, "nimbus", &[text_file("a.txt", "words"), png_file("b.png")], 9).unwrap();
    let parts = images_of(&root, "nimbus", &mixed);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].mime_type, "image/png");
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(&parts[0].data).unwrap(), PNG.to_vec());
}

#[test]
fn a_message_that_is_too_long_leaves_nothing_saved() {
    let root = temp("toolong");
    let err = prepare(&root, "nimbus", &"w".repeat(MAX_MESSAGE_CHARS), &[png_file("a.png")], 3, None).unwrap_err();
    assert_eq!(err.code, "POTATO_ATTACHMENT");
    let dir = root.join("brain/nimbus/attachments");
    assert_eq!(std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0), 0);
}

// ---- cleanup ---------------------------------------------------------------------------------------------------------

const DAY: i64 = 24 * 60 * 60 * 1000;

#[test]
fn a_file_is_expired_at_exactly_the_keep_time_and_a_future_one_never() {
    assert!(!is_expired(1000, 1000 + 7 * DAY - 1, KEEP_MS));
    assert!(is_expired(1000, 1000 + 7 * DAY, KEEP_MS));
    assert!(is_expired(0, 30 * DAY, KEEP_MS));
    assert!(!is_expired(5 * DAY, DAY, KEEP_MS), "a time in the future is not old");
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn age(path: &std::path::Path, days: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 3600);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

#[test]
fn the_sweep_removes_old_files_directly_in_the_folder_and_nothing_else() {
    let root = temp("sweep");
    let dir = root.join("brain/nimbus/attachments");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    for name in ["old.png", "fresh.png", "sub/deep-old.png"] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    age(&dir.join("old.png"), 8);
    age(&dir.join("sub/deep-old.png"), 30);
    // A file next to the folder, in the bot's own folder, is not an attachment.
    let beside = root.join("brain/nimbus/notes.txt");
    std::fs::write(&beside, b"keep").unwrap();
    age(&beside, 30);
    assert_eq!(sweep_all(&root, now_ms()), 1);
    assert!(!dir.join("old.png").exists());
    assert!(dir.join("fresh.png").exists());
    assert!(dir.join("sub/deep-old.png").exists(), "nothing deeper than the folder itself is touched");
    assert!(beside.exists());
    // A folder that is not there is fine.
    assert_eq!(sweep_dir(&root.join("nope"), now_ms(), KEEP_MS), 0);
}
