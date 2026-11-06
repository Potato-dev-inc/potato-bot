//! The file browser (`files_browser.rs`) against the REAL guest script, run with a POSIX `sh` on a folder of this computer
//! that stands in for the bot's home. The tests that make links, climb with `..` or reach for a hidden folder are the
//! security tests of this feature: they must never be deleted or weakened. They need a `sh` (Git for Windows has one) and
//! permission to make symbolic links (Developer Mode); when either is missing they FAIL with a sentence that says so, they do
//! not skip.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use super::files_browser::*;
use super::sandbox::files_script::{self, FilesOutput};
use super::sandbox::{FakeSandbox, Recorded, Sandbox};

// ---- the harness: the real script, run locally ---------------------------------------------------

/// A POSIX `sh` together with the tools the script uses (realpath, find, head ...) in the same folder. On Windows that is the
/// `usr\bin` folder of Git for Windows: the shell is started with only that folder on its PATH, so Windows' own `find.exe`
/// (a different program) can never be picked up by mistake.
fn sh_dir() -> PathBuf {
    let sh = if cfg!(windows) { "sh.exe" } else { "sh" };
    let tool = if cfg!(windows) { "realpath.exe" } else { "realpath" };
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    folders.extend([r"C:\Program Files\Git\usr\bin", "/usr/bin", "/bin"].iter().map(PathBuf::from));
    for dir in folders {
        if dir.join(sh).is_file() && dir.join(tool).is_file() {
            return dir;
        }
    }
    panic!("these tests run the real guest script and need a POSIX sh with its tools (Git for Windows has them); none was found");
}

fn run_script(home: &str, args: &[String], stdin: Option<Vec<u8>>) -> FilesOutput {
    let dir = sh_dir();
    // The arguments go in through the environment and a `set --`, not on the command line: Git for Windows' shell rewrites a
    // leading ~ on its own command line into the Windows home, which the guest's Linux shell never does.
    let set_args: String = (0..args.len()).map(|n| format!(" \"$PM_TEST_ARG_{n}\"")).collect();
    let text = format!("set --{set_args}\n{}", files_script::script());
    let mut command = Command::new(dir.join(if cfg!(windows) { "sh.exe" } else { "sh" }));
    command.env("PATH", &dir).env("PM_FILES_HOME", home).arg("-c").arg(text).arg("sh");
    for (n, arg) in args.iter().enumerate() {
        command.env(format!("PM_TEST_ARG_{n}"), arg);
    }
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("start sh");
    let mut pipe = child.stdin.take().expect("stdin");
    let writer = std::thread::spawn(move || {
        if let Some(bytes) = stdin {
            let _ = pipe.write_all(&bytes);
        }
    });
    let out = child.wait_with_output().expect("wait for sh");
    let _ = writer.join();
    FilesOutput { code: out.status.code(), stdout: out.stdout }
}

/// A folder of this computer that is the bot's "home", beside a folder outside it that holds a "secret".
struct Rig {
    base: PathBuf,
    home: PathBuf,
    outside: PathBuf,
    sandbox: FakeSandbox,
}

/// The path as the POSIX shell on this computer writes it (`/c/Users/...` for the Git for Windows shell), so the shell's own
/// answers and the paths given to it are in one spelling. On Linux it is the path itself.
fn slashes(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    // Git for Windows' shell mounts the temp folder as /tmp and prints links that lead into it that way, so the tests say it too.
    let temp = std::env::temp_dir().to_string_lossy().replace('\\', "/");
    if cfg!(windows) && text.starts_with(temp.trim_end_matches('/')) {
        return format!("/tmp{}", &text[temp.trim_end_matches('/').len()..]);
    }
    if cfg!(windows) && text.len() >= 2 && text.as_bytes()[1] == b':' {
        format!("/{}{}", text[..1].to_lowercase(), &text[2..])
    } else {
        text
    }
}

impl Rig {
    fn new(name: &str) -> Rig {
        let base = std::env::temp_dir().join(format!("pm-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let outside = base.join("outside");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "tool secret").unwrap();
        let sandbox = FakeSandbox::new();
        let home_text = slashes(&home);
        sandbox.set_files_hook(Arc::new(move |args, stdin| run_script(&home_text, args, stdin)));
        Rig { base, home, outside, sandbox }
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        let path = self.home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn dir(&self, rel: &str) {
        std::fs::create_dir_all(self.home.join(rel)).unwrap();
    }

    fn link_dir(&self, rel: &str, target: &Path) {
        let link = self.home.join(rel);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        make_dir_link(target, &link);
    }

    fn link_file(&self, rel: &str, target: &Path) {
        let link = self.home.join(rel);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        make_file_link(target, &link);
    }

    fn abs(&self, rel: &str) -> String {
        format!("{}/{rel}", slashes(&self.home))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Links first, so a cleanup can never walk through one.
        fn unlink(dir: &Path) {
            if let Ok(read) = std::fs::read_dir(dir) {
                for entry in read.flatten() {
                    let path = entry.path();
                    let meta = std::fs::symlink_metadata(&path);
                    if meta.as_ref().is_ok_and(|m| m.file_type().is_symlink()) {
                        let _ = std::fs::remove_dir(&path).or_else(|_| std::fs::remove_file(&path));
                    } else if path.is_dir() {
                        unlink(&path);
                    }
                }
            }
        }
        unlink(&self.base);
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[cfg(windows)]
fn make_dir_link(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(target, link).expect("these tests make symbolic links: turn on Developer Mode in Windows");
}
#[cfg(windows)]
fn make_file_link(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("these tests make symbolic links: turn on Developer Mode in Windows");
}
#[cfg(unix)]
fn make_dir_link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}
#[cfg(unix)]
fn make_file_link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

fn code_of<T: std::fmt::Debug>(result: Result<T, super::store::StoreError>) -> &'static str {
    result.expect_err("this must be refused").code
}

fn names(listing: &Listing) -> Vec<&str> {
    listing.entries.iter().map(|e| e.name.as_str()).collect()
}

// ---- browsing ------------------------------------------------------------------------------------

#[test]
fn home_lists_folders_first_and_hides_top_level_dot_folders() {
    let rig = Rig::new("list");
    rig.put("notes.txt", b"hi");
    rig.put("apple.txt", b"hi");
    rig.put("docs/plan.md", b"# plan");
    rig.dir("Zed");
    rig.put(".ssh/id_ed25519", b"key");
    rig.put(".pm-sbx/vnc/control.plain", b"password");
    rig.put(".bashrc", b"x");
    let listing = list(&rig.sandbox, "").unwrap();
    assert_eq!(listing.path, "~");
    assert_eq!(names(&listing), ["docs", "Zed", "apple.txt", "notes.txt"], "folders first, then files, without regard to case; nothing hidden");
    assert!(!listing.truncated);
    let docs = &listing.entries[0];
    assert_eq!(docs.kind, EntryKind::Dir);
    assert!(!docs.link);
    let notes = listing.entries.iter().find(|e| e.name == "notes.txt").unwrap();
    assert_eq!((notes.kind, notes.size), (EntryKind::File, 2));
    assert!(notes.modified_ms > 1_500_000_000_000, "the modified time is in milliseconds");
}

#[test]
fn a_folder_below_home_shows_its_dot_files_and_its_path() {
    let rig = Rig::new("sub");
    rig.put("project/.env", b"A=1");
    rig.put("project/src/main.rs", b"fn main() {}");
    let listing = list(&rig.sandbox, "project").unwrap();
    assert_eq!(listing.path, "~/project");
    assert_eq!(names(&listing), ["src", ".env"]);
    // the same folder by its other spellings
    assert_eq!(list(&rig.sandbox, "~/project/").unwrap().path, "~/project");
    assert_eq!(list(&rig.sandbox, &rig.abs("project")).unwrap().path, "~/project");
    assert_eq!(list(&rig.sandbox, "~").unwrap().path, "~");
    assert_eq!(list(&rig.sandbox, "   ").unwrap().path, "~", "an empty path is home");
}

#[test]
fn the_upload_temp_name_is_never_listed() {
    let rig = Rig::new("temp");
    rig.put("docs/.pm-upload.123", b"half");
    rig.put("docs/a.txt", b"a");
    assert_eq!(names(&list(&rig.sandbox, "docs").unwrap()), ["a.txt"]);
}

#[test]
fn a_missing_path_and_a_file_given_as_a_folder_are_plain_errors() {
    let rig = Rig::new("missing");
    rig.put("a.txt", b"a");
    assert_eq!(code_of(list(&rig.sandbox, "nope")), "POTATO_FILES_NOT_FOUND");
    assert_eq!(code_of(list(&rig.sandbox, "a.txt")), "POTATO_FILES_NOT_FOLDER");
}

#[test]
fn a_big_folder_is_cut_at_2000_entries_and_says_so() {
    let rig = Rig::new("big");
    rig.dir("many");
    for n in 0..(MAX_LIST + 5) {
        std::fs::write(rig.home.join("many").join(format!("f{n:04}")), b"").unwrap();
    }
    let listing = list(&rig.sandbox, "many").unwrap();
    assert_eq!(listing.entries.len(), MAX_LIST);
    assert!(listing.truncated);
}

#[test]
fn names_with_spaces_and_odd_letters_come_through_whole() {
    let rig = Rig::new("odd");
    rig.put("caf\u{e9} \u{65e5}\u{672c} 1.md", b"2");
    let listing = list(&rig.sandbox, "").unwrap();
    assert_eq!(names(&listing), ["caf\u{e9} \u{65e5}\u{672c} 1.md"]);
}

#[test]
fn a_name_with_a_tab_in_it_is_read_whole_from_the_scripts_records() {
    // Windows cannot make such a file, so the records the script prints are played back by hand.
    let mut bytes = b"P\tdocs\0".to_vec();
    bytes.extend_from_slice(b"f\tf\t3\t1700000000.5\ttab\tname.txt\0");
    bytes.extend_from_slice(b"l\td\t0\t1700000001.0\tshortcut\0");
    bytes.extend_from_slice(b"f\tf\t0\t1700000002.0\t.pm-upload.77\0");
    bytes.extend_from_slice(b"junk without tabs\0");
    let fake = FakeSandbox::new();
    fake.set_files_hook(Arc::new(move |_, _| FilesOutput { code: Some(0), stdout: bytes.clone() }));
    let listing = list(&fake, "docs").unwrap();
    assert_eq!(listing.path, "~/docs");
    assert_eq!(names(&listing), ["shortcut", "tab\tname.txt"]);
    assert!(listing.entries[0].link && listing.entries[0].kind == EntryKind::Dir);
    assert_eq!(listing.entries[1].modified_ms, 1_700_000_000_500);
}

// ---- security: climbing out, links, hidden folders ------------------------------------------------

#[test]
fn security_dot_dot_and_outside_paths_are_refused_for_every_call() {
    let rig = Rig::new("traversal");
    rig.put("docs/a.txt", b"a");
    let sibling_dir = PathBuf::from(format!("{}-evil", rig.home.display()));
    std::fs::create_dir_all(&sibling_dir).unwrap();
    std::fs::write(sibling_dir.join("secret.txt"), "x").unwrap();
    let sibling = slashes(&sibling_dir);
    let outside_secret = format!("{}/secret.txt", slashes(&rig.outside));
    for bad in [
        "..",
        "../outside",
        "../outside/secret.txt",
        "docs/../../outside/secret.txt",
        "docs/..",
        "~/../outside",
        "./../outside",
        "docs/../../../etc/passwd",
    ] {
        assert_eq!(code_of(list(&rig.sandbox, bad)), "POTATO_FILES_INVALID", "list {bad}");
        assert_eq!(code_of(preview(&rig.sandbox, bad)), "POTATO_FILES_INVALID", "preview {bad}");
        assert_eq!(code_of(download(&rig.sandbox, bad)), "POTATO_FILES_INVALID", "download {bad}");
        assert_eq!(code_of(upload(&rig.sandbox, bad, "x.txt", b"x".to_vec())), "POTATO_FILES_INVALID", "upload into {bad}");
    }
    for outside in [
        outside_secret.as_str(),
        "/etc/passwd",
        "/opt/pm-connectors/github/token",
        "/run/pm-sbx/control.token",
        "/var/lib/pm-sbx/mcp.json",
        "/",
        &sibling,
        &format!("{sibling}/secret.txt"),
    ] {
        assert_eq!(code_of(list(&rig.sandbox, outside)), "POTATO_FILES_OUTSIDE", "list {outside}");
        assert_eq!(code_of(preview(&rig.sandbox, outside)), "POTATO_FILES_OUTSIDE", "preview {outside}");
        assert_eq!(code_of(download(&rig.sandbox, outside)), "POTATO_FILES_OUTSIDE", "download {outside}");
        assert_eq!(code_of(upload(&rig.sandbox, outside, "x.txt", b"x".to_vec())), "POTATO_FILES_OUTSIDE", "upload into {outside}");
    }
    assert!(!rig.outside.join("x.txt").exists());
    assert!(!sibling_dir.join("x.txt").exists());
    let _ = std::fs::remove_dir_all(&sibling_dir);
}

#[test]
fn security_an_outside_path_looks_the_same_whether_or_not_it_exists() {
    let rig = Rig::new("oracle");
    let present = format!("{}/secret.txt", slashes(&rig.outside));
    let absent = format!("{}/nothing-here.txt", slashes(&rig.outside));
    assert_eq!(code_of(preview(&rig.sandbox, &present)), code_of(preview(&rig.sandbox, &absent)));
    assert_eq!(code_of(list(&rig.sandbox, "/opt/pm-connectors")), code_of(list(&rig.sandbox, "/opt/does-not-exist")));
}

#[test]
fn security_a_link_that_leads_out_of_home_is_refused_for_every_call() {
    let rig = Rig::new("links");
    rig.link_dir("out_dir", &rig.outside);
    rig.put("docs/a.txt", b"a");
    rig.link_file("docs/out_file", &rig.outside.join("secret.txt"));
    // a chain: first -> second -> outside
    rig.link_dir("second", &rig.outside);
    rig.link_dir("first", &rig.home.join("second"));
    // a link to a link that sits in the middle of a longer path
    for path in ["out_dir", "out_dir/secret.txt", "docs/out_file", "first", "first/secret.txt", "second/secret.txt"] {
        assert!(code_of(list(&rig.sandbox, path)).starts_with("POTATO_FILES_"), "list {path}");
        let preview_code = code_of(preview(&rig.sandbox, path));
        assert_eq!(preview_code, "POTATO_FILES_OUTSIDE", "preview {path}");
        assert_eq!(code_of(download(&rig.sandbox, path)), "POTATO_FILES_OUTSIDE", "download {path}");
    }
    assert_eq!(code_of(list(&rig.sandbox, "out_dir")), "POTATO_FILES_OUTSIDE");
    assert_eq!(code_of(list(&rig.sandbox, "first")), "POTATO_FILES_OUTSIDE");
    assert_eq!(code_of(upload(&rig.sandbox, "out_dir", "planted.txt", b"x".to_vec())), "POTATO_FILES_OUTSIDE");
    assert_eq!(code_of(upload(&rig.sandbox, "first", "planted.txt", b"x".to_vec())), "POTATO_FILES_OUTSIDE");
    assert!(!rig.outside.join("planted.txt").exists(), "nothing was written outside home");
    // the links are still listed, marked as links, so the person sees what they are
    let home = list(&rig.sandbox, "").unwrap();
    assert!(home.entries.iter().any(|e| e.name == "out_dir" && e.link));
}

#[test]
fn security_a_link_to_a_hidden_folder_of_home_is_refused() {
    let rig = Rig::new("hidden-link");
    rig.put(".pm-sbx/vnc/control.plain", b"the control password");
    rig.put(".browser-profiles/chromium-bot-1/Cookies", b"cookies");
    rig.link_dir("shortcut", &rig.home.join(".pm-sbx"));
    rig.link_file("docs/pw", &rig.home.join(".pm-sbx/vnc/control.plain"));
    rig.link_dir("profiles", &rig.home.join(".browser-profiles"));
    for path in ["shortcut", "shortcut/vnc", "shortcut/vnc/control.plain", "docs/pw", "profiles", "profiles/chromium-bot-1/Cookies"] {
        let code = code_of(preview(&rig.sandbox, path));
        assert_eq!(code, "POTATO_FILES_OUTSIDE", "preview {path}");
        assert_eq!(code_of(download(&rig.sandbox, path)), "POTATO_FILES_OUTSIDE", "download {path}");
    }
    assert_eq!(code_of(list(&rig.sandbox, "shortcut")), "POTATO_FILES_OUTSIDE");
    assert_eq!(code_of(upload(&rig.sandbox, "shortcut", "x.txt", b"x".to_vec())), "POTATO_FILES_OUTSIDE");
}

#[test]
fn security_hidden_top_level_folders_are_refused_by_name_too() {
    let rig = Rig::new("hidden");
    rig.put(".pm-sbx/vnc/control.plain", b"the control password");
    rig.put(".pm-sbx/logs/x11vnc.log", b"log");
    rig.put(".ssh/id_ed25519", b"key");
    rig.put(".config/gh/hosts.yml", b"token");
    rig.put(".bashrc", b"alias");
    for path in [".pm-sbx", ".pm-sbx/vnc/control.plain", ".pm-sbx/logs", ".ssh", ".ssh/id_ed25519", ".config/gh/hosts.yml", ".bashrc", "~/.pm-sbx"] {
        assert_eq!(code_of(list(&rig.sandbox, path)), "POTATO_FILES_OUTSIDE", "list {path}");
        assert_eq!(code_of(preview(&rig.sandbox, path)), "POTATO_FILES_OUTSIDE", "preview {path}");
        assert_eq!(code_of(download(&rig.sandbox, path)), "POTATO_FILES_OUTSIDE", "download {path}");
    }
    let absolute = rig.abs(".pm-sbx/vnc/control.plain");
    assert_eq!(code_of(preview(&rig.sandbox, &absolute)), "POTATO_FILES_OUTSIDE");
    // a dot name is allowed deeper down
    rig.put("project/.env", b"A=1");
    assert_eq!(preview(&rig.sandbox, "project/.env").unwrap().text.as_deref(), Some("A=1"));
    // nothing can be uploaded into a hidden folder, or as a hidden name in home
    assert_eq!(code_of(upload(&rig.sandbox, ".ssh", "authorized_keys", b"k".to_vec())), "POTATO_FILES_OUTSIDE");
    assert_eq!(code_of(upload(&rig.sandbox, "", ".bashrc", b"evil".to_vec())), "POTATO_FILES_OUTSIDE");
    assert_eq!(std::fs::read(rig.home.join(".bashrc")).unwrap(), b"alias");
    assert!(!rig.home.join(".ssh/authorized_keys").exists());
}

#[test]
fn a_link_inside_home_that_stays_inside_works() {
    let rig = Rig::new("inside");
    rig.put("docs/a.txt", b"inside text");
    rig.link_dir("shortcut", &rig.home.join("docs"));
    rig.link_file("alias.txt", &rig.home.join("docs/a.txt"));
    assert_eq!(names(&list(&rig.sandbox, "shortcut").unwrap()), ["a.txt"]);
    assert_eq!(list(&rig.sandbox, "shortcut").unwrap().path, "~/docs", "a link is shown at the place it really is");
    assert_eq!(preview(&rig.sandbox, "alias.txt").unwrap().text.as_deref(), Some("inside text"));
    let home = list(&rig.sandbox, "").unwrap();
    let alias = home.entries.iter().find(|e| e.name == "alias.txt").unwrap();
    assert!(alias.link);
    assert_eq!(alias.kind, EntryKind::File);
    let shortcut = home.entries.iter().find(|e| e.name == "shortcut").unwrap();
    assert!(shortcut.link);
    assert_eq!(shortcut.kind, EntryKind::Dir);
}

#[test]
fn security_the_script_is_the_only_gate_and_it_is_fixed() {
    // The checks are in the script text the computer runs; this keeps the pieces from being edited out unnoticed.
    let script = files_script::script();
    assert!(!script.contains('\r'));
    for needle in [
        "realpath -m",
        "case \"/$1/\" in */../*) exit 11",
        "case $c in \"$HOME_ROOT\" | \"$HOME_ROOT\"/*) ;; *) exit 5",
        "\"$HOME_REAL\"/*) REL=${R#\"$HOME_REAL\"/} ;;",
        "case $REL in .*) exit 5",
        "ln -- \"$tmp\" \"$dest\"",
        "! -name '.*'",
    ] {
        assert!(script.contains(needle), "the script lost: {needle}");
    }
}

// ---- preview and download ------------------------------------------------------------------------

#[test]
fn a_text_file_shows_its_start_and_says_when_it_was_cut() {
    let rig = Rig::new("text");
    rig.put("small.txt", "hello \u{e9}\nworld".as_bytes());
    let small = preview(&rig.sandbox, "small.txt").unwrap();
    assert_eq!((small.kind, small.text.as_deref(), small.truncated, small.size), (PreviewKind::Text, Some("hello \u{e9}\nworld"), false, 14));
    assert_eq!(small.name, "small.txt");
    let big = "a".repeat(MAX_TEXT_PREVIEW + 1000);
    rig.put("big.log", big.as_bytes());
    let cut = preview(&rig.sandbox, "big.log").unwrap();
    assert_eq!(cut.text.as_deref().map(str::len), Some(MAX_TEXT_PREVIEW));
    assert!(cut.truncated);
    assert_eq!(cut.size as usize, big.len());
    // a file with no extension is read as text when it is text
    rig.put("Makefile", b"all:\n\techo hi\n");
    assert_eq!(preview(&rig.sandbox, "Makefile").unwrap().kind, PreviewKind::Text);
    // an SVG or an HTML file is only ever shown as text, never drawn
    rig.put("logo.svg", b"<svg onload=alert(1)/>");
    let svg = preview(&rig.sandbox, "logo.svg").unwrap();
    assert_eq!((svg.kind, svg.image), (PreviewKind::Text, None));
}

#[test]
fn a_small_picture_comes_as_a_data_address_and_a_big_one_does_not() {
    let rig = Rig::new("image");
    rig.put("tiny.png", &[0x89, b'P', b'N', b'G', 1, 2, 3]);
    let shown = preview(&rig.sandbox, "tiny.png").unwrap();
    assert_eq!(shown.kind, PreviewKind::Image);
    assert!(shown.image.as_deref().unwrap().starts_with("data:image/png;base64,iVBORw"), "{:?}", shown.image);
    rig.put("huge.jpg", &vec![0xff; MAX_IMAGE_PREVIEW + 1]);
    let huge = preview(&rig.sandbox, "huge.jpg").unwrap();
    assert_eq!((huge.kind, huge.image), (PreviewKind::Other, None));
}

#[test]
fn a_binary_file_a_pdf_and_a_folder_have_no_text_preview() {
    let rig = Rig::new("binary");
    rig.put("tool.bin", &[0x7f, b'E', b'L', b'F', 0, 0, 1]);
    rig.put("paper.pdf", b"%PDF-1.4 text");
    rig.dir("docs");
    assert_eq!(preview(&rig.sandbox, "tool.bin").unwrap().kind, PreviewKind::Other);
    assert_eq!(preview(&rig.sandbox, "paper.pdf").unwrap().kind, PreviewKind::Other);
    assert_eq!(code_of(preview(&rig.sandbox, "docs")), "POTATO_FILES_NOT_FILE");
    assert_eq!(code_of(preview(&rig.sandbox, "ghost.txt")), "POTATO_FILES_NOT_FOUND");
}

#[test]
fn a_download_gives_the_exact_bytes_and_refuses_folders_and_big_files() {
    let rig = Rig::new("download");
    let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    rig.put("docs/data.bin", &bytes);
    let (name, got) = download(&rig.sandbox, "docs/data.bin").unwrap();
    assert_eq!(name, "data.bin");
    assert_eq!(got, bytes, "binary bytes, NUL and all, come through untouched");
    rig.put("exactly.bin", &vec![7u8; MAX_TRANSFER]);
    assert_eq!(download(&rig.sandbox, "exactly.bin").unwrap().1.len(), MAX_TRANSFER);
    rig.put("toobig.bin", &vec![7u8; MAX_TRANSFER + 1]);
    assert_eq!(code_of(download(&rig.sandbox, "toobig.bin")), "POTATO_FILES_TOO_BIG");
    assert_eq!(code_of(download(&rig.sandbox, "docs")), "POTATO_FILES_NOT_FILE");
    assert_eq!(code_of(download(&rig.sandbox, "ghost.bin")), "POTATO_FILES_NOT_FOUND");
}

// ---- upload --------------------------------------------------------------------------------------

#[test]
fn an_upload_lands_in_the_folder_and_never_replaces_a_file() {
    let rig = Rig::new("upload");
    rig.dir("inbox");
    let at = upload(&rig.sandbox, "inbox", "report.csv", b"a,b\n1,2\n".to_vec()).unwrap();
    assert_eq!(at, "~/inbox/report.csv");
    assert_eq!(std::fs::read(rig.home.join("inbox/report.csv")).unwrap(), b"a,b\n1,2\n");
    let again = upload(&rig.sandbox, "inbox", "report.csv", b"REPLACED".to_vec());
    assert_eq!(code_of(again), "POTATO_FILES_EXISTS");
    assert_eq!(std::fs::read(rig.home.join("inbox/report.csv")).unwrap(), b"a,b\n1,2\n", "the first file is untouched");
    assert_eq!(upload(&rig.sandbox, "", "top.txt", b"t".to_vec()).unwrap(), "~/top.txt");
    assert_eq!(upload(&rig.sandbox, "~/inbox", "bytes.bin", vec![0, 1, 2, 255]).unwrap(), "~/inbox/bytes.bin");
    assert_eq!(std::fs::read(rig.home.join("inbox/bytes.bin")).unwrap(), [0, 1, 2, 255]);
    let left: Vec<_> = std::fs::read_dir(rig.home.join("inbox")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(left.iter().all(|n| !n.starts_with(".pm-upload")), "no temp file is left: {left:?}");
}

#[test]
fn security_an_upload_name_cannot_carry_a_path() {
    let rig = Rig::new("upload-names");
    rig.dir("inbox");
    // the app cleans the name down to its last part ...
    assert_eq!(upload(&rig.sandbox, "inbox", "../../escape.txt", b"x".to_vec()).unwrap(), "~/inbox/escape.txt");
    assert_eq!(upload(&rig.sandbox, "inbox", "C:\\Users\\me\\Desktop\\win.txt", b"x".to_vec()).unwrap(), "~/inbox/win.txt");
    assert!(!rig.home.join("escape.txt").exists());
    // ... refuses names that are nothing ...
    for bad in ["", ".", "..", "...", "a\nb.txt", "bell\u{7}.txt", "  "] {
        assert_eq!(code_of(upload(&rig.sandbox, "inbox", bad, b"x".to_vec())), "POTATO_FILES_BAD_NAME", "{bad:?}");
    }
    assert_eq!(code_of(upload(&rig.sandbox, "inbox", &"n".repeat(300), b"x".to_vec())), "POTATO_FILES_BAD_NAME");
    // ... and the script refuses them again, in case the app's check were ever skipped
    for bad in ["a/b", "../x", "..", ".", "", "/abs"] {
        let out = rig.sandbox.files_run(&["put".into(), "inbox".into(), bad.into(), "100".into()], Some(b"x".to_vec())).unwrap();
        assert_eq!(out.code, Some(12), "the script took the name {bad:?}");
    }
    assert!(!rig.home.join("x").exists() && !rig.home.join("abs").exists());
}

#[test]
fn an_upload_into_a_file_or_a_missing_folder_is_refused_and_makes_no_folder() {
    let rig = Rig::new("upload-where");
    rig.put("a.txt", b"a");
    assert_eq!(code_of(upload(&rig.sandbox, "a.txt", "b.txt", b"b".to_vec())), "POTATO_FILES_NOT_FOLDER");
    assert_eq!(code_of(upload(&rig.sandbox, "nope", "b.txt", b"b".to_vec())), "POTATO_FILES_NOT_FOUND");
    assert!(!rig.home.join("nope").exists());
}

#[test]
fn an_upload_over_10_mb_is_refused_before_it_is_sent_and_by_the_script_too() {
    let rig = Rig::new("upload-big");
    rig.dir("inbox");
    assert_eq!(code_of(upload(&rig.sandbox, "inbox", "big.bin", vec![1u8; MAX_TRANSFER + 1])), "POTATO_FILES_TOO_BIG");
    assert!(rig.sandbox.calls().iter().all(|c| !matches!(c, Recorded::FilesRun(args) if args.first().map(String::as_str) == Some("put"))), "the app did not even send it");
    // the script's own limit, with a small max
    let out = rig.sandbox.files_run(&["put".into(), "inbox".into(), "small.bin".into(), "10".into()], Some(vec![1u8; 11])).unwrap();
    assert_eq!(out.code, Some(4));
    assert!(!rig.home.join("inbox/small.bin").exists());
    assert_eq!(upload(&rig.sandbox, "inbox", "exactly.bin", vec![1u8; MAX_TRANSFER]).unwrap(), "~/inbox/exactly.bin");
}

// ---- the plain checks of the app's own -----------------------------------------------------------

fn canned(code: Option<i32>) -> FakeSandbox {
    let fake = FakeSandbox::new();
    fake.set_files_hook(Arc::new(move |_, _| FilesOutput { code, stdout: Vec::new() }));
    fake
}

#[test]
fn every_exit_code_of_the_script_has_a_plain_sentence() {
    let cases: [(Option<i32>, &str); 12] = [
        (None, "POTATO_FILES_TIMEOUT"),
        (Some(2), "POTATO_FILES_NOT_FOUND"),
        (Some(3), "POTATO_FILES_NOT_FOLDER"),
        (Some(4), "POTATO_FILES_TOO_BIG"),
        (Some(5), "POTATO_FILES_OUTSIDE"),
        (Some(6), "POTATO_FILES_EXISTS"),
        (Some(7), "POTATO_FILES_NOT_FILE"),
        (Some(8), "POTATO_FILES_WRITE_FAILED"),
        (Some(11), "POTATO_FILES_INVALID"),
        (Some(12), "POTATO_FILES_BAD_NAME"),
        (Some(20), "POTATO_FILES_NO_HOME"),
        (Some(99), "POTATO_FILES_INVALID"),
    ];
    for (exit, want) in cases {
        let error = list(&canned(exit), "docs").expect_err("a failed run is an error");
        assert_eq!(error.code, want, "exit {exit:?}");
        assert!(error.message.ends_with('.') && !error.message.contains("exit"), "{}", error.message);
    }
}

#[test]
fn a_path_with_control_characters_never_reaches_the_computer() {
    let fake = canned(Some(0));
    for bad in ["a\nb", "a\u{0}b", "tab\there"] {
        assert_eq!(code_of(list(&fake, bad)), "POTATO_FILES_INVALID", "{bad:?}");
    }
    assert_eq!(code_of(list(&fake, &"x".repeat(5000))), "POTATO_FILES_INVALID");
    assert!(fake.calls().is_empty());
}

#[test]
fn the_app_asks_the_computer_only_through_the_files_script() {
    let rig = Rig::new("door");
    rig.put("a.txt", b"a");
    let _ = list(&rig.sandbox, "");
    let _ = preview(&rig.sandbox, "a.txt");
    let _ = download(&rig.sandbox, "a.txt");
    let _ = upload(&rig.sandbox, "", "b.txt", b"b".to_vec());
    let calls = rig.sandbox.calls();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|c| matches!(c, Recorded::FilesRun(_))), "{calls:?}");
}

#[test]
fn a_computer_that_cannot_browse_says_so() {
    let fake = FakeSandbox::new();
    assert_eq!(code_of(list(&fake, "")), "SANDBOX_FILES_UNSUPPORTED");
}

#[test]
fn a_download_is_saved_only_to_a_real_place_the_person_picked() {
    let dir = std::env::temp_dir().join(format!("pm-files-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    save_download(&dir.join("out.bin"), &[1, 2, 3]).unwrap();
    assert_eq!(std::fs::read(dir.join("out.bin")).unwrap(), [1, 2, 3]);
    assert_eq!(code_of(save_download(Path::new("out.bin"), &[1])), "POTATO_FILES_BAD_DEST", "a relative path is not a pick");
    assert_eq!(code_of(save_download(&dir, &[1])), "POTATO_FILES_BAD_DEST", "a folder is not a file");
    assert_eq!(code_of(save_download(&dir.join("missing").join("out.bin"), &[1])), "POTATO_FILES_BAD_DEST");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_upload_source_must_be_one_real_file_of_a_fair_size() {
    let dir = std::env::temp_dir().join(format!("pm-files-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("photo one.png"), [1, 2, 3]).unwrap();
    assert_eq!(read_upload(&dir.join("photo one.png")).unwrap(), ("photo one.png".to_string(), vec![1, 2, 3]));
    assert_eq!(code_of(read_upload(&dir)), "POTATO_FILES_BAD_SOURCE", "a folder is not one file");
    assert_eq!(code_of(read_upload(Path::new("relative.txt"))), "POTATO_FILES_BAD_SOURCE");
    assert_eq!(code_of(read_upload(&dir.join("ghost.txt"))), "POTATO_FILES_BAD_SOURCE");
    std::fs::write(dir.join("huge.bin"), vec![0u8; MAX_TRANSFER + 1]).unwrap();
    assert_eq!(code_of(read_upload(&dir.join("huge.bin"))), "POTATO_FILES_TOO_BIG");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_browser_and_its_commands_never_use_the_bots_file_tools() {
    // The person's browser has its own door; it must not borrow the bot's tools (which are not confined to home).
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["potato/files_browser.rs", "commands/potato_files.rs"] {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        let code: String = text.split("#[cfg(test)]").next().unwrap().lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        for banned in [".read_file(", ".write_file(", ".fetch_file(", ".shell(", ".root_shell(", "mcp_call", "secret_config", "/opt/", "/run/"] {
            assert!(!code.contains(banned), "{file} uses {banned}");
        }
    }
}
