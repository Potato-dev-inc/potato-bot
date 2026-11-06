//! The owner's site limit for a bot's web tools (`bot_settings::SiteLimit`, enforced in `sandbox_calls.rs`): default off, a plain
//! list of sites that includes their subdomains, enforced on `web_fetch` (and on every redirect it would follow),
//! `browser_navigate` and a web address opened with `computer_act`, set only by the owner's app command, and a damaged
//! settings file reads as the strictest answer. The tests that say what is refused are security tests: never delete them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::bot_settings::{self, clean_site, clean_sites, site_allowed, SiteLimit};
use super::protocol::Trust;
use super::sandbox::lease::ScreenLease;
use super::sandbox::{FakeSandbox, PageHop, Recorded};
use super::sandbox_calls::*;
use super::service_tests::{FakeHost, Rig};

fn limit(sites: &[&str]) -> SiteLimit {
    SiteLimit { enabled: true, sites: sites.iter().map(|s| s.to_string()).collect() }
}

// ---- the list ------------------------------------------------------------------------------------

#[test]
fn a_site_is_typed_in_any_plain_way_and_kept_as_a_host_name() {
    for (typed, kept) in [
        ("example.com", "example.com"),
        ("  Example.COM  ", "example.com"),
        ("https://example.com/some/page?x=1#top", "example.com"),
        ("http://www.example.com", "www.example.com"),
        ("*.example.com", "example.com"),
        ("example.com.", "example.com"),
        ("docs.rs", "docs.rs"),
        ("a-b.example.co.uk", "a-b.example.co.uk"),
        ("yourname.github.io", "yourname.github.io"),
    ] {
        assert_eq!(clean_site(typed).unwrap(), kept, "{typed}");
    }
}

#[test]
fn a_site_that_is_not_a_plain_name_or_covers_a_whole_hosting_service_is_refused_with_a_sentence() {
    for typed in [
        "",
        "   ",
        "localhost",
        "com",
        "example",
        "1.2.3.4",
        "10.0.0.1",
        "example.com:8080",
        "user:pass@example.com",
        "exa mple.com",
        "-bad.example.com",
        "bad-.example.com",
        "a..b.com",
        ".example.com",
        "caf\u{e9}.example.com",
        "github.io",
        "co.uk",
        "vercel.app",
        "amazonaws.com",
    ] {
        let error = clean_site(typed).expect_err(typed);
        assert_eq!(error.code, "POTATO_SITE_INVALID", "{typed:?}");
        assert!(error.message.ends_with('.') || error.message.ends_with(')'), "{typed:?}: {}", error.message);
    }
}

#[test]
fn a_list_drops_blanks_and_repeats_and_holds_fifty_at_most() {
    let list = clean_sites(&["Example.com".into(), "".into(), "https://example.com/x".into(), "docs.rs".into()]).unwrap();
    assert_eq!(list, ["example.com", "docs.rs"]);
    assert_eq!(clean_sites(&["good.com".into(), "nope".into()]).unwrap_err().code, "POTATO_SITE_INVALID", "one bad entry refuses the whole list");
    let many: Vec<String> = (0..=bot_settings::MAX_SITES).map(|n| format!("site{n}.example.com")).collect();
    assert_eq!(clean_sites(&many).unwrap_err().code, "POTATO_SITE_INVALID");
    assert_eq!(clean_sites(&many[..bot_settings::MAX_SITES]).unwrap().len(), bot_settings::MAX_SITES);
}

#[test]
fn security_a_site_includes_its_subdomains_and_nothing_that_only_looks_like_it() {
    let on = limit(&["example.com"]);
    for yes in ["example.com", "www.example.com", "a.b.example.com", "EXAMPLE.com", "example.com."] {
        assert!(site_allowed(&on, yes), "{yes}");
    }
    for no in [
        "notexample.com",
        "example.com.evil.net",
        "example.com.evil.com",
        "evil.com",
        "example.org",
        "examplecom",
        "evil.com/example.com",
        "",
        "com",
        "[::1]",
        "1.2.3.4",
    ] {
        assert!(!site_allowed(&on, no), "{no}");
    }
    assert!(site_allowed(&SiteLimit::default(), "anything.example"), "off: everything is open");
    assert!(site_allowed(&SiteLimit { enabled: false, sites: vec!["example.com".into()] }, "evil.com"), "a list that is switched off limits nothing");
    assert!(!site_allowed(&limit(&[]), "example.com"), "on with an empty list: nothing is open");
}

// ---- saved by the owner's command, and read back -------------------------------------------------

#[test]
fn the_limit_is_off_by_default_and_the_owners_command_saves_it() {
    let rig = Rig::new("sites-save");
    let options = rig.svc.bot_options("nimbus").unwrap();
    assert_eq!(options.site_limit, SiteLimit::default());
    assert!(!options.site_limit.enabled);
    let saved = rig.svc.set_site_limit("nimbus", true, &["https://Example.com/x".into(), "docs.rs".into()]).unwrap();
    assert_eq!(saved.site_limit, limit(&["example.com", "docs.rs"]));
    assert_eq!(rig.svc.bot_options("nimbus").unwrap().site_limit, limit(&["example.com", "docs.rs"]));
    // switching it off keeps the list
    let off = rig.svc.set_site_limit("nimbus", false, &["example.com".into()]).unwrap();
    assert!(!off.site_limit.enabled);
    assert_eq!(off.site_limit.sites, ["example.com"]);
}

#[test]
fn a_bad_site_saves_nothing_and_an_unknown_bot_is_refused() {
    let rig = Rig::new("sites-bad");
    rig.svc.set_site_limit("nimbus", true, &["example.com".into()]).unwrap();
    assert_eq!(rig.svc.set_site_limit("nimbus", true, &["example.com".into(), "1.2.3.4".into()]).unwrap_err().code, "POTATO_SITE_INVALID");
    assert_eq!(rig.svc.bot_options("nimbus").unwrap().site_limit, limit(&["example.com"]), "the old list stays");
    assert_eq!(rig.svc.set_site_limit("ghost", true, &[]).unwrap_err().code, "POTATO_BOT_NOT_FOUND");
}

#[test]
fn security_no_tool_the_bot_can_call_reads_or_writes_the_limit() {
    // The setting is written by the owner's app command only. The brain package does not know it exists, and in the app only
    // the settings file, the engine's setter, the gate in the calls file and the command mention it.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = ["potato/bot_settings.rs", "potato/service.rs", "potato/sandbox_calls.rs", "commands/potato_bot_settings.rs", "lib.rs"];
    fn walk(dir: &std::path::Path, base: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                if rel.ends_with("_tests.rs") || rel.contains("/tests") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let code = text.split("#[cfg(test)]").next().unwrap_or("");
                if code.contains("site_limit") || code.contains("potato_bot_sites_set") {
                    hits.push(rel);
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(&src, &src, &mut hits);
    for hit in &hits {
        assert!(allowed.contains(&hit.as_str()), "{hit} touches the site limit");
    }
    assert!(hits.iter().any(|h| h == "potato/sandbox_calls.rs"), "the scan must see the gate: {hits:?}");
    let brain = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("potato-brain").join("src");
    fn brain_walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                brain_walk(&path, hits);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                let lower = text.to_lowercase();
                if lower.contains("sitelimit") || lower.contains("site_limit") || lower.contains("potato_bot_sites_set") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let mut brain_hits = Vec::new();
    brain_walk(&brain, &mut brain_hits);
    assert!(brain_hits.is_empty(), "the brain package knows the site limit: {brain_hits:?}");
}

// ---- enforced on the web tools -------------------------------------------------------------------

struct Fixture {
    calls: Calls,
    port: Port,
    fake: Arc<FakeSandbox>,
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("pm-potato-sites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let host = Arc::new(FakeHost {
            now: AtomicI64::new(1_000),
            secrets: Mutex::new(HashMap::new()),
            changes: Mutex::new(Vec::new()),
            resources: Mutex::new(None),
            url: Mutex::new(None),
        });
        let fake = Arc::new(FakeSandbox::new());
        let port = Port { sandbox: fake.clone(), lease: Arc::new(Mutex::new(ScreenLease::new())) };
        Fixture { calls: Calls::new(root.clone(), host), port, fake, root }
    }

    /// Another computer and screen over the same settings folder (the screen belongs to one bot at a time).
    fn sibling(&self) -> Fixture {
        let host = Arc::new(FakeHost {
            now: AtomicI64::new(1_000),
            secrets: Mutex::new(HashMap::new()),
            changes: Mutex::new(Vec::new()),
            resources: Mutex::new(None),
            url: Mutex::new(None),
        });
        let fake = Arc::new(FakeSandbox::new());
        let port = Port { sandbox: fake.clone(), lease: Arc::new(Mutex::new(ScreenLease::new())) };
        Fixture { calls: Calls::new(self.root.clone(), host), port, fake, root: self.root.clone() }
    }

    fn limit(&self, bot: &str, enabled: bool, sites: &[&str]) {
        bot_settings::update(&self.root, bot, |s| {
            s.site_limit = SiteLimit { enabled, sites: sites.iter().map(|x| x.to_string()).collect() };
            Ok(())
        })
        .unwrap();
    }

    fn call(&self, bot: &str, tool: &str, args: Value) -> Result<Value, CallError> {
        let ctx = Context { bot: bot.to_string(), conversation: format!("app:dm:{bot}"), trust: Trust::OwnerPc, want_image: false };
        self.calls.execute(Some(&self.port), Gates { bot_on: true, sandbox_on: true }, &ctx, tool, &args)
    }

    fn web_calls(&self) -> usize {
        self.fake.calls().iter().filter(|c| matches!(c, Recorded::Browser(_) | Recorded::FetchPageHop(_) | Recorded::Act(_))).count()
    }
}

fn code(result: Result<Value, CallError>) -> &'static str {
    result.expect_err("the call must be refused").code
}

fn page(body: &str) -> PageHop {
    PageHop { content_type: "text/html".into(), bytes: body.as_bytes().to_vec(), redirect: None }
}

fn redirect(to: &str) -> PageHop {
    PageHop { content_type: String::new(), bytes: Vec::new(), redirect: Some(to.into()) }
}

#[test]
fn with_the_limit_off_every_public_site_opens_as_before() {
    let f = Fixture::new("off");
    f.fake.set_browser_reply("{\"ok\":true}");
    f.limit("nimbus", false, &["example.com"]);
    assert!(f.call("nimbus", "browser_navigate", json!({ "url": "https://anywhere.example.org/" })).is_ok());
    // no settings file at all is the same
    let g = f.sibling();
    g.fake.set_browser_reply("{\"ok\":true}");
    assert!(g.call("fresh", "browser_navigate", json!({ "url": "https://anywhere.example.org/" })).is_ok());
}

#[test]
fn security_browser_navigate_opens_only_listed_sites_and_a_refusal_never_reaches_the_computer() {
    let f = Fixture::new("navigate");
    f.fake.set_browser_reply("{\"ok\":true}");
    f.limit("nimbus", true, &["example.com"]);
    assert!(f.call("nimbus", "browser_navigate", json!({ "url": "https://example.com/a" })).is_ok());
    assert!(f.call("nimbus", "browser_navigate", json!({ "url": "https://www.example.com/a?q=1#x" })).is_ok());
    let before = f.web_calls();
    for url in [
        "https://evil.com/",
        "https://example.com.evil.net/",
        "https://notexample.com/",
        "http://evil.com/#example.com",
        "https://evil.com/?next=https://example.com",
    ] {
        let error = f.call("nimbus", "browser_navigate", json!({ "url": url })).expect_err(url);
        assert_eq!(error.code, "SITE_NOT_ALLOWED", "{url}");
        assert!(error.message.contains("limits this bot to certain sites"), "{}", error.message);
        assert!(error.message.contains("Advanced tab"), "{}", error.message);
    }
    assert_eq!(f.web_calls(), before, "a refused address touched nothing");
    // the limit is this bot's own
    let g = f.sibling();
    g.fake.set_browser_reply("{\"ok\":true}");
    assert!(g.call("other", "browser_navigate", json!({ "url": "https://evil.com/" })).is_ok());
}

#[test]
fn security_a_refused_address_costs_nothing_and_takes_no_screen() {
    let f = Fixture::new("cost");
    f.limit("nimbus", true, &["example.com"]);
    assert_eq!(code(f.call("nimbus", "browser_navigate", json!({ "url": "https://evil.com/" }))), "SITE_NOT_ALLOWED");
    assert!(f.fake.calls().is_empty(), "the sandbox was not even started");
}

#[test]
fn security_web_fetch_is_checked_before_the_first_request_and_at_every_redirect() {
    let f = Fixture::new("fetch");
    f.limit("nimbus", true, &["example.com"]);
    // an unlisted address is refused before any request
    assert_eq!(code(f.call("nimbus", "web_fetch", json!({ "url": "https://evil.com/" }))), "SITE_NOT_ALLOWED");
    assert!(f.fake.calls().is_empty());
    // a listed address is read one step at a time
    f.fake.queue_page_hops(vec![page("hello")]);
    let ok = f.call("nimbus", "web_fetch", json!({ "url": "https://example.com/a" })).unwrap();
    assert_eq!((ok["body"].as_str(), ok["url"].as_str()), (Some("hello"), Some("https://example.com/a")));
    assert_eq!(f.fake.calls(), vec![Recorded::FetchPageHop("https://example.com/a".into())]);
    // a listed site that sends the bot to an unlisted one: the second request is never made
    let g = Fixture::new("fetch-redirect");
    g.limit("nimbus", true, &["example.com"]);
    g.fake.queue_page_hops(vec![redirect("https://evil.com/steal"), page("should never be read")]);
    assert_eq!(code(g.call("nimbus", "web_fetch", json!({ "url": "https://example.com/go" }))), "SITE_NOT_ALLOWED");
    assert_eq!(g.fake.calls(), vec![Recorded::FetchPageHop("https://example.com/go".into())]);
    // a redirect inside the list is followed, and the answer names the page that was read
    let h = Fixture::new("fetch-inside");
    h.limit("nimbus", true, &["example.com"]);
    h.fake.queue_page_hops(vec![redirect("https://www.example.com/next"), page("arrived")]);
    let moved = h.call("nimbus", "web_fetch", json!({ "url": "http://example.com/" })).unwrap();
    assert_eq!((moved["body"].as_str(), moved["url"].as_str()), (Some("arrived"), Some("https://www.example.com/next")));
}

#[test]
fn a_redirect_to_a_private_address_or_a_non_web_address_is_refused_even_inside_the_list() {
    let f = Fixture::new("fetch-private");
    f.limit("nimbus", true, &["example.com"]);
    f.fake.queue_page_hops(vec![redirect("http://127.0.0.1:8080/admin")]);
    assert_eq!(code(f.call("nimbus", "web_fetch", json!({ "url": "https://example.com/" }))), "URL_NOT_ALLOWED");
    f.fake.queue_page_hops(vec![redirect("file:///etc/passwd")]);
    assert_eq!(code(f.call("nimbus", "web_fetch", json!({ "url": "https://example.com/" }))), "URL_NOT_ALLOWED");
}

#[test]
fn a_fetch_that_redirects_for_ever_stops_after_five() {
    let f = Fixture::new("fetch-loop");
    f.limit("nimbus", true, &["example.com"]);
    f.fake.queue_page_hops((0..10).map(|n| redirect(&format!("https://example.com/{n}"))).collect());
    assert_eq!(code(f.call("nimbus", "web_fetch", json!({ "url": "https://example.com/" }))), "SANDBOX_PAGE_REDIRECTS");
    assert_eq!(f.fake.calls().len(), 6, "the first address and five redirects");
}

#[test]
fn security_a_web_address_opened_with_computer_act_follows_the_list_too() {
    let f = Fixture::new("focus");
    f.limit("nimbus", true, &["example.com"]);
    let open = |uri: &str| json!({ "actions": [{ "kind": "focus", "application": "chrome", "uri": uri }] });
    assert_eq!(code(f.call("nimbus", "computer_act", open("https://evil.com/"))), "SITE_NOT_ALLOWED");
    assert!(f.fake.actions().is_empty());
    assert!(f.call("nimbus", "computer_act", open("https://example.com/")).is_ok());
    // an app with no address is not a web address
    assert!(f.call("nimbus", "computer_act", json!({ "actions": [{ "kind": "focus", "application": "terminal" }] })).is_ok());
    // a file path is not a web address either
    assert!(f.call("nimbus", "computer_act", json!({ "actions": [{ "kind": "focus", "application": "editor", "uri": "/home/pm/notes.txt" }] })).is_ok());
}

#[test]
fn security_a_settings_file_that_cannot_be_read_means_no_site_opens() {
    let f = Fixture::new("damaged");
    std::fs::create_dir_all(f.root.join("bot-settings")).unwrap();
    std::fs::write(f.root.join("bot-settings").join("nimbus.json"), "{ damaged").unwrap();
    assert_eq!(code(f.call("nimbus", "web_fetch", json!({ "url": "https://example.com/" }))), "SITE_NOT_ALLOWED");
    assert_eq!(code(f.call("nimbus", "browser_navigate", json!({ "url": "https://example.com/" }))), "SITE_NOT_ALLOWED");
}

#[test]
fn the_limit_is_read_fresh_on_every_call() {
    let f = Fixture::new("fresh");
    f.fake.set_browser_reply("{\"ok\":true}");
    f.limit("nimbus", true, &["example.com"]);
    assert_eq!(code(f.call("nimbus", "browser_navigate", json!({ "url": "https://other.org/" }))), "SITE_NOT_ALLOWED");
    f.limit("nimbus", true, &["example.com", "other.org"]);
    assert!(f.call("nimbus", "browser_navigate", json!({ "url": "https://other.org/" })).is_ok());
    f.limit("nimbus", false, &["example.com"]);
    assert!(f.call("nimbus", "browser_navigate", json!({ "url": "https://third.net/" })).is_ok());
}

#[test]
fn the_real_sandbox_fetches_one_step_at_a_time_and_a_sandbox_that_cannot_says_so() {
    // The default for a sandbox that cannot do a one-step fetch is a refusal, never a quiet fall back to the plain fetch (which
    // follows redirects on its own and would skip the list).
    struct NoHop;
    impl super::sandbox::Sandbox for NoHop {
        fn status(&self) -> super::sandbox::SbxResult<super::sandbox::Status> {
            unreachable!()
        }
        fn install(&self, _: &super::sandbox::install::InstallOptions, _: &mut dyn FnMut(super::sandbox::install::Progress)) -> super::sandbox::SbxResult<()> {
            unreachable!()
        }
        fn start(&self) -> super::sandbox::SbxResult<()> {
            unreachable!()
        }
        fn stop(&self) -> super::sandbox::SbxResult<()> {
            unreachable!()
        }
        fn snapshot(&self, _: &str) -> super::sandbox::SbxResult<()> {
            unreachable!()
        }
        fn reset(&self, _: &str) -> super::sandbox::SbxResult<()> {
            unreachable!()
        }
        fn remove(&self) -> super::sandbox::SbxResult<()> {
            unreachable!()
        }
        fn observe(&self) -> super::sandbox::SbxResult<super::sandbox::Observation> {
            unreachable!()
        }
        fn act(&self, _: &[super::sandbox::control::Action]) -> super::sandbox::SbxResult<super::sandbox::Observation> {
            unreachable!()
        }
        fn browser(&self, _: &[String]) -> super::sandbox::SbxResult<String> {
            unreachable!()
        }
        fn screen_target(&self) -> super::sandbox::SbxResult<super::sandbox::ScreenTarget> {
            unreachable!()
        }
    }
    use super::sandbox::Sandbox;
    let error = NoHop.fetch_page_hop("https://example.com/", 10).err().expect("unsupported by default");
    assert_eq!(error.code, "SANDBOX_WEB_UNSUPPORTED");
}
