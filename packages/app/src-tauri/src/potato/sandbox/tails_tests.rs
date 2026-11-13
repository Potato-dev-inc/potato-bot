//! Cutting long output to its head AND tail, and reading a window of a file.

use super::{cut_chars, parse_slice, FakeSandbox, FileRange, FileSlice, Sandbox};

#[test]
fn short_text_is_left_alone() {
    assert_eq!(cut_chars("hello".to_string(), 10), "hello");
    assert_eq!(cut_chars("x".repeat(10), 10), "x".repeat(10));
}

#[test]
fn long_text_keeps_its_start_and_its_end_with_a_marker_between() {
    let text = format!("START{}END", "m".repeat(1000));
    let out = cut_chars(text, 100);
    assert!(out.starts_with("START"), "{out}");
    assert!(out.ends_with("END"), "{out}");
    assert!(out.contains("bytes cut"), "{out}");
    // 25 head + 75 tail + the marker line.
    assert!(out.len() < 100 + 40, "{}", out.len());
}

#[test]
fn the_marker_counts_the_bytes_that_are_gone() {
    let text = "a".repeat(30) + &"b".repeat(1000) + &"c".repeat(70);
    let out = cut_chars(text, 100);
    // head = 25 (a), tail = 75 (70 c + 5 b); the middle holds 1100 - 100 = 1000 bytes.
    assert!(out.contains("… 1000 bytes cut …"), "{out}");
    assert!(out.ends_with(&("b".repeat(5) + &"c".repeat(70))), "{out}");
}

#[test]
fn cutting_never_splits_a_multibyte_character() {
    let text = "é".repeat(500);
    let out = cut_chars(text, 100);
    assert!(out.starts_with("éé") && out.ends_with("éé"));
    assert!(out.contains("bytes cut"));
}

#[test]
fn the_guest_script_output_splits_into_size_and_bytes() {
    let slice = parse_slice(b"12\nhello\nworld".to_vec()).unwrap();
    assert_eq!(slice, FileSlice { bytes: b"hello\nworld".to_vec(), total: 12 });
    assert_eq!(parse_slice(b"0\n".to_vec()).unwrap().bytes, b"");
    assert!(parse_slice(b"nope\nabc".to_vec()).is_none());
    assert!(parse_slice(b"12".to_vec()).is_none());
}

#[test]
fn a_slice_knows_where_it_starts() {
    let slice = FileSlice { bytes: vec![0; 10], total: 100 };
    assert_eq!(slice.start(FileRange::Tail), 90);
    assert_eq!(slice.start(FileRange::From(40)), 40);
    assert_eq!(slice.start(FileRange::From(500)), 100);
}

#[test]
fn the_fake_reads_the_end_and_a_window_of_a_file() {
    let fake = FakeSandbox::new();
    fake.use_file_system();
    fake.put_file("/home/pm/big.log", b"0123456789");
    let tail = fake.read_file_range("/home/pm/big.log", FileRange::Tail, 4).unwrap();
    assert_eq!((tail.bytes.as_slice(), tail.total), (&b"6789"[..], 10));
    let window = fake.read_file_range("/home/pm/big.log", FileRange::From(2), 3).unwrap();
    assert_eq!(window.bytes, b"234");
    assert_eq!(fake.read_file_range("/home/pm/none", FileRange::Tail, 4).unwrap_err().code, "SANDBOX_NO_FILE");
}
