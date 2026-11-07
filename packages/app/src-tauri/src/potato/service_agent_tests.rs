//! A bot that talks through the owner's own agent ("Through my agents"): the agent runs in the (fake) sandbox, one process a turn.
//! Fakes only. A security test here is never deleted: an agent that is not approved, a message that is not the owner's own, a
//! group, a missing sandbox, a failed run and a stop must each end in a plain note, and no other brain may take over.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::bot_settings;
use super::custom_agents_tests::GOOD;
use super::rooms::store::{self as rstore, Room};
use super::sandbox::lease::ScreenLease;
use super::sandbox::{FakeSandbox, ShellOutput};
use super::sandbox_calls::Port;
use super::service_tests::Rig;
use super::store::{self, BrainKind, FromKind, MsgKind};

fn out(code: i32, stdout: &str, stderr: &str) -> ShellOutput {
    ShellOutput { code: Some(code), stdout: stdout.into(), stderr: stderr.into(), timed_out: false }
}

/// nimbus talks through the approved agent "scribe"; the sandbox is on; the spawner runs a job at once.
fn rig(name: &str) -> (Rig, Arc<FakeSandbox>) {
    let rig = Rig::new(name);
    rig.svc.set_spawner(Arc::new(|job| job()));
    let fake = Arc::new(FakeSandbox::new());
    rig.svc.set_sandbox_port(Some(Port { sandbox: fake.clone(), lease: Arc::new(Mutex::new(ScreenLease::new())) }));
    rig.svc.set_sandbox_enabled(true);
    bot_settings::update(&rig.root, "nimbus", |s| {
        s.brain = Some(BrainKind::Agents);
        Ok(())
    })
    .unwrap();
    let seen = rig.svc.agent_check(GOOD).unwrap();
    rig.svc.agent_add("nimbus", GOOD, &seen.fingerprint).unwrap();
    rig.svc.agent_pick("nimbus", Some("scribe")).unwrap();
    (rig, fake)
}

fn replies(rig: &Rig) -> Vec<String> {
    rig.thread().into_iter().filter(|m| m.from.kind == FromKind::Bot && m.kind == MsgKind::Text).map(|m| m.text).collect()
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn adding_needs_the_fingerprint_the_owner_read_and_only_approved_agents_can_be_picked() {
    let rig = Rig::new("agent-add");
    let seen = rig.svc.agent_check(GOOD).unwrap();
    assert!(rig.svc.agent_add("nimbus", GOOD, "not-the-one-they-read").unwrap_err().message.contains("changed after you read"));
    assert!(rig.svc.agent_add("nimbus", &GOOD.replace("--plain", "--other"), &seen.fingerprint).is_err(), "a changed text is not the approved one");
    assert!(rig.svc.agents_view("nimbus").unwrap().agents.is_empty());
    // an agent that was never approved cannot be picked
    assert!(rig.svc.agent_pick("nimbus", Some("scribe")).unwrap_err().message.contains("not approved"));
    let view = rig.svc.agent_add("nimbus", GOOD, &seen.fingerprint).unwrap();
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].command, "scribe run --plain -- <your message>");
    assert!(view.own_account_note.contains("cost isn't counted") && view.decides_note.contains("Approval cards don't apply"));
    assert_eq!(rig.svc.agent_pick("nimbus", Some("scribe")).unwrap().brain_agent.as_deref(), Some("scribe"));
    // removing it un-picks it
    assert_eq!(rig.svc.agent_remove("nimbus", "scribe").unwrap().brain_agent, None);
}

#[test]
fn the_owners_message_runs_the_agent_in_the_sandbox_and_its_words_are_the_reply() {
    let (rig, fake) = rig("agent-turn");
    bot_settings::update(&rig.root, "nimbus", |s| {
        bot_settings::save_instructions(s, "Be brief. Key sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD", 1).map(|_| ())
    })
    .unwrap();
    fake.queue_agent_results(vec![out(0, "Hello from the agent\n", "")]);
    let sent = rig.svc.send("nimbus", "hi there").unwrap();
    assert!(sent.delivered);
    assert_eq!(replies(&rig), ["Hello from the agent"]);
    let scripts = fake.agent_scripts();
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0].contains("hi there") && scripts[0].contains("Be brief."));
    // the key in the instructions did not travel; no brain process was started; the bot's own shell was not used
    assert!(!scripts[0].contains("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD"), "{}", scripts[0]);
    assert_eq!(rig.spawned(), 0);
    assert!(fake.calls().iter().all(|c| !matches!(c, super::sandbox::Recorded::Shell(_) | super::sandbox::Recorded::RootShell(_) | super::sandbox::Recorded::SecretConfig(_))));
    // the agent's words are untrusted: they are in the thread and nowhere else (no note was written)
    assert!(super::memory_notes::list(&rig.root, "nimbus").unwrap().is_empty());
    // the second turn carries the first as recent chat
    fake.queue_agent_results(vec![out(0, "second", "")]);
    rig.svc.send("nimbus", "and again").unwrap();
    let scripts = fake.agent_scripts();
    assert!(scripts[1].contains("Owner: hi there") && scripts[1].contains("Hello from the agent"));
}

#[test]
fn an_agent_that_is_not_approved_or_changed_is_refused_and_nothing_runs() {
    let (rig, fake) = rig("agent-unapproved");
    // the stored definition is edited behind the app's back
    bot_settings::update(&rig.root, "nimbus", |s| {
        s.custom_agents[0].toml = s.custom_agents[0].toml.replace("--plain", "--evil");
        Ok(())
    })
    .unwrap();
    rig.svc.send("nimbus", "hi").unwrap();
    assert!(fake.agent_scripts().is_empty());
    assert!(rig.notes().iter().any(|n| n.contains("cannot run") && n.contains("changed since you approved")), "{:?}", rig.notes());
    // a pick of an id that was never approved
    bot_settings::update(&rig.root, "nimbus", |s| {
        s.brain_agent = Some("ghost".into());
        Ok(())
    })
    .unwrap();
    rig.svc.send("nimbus", "hi again").unwrap();
    assert!(fake.agent_scripts().is_empty());
    assert!(rig.notes().iter().any(|n| n.contains("not approved")));
    assert!(replies(&rig).is_empty());
}

#[test]
fn a_message_that_is_not_the_owners_own_is_refused_with_a_plain_reason() {
    let (rig, fake) = rig("agent-peer");
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    // another bot's message and a card answer are not the owner's own words
    rig.svc.send_as("nimbus", "do this for me", super::service::Origin::Peer { from_id: "mochi", from_name: "Mochi" }).unwrap();
    rig.svc.send_as("nimbus", "Yes", super::service::Origin::Answer).unwrap();
    assert!(fake.agent_scripts().is_empty());
    assert!(rig.notes().iter().filter(|n| n.contains("cannot be limited") && n.contains("only you in your private chat")).count() >= 2, "{:?}", rig.notes());
}

#[test]
fn a_group_never_reaches_the_agent_and_says_why() {
    let (rig, fake) = rig("agent-room");
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    let room = "r0123456789ab";
    rstore::save_rooms(&rig.root, &[Room { id: room.into(), name: "Weekend".into(), bot_ids: vec!["nimbus".into(), "mochi".into()], created_ms: 1, last_ms: 1, unread: 0, ..Default::default() }]).unwrap();
    rig.svc.send_room(room, "@everyone hi").unwrap();
    assert!(fake.agent_scripts().is_empty());
    let lines = rig.svc.room_lines(room, 50).unwrap();
    assert!(lines.iter().any(|l| l.text.contains("talks through your own agent") && l.text.contains("not in groups")), "{:?}", lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>());
}

#[test]
fn with_no_sandbox_it_says_so_plainly_and_runs_nothing_and_falls_back_to_nothing() {
    let (rig, fake) = rig("agent-nosandbox");
    rig.svc.set_sandbox_enabled(false);
    let sent = rig.svc.send("nimbus", "hi").unwrap();
    assert!(!sent.delivered);
    assert!(rig.notes().iter().any(|n| n.contains("runs only in the sandbox") && n.contains("not switched on or not set up")), "{:?}", rig.notes());
    assert!(fake.agent_scripts().is_empty());
    assert_eq!(rig.spawned(), 0, "no other brain took over");
    rig.svc.set_sandbox_enabled(true);
    rig.svc.set_sandbox_port(None);
    rig.svc.send("nimbus", "hi").unwrap();
    assert_eq!(rig.spawned(), 0);
}

#[test]
fn a_failed_run_shows_a_plain_error_and_no_other_brain_takes_over() {
    let (rig, fake) = rig("agent-fail");
    fake.queue_agent_results(vec![out(2, "", "boom: no such model")]);
    rig.svc.send("nimbus", "hi").unwrap();
    assert!(replies(&rig).is_empty());
    assert!(rig.notes().iter().any(|n| n.contains("Scribe stopped with an error") && n.contains("boom: no such model")), "{:?}", rig.notes());
    assert_eq!(rig.spawned(), 0);
    fake.queue_agent_results(vec![out(127, "", "AGENT_NOT_INSTALLED\n")]);
    rig.svc.send("nimbus", "hi").unwrap();
    assert!(rig.notes().iter().any(|n| n.contains("not installed in the sandbox")));
    assert_eq!(rig.spawned(), 0);
}

#[test]
fn the_time_limit_stops_a_run_that_never_ends() {
    let (rig, fake) = rig("agent-timeout");
    rig.svc.set_agent_limits(0, 150);
    fake.set_agent_hang(true);
    rig.svc.send("nimbus", "hi").unwrap();
    assert!(rig.notes().iter().any(|n| n.contains("took too long")), "{:?}", rig.notes());
    assert!(replies(&rig).is_empty());
    assert!(!rig.svc.agent_running("nimbus"));
}

#[test]
fn stop_kills_the_agent_and_no_reply_is_saved() {
    let (rig, fake) = rig("agent-stop");
    rig.svc.set_spawner(Arc::new(|job| {
        std::thread::spawn(job);
    }));
    fake.set_agent_hang(true);
    rig.svc.send("nimbus", "a long job").unwrap();
    wait_for("the agent to start", || rig.svc.agent_running("nimbus") && !fake.agent_scripts().is_empty());
    // a second message while it works is not run beside it
    rig.svc.send("nimbus", "another").unwrap();
    assert_eq!(fake.agent_scripts().len(), 1);
    assert!(rig.notes().iter().any(|n| n.contains("still working")));
    rig.svc.stop_bot_by_person("nimbus");
    wait_for("the agent to be killed", || !rig.svc.agent_running("nimbus"));
    wait_for("the stop note", || rig.notes().iter().any(|n| n.contains("Stopped") && n.contains("killed in the sandbox")));
    assert!(replies(&rig).is_empty());
}

#[test]
fn only_one_agent_turn_runs_in_the_sandbox_at_a_time_even_for_another_bot() {
    let (rig, fake) = rig("agent-one-at-a-time");
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    bot_settings::update(&rig.root, "mochi", |s| {
        s.brain = Some(BrainKind::Agents);
        Ok(())
    })
    .unwrap();
    let seen = rig.svc.agent_check(GOOD).unwrap();
    rig.svc.agent_add("mochi", GOOD, &seen.fingerprint).unwrap();
    rig.svc.agent_pick("mochi", Some("scribe")).unwrap();
    rig.svc.set_spawner(Arc::new(|job| {
        std::thread::spawn(job);
    }));
    fake.set_agent_hang(true);
    rig.svc.send("nimbus", "a long job").unwrap();
    wait_for("the first agent to start", || rig.svc.agent_running("nimbus") && !fake.agent_scripts().is_empty());
    let sent = rig.svc.send("mochi", "me too").unwrap();
    assert!(!sent.delivered);
    assert_eq!(fake.agent_scripts().len(), 1, "the second turn did not start");
    let mochi_notes: Vec<String> = rig.svc.messages("mochi", 100).unwrap().into_iter().filter(|m| m.kind == MsgKind::Note).map(|m| m.text).collect();
    assert!(mochi_notes.iter().any(|n| n.contains("Another agent is working in the sandbox")), "{mochi_notes:?}");
    rig.svc.stop_bot_by_person("nimbus");
    wait_for("the agent to be killed", || !rig.svc.agent_running("nimbus"));
}

#[test]
fn a_launch_that_refuses_because_a_guard_is_missing_shows_its_words_and_no_reply() {
    let (rig, fake) = rig("agent-guard-refusal");
    for (code, words) in [
        (72, "The sandbox could not close its local network to this agent. Nothing was started."),
        (71, "The sandbox screen is open to other users. Restart the sandbox, then try again. Nothing was started."),
        (73, "The sandbox could not stop an earlier agent run. Nothing was started."),
    ] {
        fake.queue_agent_results(vec![out(code, "", words)]);
        rig.svc.send("nimbus", "hi").unwrap();
        assert!(rig.notes().iter().any(|n| n.contains(words)), "{:?}", rig.notes());
    }
    assert!(replies(&rig).is_empty());
    assert_eq!(rig.spawned(), 0, "no other brain took over");
    assert!(!rig.svc.agent_running("nimbus"));
}

const SECRET_DEF: &str = r#"schema_version = 1

[agent]
id = "keyed"
label = "Keyed"

[binary]
names = ["keyed"]

[headless]
argv = ["run"]
task_separator = "--"

[env]
secret = ["KEYED_TOKEN"]
"#;

const THE_KEY: &str = "keyed-live-value-4f9a1c77e2";

fn keyed_rig(name: &str) -> (Rig, Arc<FakeSandbox>) {
    let (rig, fake) = rig(name);
    let seen = rig.svc.agent_check(SECRET_DEF).unwrap();
    rig.svc.agent_add("nimbus", SECRET_DEF, &seen.fingerprint).unwrap();
    rig.svc.agent_pick("nimbus", Some("keyed")).unwrap();
    (rig, fake)
}

fn key_name() -> String {
    super::custom_agents::agent_secret_name("nimbus", "keyed", "KEYED_TOKEN")
}

fn files_under(dir: &std::path::Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, found);
        } else if let Ok(bytes) = std::fs::read(&path) {
            found.push(String::from_utf8_lossy(&bytes).to_string());
        }
    }
}

#[test]
fn the_owners_secret_goes_to_that_agents_process_only_and_is_in_no_thread_file_view_or_reply() {
    let (rig, fake) = keyed_rig("agent-secret");
    // the page sees the name to set, not a value
    let view = rig.svc.agents_view("nimbus").unwrap();
    let keyed = view.agents.iter().find(|a| a.id == "keyed").unwrap();
    assert_eq!((keyed.secrets[0].env.as_str(), keyed.secrets[0].key.as_str()), ("KEYED_TOKEN", key_name().as_str()));
    // the owner types it in Settings (the credential store); another agent of the same bot gets nothing of it
    rig.host.secrets.lock().insert(key_name(), THE_KEY.to_string());
    // the agent prints the key back in its answer, and in its error stream
    fake.queue_agent_results(vec![out(0, &format!("done, key was {THE_KEY}"), &format!("warn {THE_KEY}"))]);
    rig.svc.send("nimbus", "go").unwrap();
    let scripts = fake.agent_scripts();
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].matches(THE_KEY).count(), 1, "once, in the here-document for that agent");
    assert!(scripts[0].contains(&format!("<<'PMSEC'\nKEYED_TOKEN={THE_KEY}\nPMSEC\n")));
    // the answer was shown without it
    assert_eq!(replies(&rig), ["done, key was [hidden]"]);
    // not in the thread, the notes, the settings, the view, any file the bot has, nor the recorded sandbox calls except that script
    let all: String = rig.thread().into_iter().map(|m| m.text).collect::<Vec<_>>().join("\n");
    assert!(!all.contains(THE_KEY));
    let view_json = serde_json::to_string(&rig.svc.agents_view("nimbus").unwrap()).unwrap();
    assert!(!view_json.contains(THE_KEY));
    let mut files = Vec::new();
    files_under(&rig.root, &mut files);
    assert!(!files.is_empty() && files.iter().all(|f| !f.contains(THE_KEY)), "no file under the bot's data holds the value");
    assert!(fake.calls().iter().filter(|c| format!("{c:?}").contains(THE_KEY)).count() == 1);
    // a failed run: the error text is cleaned too
    fake.queue_agent_results(vec![out(2, "", &format!("auth failed for {THE_KEY}"))]);
    rig.svc.send("nimbus", "again").unwrap();
    assert!(rig.notes().iter().any(|n| n.contains("auth failed for [hidden]")), "{:?}", rig.notes());
    assert!(rig.notes().iter().all(|n| !n.contains(THE_KEY)));
}

#[test]
fn an_agent_that_was_given_no_value_still_runs_and_gets_none() {
    let (rig, fake) = keyed_rig("agent-nosecret");
    fake.queue_agent_results(vec![out(0, "hello", "")]);
    rig.svc.send("nimbus", "go").unwrap();
    let script = &fake.agent_scripts()[0];
    assert!(!script.contains("KEYED_TOKEN") && script.contains("</dev/null"));
    assert_eq!(replies(&rig), ["hello"]);
}

#[test]
fn removing_an_agent_deletes_its_saved_space_and_its_secrets_and_a_failed_delete_keeps_the_agent() {
    let (rig, fake) = keyed_rig("agent-remove-space");
    rig.host.secrets.lock().insert(key_name(), THE_KEY.to_string());
    fake.set_agent_home_fails(true);
    let e = rig.svc.agent_remove("nimbus", "keyed").unwrap_err();
    assert!(e.message.contains("saved space could not be deleted"), "{}", e.message);
    assert!(rig.svc.agents_view("nimbus").unwrap().agents.iter().any(|a| a.id == "keyed"), "still there, so it can be tried again");
    assert!(rig.host.secrets.lock().contains_key(&key_name()));
    fake.set_agent_home_fails(false);
    let view = rig.svc.agent_remove("nimbus", "keyed").unwrap();
    assert!(view.agents.iter().all(|a| a.id != "keyed") && view.brain_agent.is_none());
    assert!(fake.calls().contains(&super::sandbox::Recorded::AgentHomeRemove("/var/lib/pm-agents/nimbus--keyed.img".into())));
    assert!(!rig.host.secrets.lock().contains_key(&key_name()), "its value left the credential store");
}
