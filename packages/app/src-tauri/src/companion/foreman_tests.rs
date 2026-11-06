//! The foreman side of a companion: it finds what was said (`companion_recall`) and it hires helpers.
//! On the fake host: no terminal, no index file, no thread. The real index is tested in `chat_index`, the real
//! route in `bridge/routes/companion_mcp.rs`, the budget in `budgets`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::caller::{decide, CompanionPane};
use super::model::FeedKind;
use super::store;
use super::test_host::{draft, start_ms, temp_dir, FakeHost};
use super::Service;
use crate::pty::agent_proc::ProcEntry;

const MOCHI: &str = "companion-mochi";
const BIT: &str = "companion-bit";

fn two_companions() -> (Service, Arc<FakeHost>, std::path::PathBuf) {
    let root = temp_dir("pm-companion-foreman");
    let host = FakeHost::new(start_ms());
    host.recall_enabled.store(true, Ordering::SeqCst);
    let service = Service::new(host.clone(), root.clone());
    service.create(draft("Mochi", &root.join("mochi"))).unwrap();
    service.create(draft("Bit", &root.join("bit"))).unwrap();
    (service, host, root)
}

#[test]
fn a_new_companion_saves_its_pane_id_in_its_profile() {
    let (service, _host, root) = two_companions();
    let saved = store::load(&root).unwrap();
    let mochi = saved.iter().find(|p| p.id == "mochi").unwrap();
    assert_eq!(mochi.pane_ids, vec![MOCHI.to_string()]);
    let _ = service;
}

// ---- the brief ----------------------------------------------------------------------------------

fn brief_of(root: &std::path::Path, id: &str) -> String {
    std::fs::read_to_string(root.join(id).join("AGENTS.md")).unwrap()
}

#[test]
fn the_brief_tells_about_recall_only_while_the_index_is_on() {
    let (_service, _host, root) = two_companions();
    let on = brief_of(&root, "mochi");
    assert!(on.contains("companion_recall"));
    assert!(on.contains("run_agent"), "the hiring rule is always there");

    let root2 = temp_dir("pm-companion-foreman-off");
    let host2 = FakeHost::new(start_ms());
    let service2 = Service::new(host2, root2.clone());
    service2.create(draft("Mochi", &root2.join("mochi"))).unwrap();
    let off = brief_of(&root2, "mochi");
    assert!(!off.contains("companion_recall"));
    assert!(off.contains("run_agent"));
    assert!(off.contains("only they can answer it"));
}

// ---- a companion hires helpers -------------------------------------------------------------------

#[test]
fn a_started_helper_becomes_a_delegated_line_in_the_feed() {
    let (service, host, root) = two_companions();
    service.note_delegated(MOCHI, "Codex", "demo-a");
    let feed = store::feed_read(&root, "mochi").unwrap();
    let line = feed.iter().find(|item| item.kind == FeedKind::Delegated).expect("a delegated line");
    assert_eq!(line.text, "Mochi started a Codex helper in demo-a.");
    assert_eq!(line.at_ms, host.now());
    // Only that companion's feed.
    assert!(store::feed_read(&root, "bit").unwrap().iter().all(|item| item.kind != FeedKind::Delegated));
    // An unknown pane writes nothing, and a missing folder name is not printed as "in .".
    service.note_delegated("5f3c-ordinary-pane", "Codex", "demo-a");
    service.note_delegated(BIT, "Codex", "  ");
    let bit = store::feed_read(&root, "bit").unwrap();
    assert_eq!(bit.iter().find(|item| item.kind == FeedKind::Delegated).unwrap().text, "Bit started a Codex helper.");
    assert_eq!(store::feed_read(&root, "mochi").unwrap().iter().filter(|item| item.kind == FeedKind::Delegated).count(), 1);
}

#[test]
fn a_delegated_line_never_carries_the_home_folder_or_a_secret() {
    let (service, _host, root) = two_companions();
    let home = root.join("mochi").display().to_string();
    service.note_delegated(MOCHI, "Codex", &home);
    let line = store::feed_read(&root, "mochi").unwrap().into_iter().find(|i| i.kind == FeedKind::Delegated).unwrap();
    assert!(!line.text.contains(&home), "{}", line.text);
}

#[test]
fn a_helper_may_not_work_in_the_companions_home_folder() {
    let (service, _host, root) = two_companions();
    let home = root.join("mochi");
    assert!(service.is_home_path(MOCHI, &home.display().to_string()));
    assert!(service.is_home_path(MOCHI, &home.join("memory").display().to_string()));
    assert!(!service.is_home_path(MOCHI, &root.join("bit").display().to_string()), "Bit's home is not Mochi's");
    assert!(!service.is_home_path(MOCHI, &root.join("mochi2").display().to_string()));
    assert!(!service.is_home_path("5f3c-ordinary-pane", &home.display().to_string()));
}

// ---- the hard rule: a companion never answers a helper's card ---------------------------------------

fn proc(pid: u32, parent: u32) -> ProcEntry {
    ProcEntry { pid, parent, image: "x.exe".into() }
}

/// Mochi's pane process 100 -> its agent 101 -> its Puppet Master MCP shim 102. The helper it started is a
/// separate pane: 400 -> 401 -> its own shim 402. An ordinary agent of the person: 300 -> 301.
fn table() -> Vec<ProcEntry> {
    vec![proc(100, 1), proc(101, 100), proc(102, 101), proc(300, 1), proc(301, 300), proc(400, 1), proc(401, 400), proc(402, 401)]
}

fn mochi_pane() -> Vec<CompanionPane> {
    vec![CompanionPane { id: MOCHI.into(), roots: vec![100], exited: false }]
}

#[test]
fn a_helper_is_not_a_companion_even_though_the_companion_started_it() {
    // The helper's own agent (pane 400..402) runs in an ordinary pane: it is not the companion, so it is
    // not the companion's voice. Its own questions go to the person like any agent's.
    let verdict = decide(&mochi_pane(), Some(&table()), Some(402), None);
    assert_eq!(verdict.suspected, None);
    assert_eq!(verdict.verified, None);
}
