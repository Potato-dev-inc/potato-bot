//! "Can control the bot's computer": default off, saved with the phone's pairing, applied to the hub at start, only the
//! PC sets it, and a signed-out phone loses it. (The live relay closing is proved in `screen_tests`.)

use std::sync::Arc;

use parking_lot::Mutex;

use super::control;
use super::testkit::kit;
use crate::mobile_pairing::{Cap, MobilePairingStore, SharedPairingStore};
use crate::potato::sandbox::screen::ScreenHub;

#[test]
fn it_is_off_by_default_and_the_device_list_says_so() {
    let k = kit("ctl-default", None);
    let (id, _) = k.pair();
    let row = k.pairing.lock().phone_devices().into_iter().find(|d| d.id == id).unwrap();
    assert!(!row.control_computer);
    assert!(k.pairing.lock().phone_control_ids().is_empty());
    assert!(!k.env.hub.phone_may_control(&id));
}

#[test]
fn the_yes_is_saved_with_the_pairing_and_survives_a_restart() {
    let k = kit("ctl-persist", None);
    let (id, _) = k.pair();
    control::set(&k.pairing, &k.env.hub, &id, true).unwrap();
    assert!(k.env.hub.phone_may_control(&id));
    // A new process: the pairing file is read again, a new hub starts with no yes, and `apply_saved` gives it back.
    let again: SharedPairingStore = Arc::new(Mutex::new(MobilePairingStore::load_or_create(k.dir.join("pairing.json")).unwrap()));
    assert!(again.lock().phone_devices().iter().find(|d| d.id == id).unwrap().control_computer);
    let hub = ScreenHub::new();
    assert!(!hub.phone_may_control(&id));
    control::apply_saved(&again, &hub);
    assert!(hub.phone_may_control(&id));
    // And off is saved too.
    control::set(&again, &hub, &id, false).unwrap();
    assert!(!hub.phone_may_control(&id));
    let third = MobilePairingStore::load_or_create(k.dir.join("pairing.json")).unwrap();
    assert!(third.phone_control_ids().is_empty());
}

#[test]
fn an_unknown_phone_is_an_error_and_changes_nothing() {
    let k = kit("ctl-unknown", None);
    let err = control::set(&k.pairing, &k.env.hub, "nobody", true).unwrap_err();
    assert!(err.starts_with("PHONE_NOT_FOUND:"), "{err}");
    assert!(!k.env.hub.phone_may_control("nobody"));
}

#[test]
fn a_revoked_phone_loses_the_yes_and_a_relinked_one_keeps_it() {
    let k = kit("ctl-revoke", None);
    let (id, _) = k.pair();
    control::set(&k.pairing, &k.env.hub, &id, true).unwrap();
    // Re-linking gives a new token to the same record: the person's yes stays.
    let code = k.pairing.lock().mint_phone_code("x".into(), crate::mobile_pairing::Caps::all(), Some(id.clone())).unwrap().code;
    k.pairing.lock().redeem_phone_code(&code, "ignored").unwrap();
    assert!(k.pairing.lock().phone_control_ids().contains(&id));
    // Signing out removes the record (and with it the saved switch) and clears the hub.
    assert!(k.pairing.lock().revoke_phone_device(&id).unwrap());
    control::clear(&k.env.hub, &id);
    assert!(!k.env.hub.phone_may_control(&id));
    assert!(k.pairing.lock().phone_control_ids().is_empty());
}

#[test]
fn the_phone_itself_signing_out_clears_the_yes() {
    let k = kit("ctl-self-out", None);
    let (id, token) = k.pair();
    control::set(&k.pairing, &k.env.hub, &id, true).unwrap();
    assert_eq!(k.authed(&token, "DELETE", "/api/me", "").status, 200);
    assert!(!k.env.hub.phone_may_control(&id));
}

#[test]
fn only_the_pc_can_change_it_the_phone_api_has_no_route() {
    let k = kit("ctl-pc-only", None);
    let (id, token) = k.pair();
    for (method, path) in [
        ("POST", "/api/me/control"),
        ("PUT", "/api/me/control"),
        ("POST", "/api/devices/control"),
        ("POST", "/api/phone/control"),
        ("POST", "/api/bots/nimbus/control-computer"),
    ] {
        let r = k.authed(&token, method, path, r#"{"on":true,"deviceId":"x"}"#);
        assert!(r.status == 404 || r.status == 403 || r.status == 405, "{method} {path} -> {}", r.status);
    }
    assert!(!k.env.hub.phone_may_control(&id));
    assert!(!k.pairing.lock().phone_devices().iter().find(|d| d.id == id).unwrap().control_computer);
    // Even a phone with every ability cannot say yes to itself.
    assert!(k.pairing.lock().device_caps(&id).unwrap().has(Cap::Control));
}
