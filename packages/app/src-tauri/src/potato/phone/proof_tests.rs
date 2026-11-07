//! A server for the proof run (ignored by default): the REAL phone server, REAL sockets and the REAL tunnel program
//! in front of a stand-in engine (a fake brain that answers every message), in a throwaway folder. A browser drives
//! the phone page against it (`D:\pm-night\phone-shots\`). It is a test binary, not the packaged app.
//!
//!   PHONE_PROOF_DIR=<empty folder> [PM_CLOUDFLARED_EXE=<exe>] [PHONE_PROOF_PORT=<n>] \
//!     cargo test --lib phone_proof_server -- --ignored --nocapture
//!
//! It writes `state.json` in the folder every second (port, addresses, tunnel). The driver talks to it with files:
//! `cmd.json` = `{ "do": "pair" | "relink" | "card" | "tunnel_on" | "tunnel_off" | "port", ... }` (deleted when done;
//! the answer is `out.json`). The file `stop` ends it, and the tunnel is turned off first; `stop-keep` ends it like an
//! app restart, leaving the tunnel program running.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::config::Config;
use super::live::{changes, Phone, Wiring};
use super::tunnel::Deps;
use crate::mobile_pairing::MobilePairingStore;
use crate::potato::service_card_tests::{ask, rig};
use crate::potato::service_tests::Rig;
use crate::potato::store::FromKind;

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("phone")
}

fn write_json(path: &Path, v: &Value) {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, v.to_string()).unwrap();
    let _ = std::fs::rename(&tmp, path);
}

/// Answers every message of the person once, as the brain would, and wakes the phones.
fn fake_brain(rig: &Rig, answered: &mut Vec<String>) {
    let waiting: Vec<_> = rig.thread().into_iter().filter(|m| m.from.kind == FromKind::You && !answered.contains(&m.id)).collect();
    for m in waiting {
        answered.push(m.id.clone());
        rig.post(vec![json!({"v":1,"type":"typing","conversationId":"app:dm:nimbus","on":true})]);
        std::thread::sleep(Duration::from_millis(900));
        rig.post(vec![json!({"v":1,"type":"reply","conversationId":"app:dm:nimbus","inReplyTo":m.id,"segments":[format!("Got it: {}", m.text.lines().next().unwrap_or(""))]})]);
        changes().bump();
    }
}

#[test]
#[ignore]
fn phone_proof_server() {
    let dir = PathBuf::from(std::env::var("PHONE_PROOF_DIR").expect("PHONE_PROOF_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let (rig, _fake) = rig("phone-proof");
    let pairing = Arc::new(parking_lot::Mutex::new(MobilePairingStore::load_or_create(dir.join("pairing.json")).unwrap()));
    let svc = rig.svc.clone();
    let port: u16 = std::env::var("PHONE_PROOF_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
    let phone = Phone::new(Wiring {
        data: dir.clone(),
        assets: Some(assets()),
        tunnel_exe: std::env::var("PM_CLOUDFLARED_EXE").ok().map(PathBuf::from),
        tunnel_deps: Deps::real(),
        groups: Arc::new(|| false),
        notify: Arc::new(|| {}),
        pairing,
        changes: changes(),
        bind: IpAddr::from([0, 0, 0, 0]),
        service: Box::new(move || Some(svc.clone())),
        config: Config { enabled: true, port, custom_address: None },
    });
    phone.start();
    let mut answered: Vec<String> = Vec::new();
    // What was said before the proof began needs no answer.
    for m in rig.thread() {
        answered.push(m.id);
    }
    loop {
        if dir.join("stop").exists() || dir.join("stop-keep").exists() {
            break;
        }
        write_json(&dir.join("state.json"), &serde_json::to_value(phone.status()).unwrap());
        if let Ok(text) = std::fs::read_to_string(dir.join("cmd.json")) {
            let _ = std::fs::remove_file(dir.join("cmd.json"));
            let cmd: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let out = match cmd["do"].as_str().unwrap_or("") {
                "pair" => phone.start_pairing(cmd["via"].as_str(), None).map(|p| serde_json::to_value(p).unwrap()),
                "relink" => phone.start_pairing(cmd["via"].as_str(), cmd["device"].as_str().map(str::to_string)).map(|p| serde_json::to_value(p).unwrap()),
                "card" => {
                    ask(&rig, cmd["id"].as_str().unwrap_or("a-1"));
                    changes().bump();
                    Ok(json!({"ok": true}))
                }
                "login" => {
                    let mut new = crate::potato::cards::New {
                        kind: crate::potato::cards::Kind::Question,
                        action: "Nimbus wants to sign in to shop.example.com".into(),
                        why: "to read your orders".into(),
                        choices: vec!["Sign in".into(), "Take over".into(), "Not now".into()],
                        conversation: "app:dm:nimbus".into(),
                        command_hash: None,
                        classes: vec![],
                        keep_going: None,
                        connector: None,
                        login: Some("https://shop.example.com".into()),
                        mcp_add: None,
                        speaker: "Nimbus".into(),
                        home: None,
                    };
                    new.login = Some("https://shop.example.com".into());
                    let raised = crate::potato::cards::raise(&rig.root, "nimbus", new, 3_000).map(|_| json!({"ok": true})).map_err(|e| e.to_string());
                    changes().bump();
                    raised
                }
                "tunnel_on" => phone.set_tunnel(true).map(|_| json!({"ok": true})),
                "tunnel_off" => phone.set_tunnel(false).map(|_| json!({"ok": true})),
                "port" => phone.set_port(cmd["port"].as_u64().unwrap_or(0) as u16).map(|_| json!({"ok": true})),
                other => Err(format!("unknown command {other}")),
            };
            write_json(&dir.join("out.json"), &match out {
                Ok(v) => v,
                Err(e) => json!({"error": e}),
            });
        }
        fake_brain(&rig, &mut answered);
        std::thread::sleep(Duration::from_millis(250));
    }
    // stop-keep is an app restart: the tunnel program is left running, as it is when the app quits.
    if !dir.join("stop-keep").exists() {
        let _ = phone.set_tunnel(false);
    }
    phone.stop();
}
