//! The phone server's door. What these tests pin down (the rules of the brief, in order):
//! nothing but the app files, `health`, `hello` and `pair` answers without a token, from the Wi-Fi or from loopback or
//! through a tunnel's headers; the Host and Origin allow-lists; CSRF; pairing is single use, short-lived and rate limited
//! (tighter through a tunnel); a revoked phone is out at once; abilities are enforced; the app files cannot be walked out of.

use serde_json::json;

use super::gate::Changes;
use super::net::Hosts;
use super::proof;
use super::testkit::{kit, req, Kit};
use crate::mobile_pairing::{Cap, Caps};

fn body_of(r: &super::http::Response) -> serde_json::Value {
    r.body_json()
}

const API_ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/me"),
    ("GET", "/api/bots"),
    ("GET", "/api/bots/nimbus"),
    ("GET", "/api/changes?since=0&wait_ms=0"),
    ("GET", "/api/rooms/r-abc"),
    ("GET", "/api/push/key"),
    ("POST", "/api/bots/nimbus/send"),
    ("POST", "/api/bots/nimbus/stop"),
    ("POST", "/api/bots/nimbus/read"),
    ("POST", "/api/bots/nimbus/cards/card-1/answer"),
    ("POST", "/api/pause-all"),
    ("POST", "/api/resume-all"),
    ("POST", "/api/push/subscribe"),
    ("POST", "/api/push/presence"),
    ("DELETE", "/api/me"),
    ("GET", "/api/v1/bots"),
    ("GET", "/api/anything-else"),
];

#[test]
fn every_route_but_the_open_ones_refuses_a_caller_with_no_token() {
    let k = kit("no-token", None);
    for (method, path) in API_ROUTES {
        let r = k.call(method, path, &[], "{}");
        assert_eq!(r.status, 401, "{method} {path} answered {} without a token", r.status);
        assert_eq!(body_of(&r)["error"], "unpaired", "{method} {path}");
    }
}

#[test]
fn a_request_with_tunnel_headers_and_no_token_is_refused() {
    let k = kit("fwd", None);
    for header in [("cf-connecting-ip", "203.0.113.9"), ("x-forwarded-for", "203.0.113.9"), ("x-forwarded-host", "bot.example.com"), ("forwarded", "for=203.0.113.9"), ("cf-ray", "abc"), ("x-real-ip", "1.2.3.4"), ("x-pm-proxied", "1")] {
        for (method, path) in API_ROUTES {
            let r = k.call(method, path, &[header], "{}");
            assert_eq!(r.status, 401, "{method} {path} with {header:?}");
        }
    }
}

#[test]
fn loopback_is_not_trust() {
    let k = kit("loopback", None);
    // The tunnel program connects from 127.0.0.1; so does any local program. Neither is the phone's owner.
    for (method, path) in API_ROUTES {
        let mut r = req(method, path, &[("host", "127.0.0.1:1420")], "{}");
        r.peer = "127.0.0.1".parse().unwrap();
        let out = super::gate::dispatch(&k.ctx, &r);
        assert_eq!(out.status, 401, "{method} {path} from loopback");
    }
}

#[test]
fn a_wrong_or_garbled_token_is_refused_and_a_valid_one_works_with_or_without_tunnel_headers() {
    let k = kit("tokens", None);
    let (_, token) = k.pair();
    for bad in ["", "AAAA", "not a token", &"x".repeat(500), "Bearer"] {
        let r = k.call("GET", "/api/me", &[("authorization", &format!("Bearer {bad}"))], "");
        assert_eq!(r.status, 401, "{bad:?}");
    }
    assert_eq!(k.call("GET", "/api/me", &[("authorization", &token)], "").status, 401, "no Bearer word");
    let ok = k.authed(&token, "GET", "/api/me", "");
    assert_eq!(ok.status, 200);
    assert_eq!(body_of(&ok)["name"], "Test phone");
    // The headers a tunnel adds change nothing about what a valid token may do.
    let bearer = format!("Bearer {token}");
    let via = k.call("GET", "/api/me", &[("authorization", &bearer), ("cf-connecting-ip", "198.51.100.7")], "");
    assert_eq!(via.status, 200);
}

#[test]
fn the_open_routes_are_exactly_health_hello_pair_and_the_app_files() {
    let k = kit("open", None);
    assert_eq!(k.call("GET", "/api/health", &[], "").status, 200);
    assert_eq!(body_of(&k.call("GET", "/api/health", &[], ""))["app"], "potato-phone");
    assert_eq!(k.call("GET", "/", &[], "").status, 200);
    assert_eq!(k.call("GET", "/app.js", &[], "").status, 200);
    assert_eq!(k.call("GET", "/pair/ABC", &[], "").status, 200, "a path with no file opens the app (the fragment does the work)");
    assert_eq!(k.call("POST", "/api/hello", &[], r#"{"kind":"pair","nonce":"0123456789abcdef"}"#).status, 200);
}

// ---- Host and Origin ------------------------------------------------------------------------------------------

#[test]
fn a_host_name_that_is_not_ours_is_refused() {
    let k = kit("host", None);
    for host in ["evil.example.com", "evil.example.com:1420", "test-pc.evil.com", "bot.example.com"] {
        let r = k.call("GET", "/api/health", &[("host", host)], "");
        assert_eq!(r.status, 421, "{host}");
        assert_eq!(k.call("GET", "/", &[("host", host)], "").status, 421, "{host} for the app files too");
    }
    for host in ["192.168.1.5", "10.0.0.2:8123", "localhost:1420", "test-pc:1420", "phone.local"] {
        assert_eq!(k.call("GET", "/api/health", &[("host", host)], "").status, 200, "{host}");
    }
    let mut no_host = req("GET", "/api/health", &[], "");
    no_host.headers.retain(|(k, _)| k != "host");
    assert_eq!(super::gate::dispatch(&k.ctx, &no_host).status, 400);
}

#[test]
fn the_configured_domain_and_nothing_else_is_accepted() {
    let k = kit("custom-host", None);
    *k.env.hosts.lock() = Hosts { custom: Some("bot.example.com".into()), tunnel: Some("quiet-fox.trycloudflare.com".into()), machine: None, tunnel_starting: false };
    assert_eq!(k.call("GET", "/api/health", &[("host", "bot.example.com")], "").status, 200);
    assert_eq!(k.call("GET", "/api/health", &[("host", "quiet-fox.trycloudflare.com")], "").status, 200);
    for host in ["other.example.com", "bot.example.com.evil.com", "evilbot.example.com", "trycloudflare.com", "other-fox.trycloudflare.com"] {
        assert_eq!(k.call("GET", "/api/health", &[("host", host)], "").status, 421, "{host}");
    }
}

#[test]
fn a_page_from_another_site_cannot_use_the_api_and_gets_no_cors_headers() {
    let k = kit("origin", None);
    let (_, token) = k.pair();
    let bearer = format!("Bearer {token}");
    for origin in ["https://evil.example.com", "http://192.168.1.77:1420", "null", "file://", "http://localhost:3000"] {
        let r = k.call("GET", "/api/me", &[("authorization", &bearer), ("origin", origin)], "");
        assert_eq!(r.status, 403, "{origin}");
        assert!(r.headers.iter().all(|(n, _)| !n.eq_ignore_ascii_case("access-control-allow-origin")), "{origin} must get no CORS header");
        let pre = k.call("OPTIONS", "/api/me", &[("origin", origin), ("access-control-request-method", "GET")], "");
        assert_eq!(pre.status, 403, "preflight from {origin}");
    }
}

#[test]
fn only_our_own_addresses_get_cors_and_the_header_names_the_origin_never_a_star() {
    let k = kit("cors", None);
    k.env.candidates.lock().push("https://quiet-fox.trycloudflare.com".into());
    let (_, token) = k.pair();
    let bearer = format!("Bearer {token}");
    let r = k.call("GET", "/api/me", &[("authorization", &bearer), ("origin", "https://quiet-fox.trycloudflare.com")], "");
    assert_eq!(r.status, 200);
    let allow: Vec<_> = r.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("access-control-allow-origin")).collect();
    assert_eq!(allow.len(), 1);
    assert_eq!(allow[0].1, "https://quiet-fox.trycloudflare.com");
    let pre = k.call("OPTIONS", "/api/me", &[("origin", "https://quiet-fox.trycloudflare.com")], "");
    assert_eq!(pre.status, 204);
    assert!(pre.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("access-control-allow-headers") && v.contains("Authorization") && v.contains("X-Potato-Phone")));
    assert!(pre.headers.iter().all(|(_, v)| v != "*"));
    // Our own page (same origin as the Host) needs none.
    let same = k.call("GET", "/api/me", &[("authorization", &bearer), ("origin", "http://192.168.1.5:1420")], "");
    assert_eq!(same.status, 200);
    assert!(same.headers.iter().all(|(n, _)| !n.eq_ignore_ascii_case("access-control-allow-origin")));
}

#[test]
fn a_change_needs_the_phone_header_so_a_form_on_another_site_cannot_make_one() {
    let k = kit("csrf", None);
    let (_, token) = k.pair();
    let bearer = format!("Bearer {token}");
    for (method, path) in [("POST", "/api/pause-all"), ("POST", "/api/bots/nimbus/stop"), ("DELETE", "/api/me")] {
        let mut r = req(method, path, &[("authorization", &bearer)], "{}");
        r.headers.retain(|(k, _)| k != "x-potato-phone");
        let out = super::gate::dispatch(&k.ctx, &r);
        assert_eq!(out.status, 403, "{method} {path}");
        assert_eq!(body_of(&out)["error"], "bad_request");
    }
    let mut pair = req("POST", "/api/pair", &[], r#"{"code":"AAAAAAAA"}"#);
    pair.headers.retain(|(k, _)| k != "x-potato-phone");
    assert_eq!(super::gate::dispatch(&k.ctx, &pair).status, 403, "even the open pair route");
}

#[test]
fn the_app_page_is_never_framed_or_cached_and_carries_a_strict_policy() {
    let k = kit("headers", None);
    let r = k.call("GET", "/", &[], "");
    let h = |name: &str| r.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default();
    assert!(h("content-security-policy").contains("frame-ancestors 'none'") && h("content-security-policy").contains("script-src 'self'"));
    assert_eq!(h("x-frame-options"), "DENY");
    assert_eq!(h("x-content-type-options"), "nosniff");
    assert_eq!(h("referrer-policy"), "no-referrer");
    assert!(h("cache-control").contains("no-store"));
}

// ---- pairing ---------------------------------------------------------------------------------------------------

fn pair_body(code: &str) -> String {
    json!({ "code": code, "deviceName": "Ren's phone" }).to_string()
}

fn mint(k: &Kit) -> String {
    k.pairing.lock().mint_phone_code("http://x".into(), Caps::all(), None).unwrap().code
}

#[test]
fn pairing_works_once_and_gives_a_token_that_works() {
    let k = kit("pair-once", None);
    let code = mint(&k);
    let r = k.call("POST", "/api/pair", &[], &pair_body(&code));
    assert_eq!(r.status, 200, "{:?}", body_of(&r));
    let out = body_of(&r);
    let token = out["token"].as_str().unwrap().to_string();
    assert_eq!(out["pcName"], "TEST-PC");
    assert_eq!(out["candidates"][0], "http://192.168.1.5:1420");
    assert_eq!(k.authed(&token, "GET", "/api/me", "").status, 200);
    let again = k.call("POST", "/api/pair", &[], &pair_body(&code));
    assert_eq!(again.status, 400, "a used code is dead");
    assert_eq!(body_of(&again)["error"], "bad_code");
}

#[test]
fn a_code_that_ran_out_says_so_and_a_wrong_one_does_not_reveal_which_is_close() {
    let k = kit("pair-expire", None);
    let code = mint(&k);
    // Move the code's clock: redeem 11 minutes later through the store the route uses.
    let late = k.pairing.lock().redeem_phone_code_at(&code, "p", crate::mobile_pairing::now_secs_for_test() + 11 * 60);
    assert_eq!(late, Err(crate::mobile_pairing::PairFail::Expired));
    let r = k.call("POST", "/api/pair", &[], &pair_body(&code));
    assert_eq!(r.status, 400, "after it is seen expired it is gone");
    let code2 = mint(&k);
    let wrong = format!("{}{}", &code2[..7], if code2.ends_with('A') { "B" } else { "A" });
    let r = k.call("POST", "/api/pair", &[], &pair_body(&wrong));
    assert_eq!(body_of(&r)["error"], "bad_code");
    assert!(!r.body.windows(code2.len()).any(|w| w == code2.as_bytes()), "the answer never contains the real code");
}

#[test]
fn five_wrong_guesses_burn_the_code() {
    let k = kit("pair-burn", None);
    let code = mint(&k);
    for _ in 0..4 {
        assert_eq!(body_of(&k.call("POST", "/api/pair", &[], &pair_body("ZZZZZZZZ")))["error"], "bad_code");
    }
    assert_eq!(body_of(&k.call("POST", "/api/pair", &[], &pair_body("ZZZZZZZZ")))["error"], "burned");
    let r = k.call("POST", "/api/pair", &[], &pair_body(&code));
    assert_eq!(r.status, 400, "the right code no longer works after five wrong ones");
    assert_eq!(body_of(&r)["error"], "bad_code");
}

#[test]
fn pairing_is_rate_limited_and_tighter_through_a_tunnel() {
    let k = kit("pair-rate", None);
    // Straight from the Wi-Fi: 10 a minute.
    let mut last = 0;
    for _ in 0..12 {
        last = k.call("POST", "/api/pair", &[], "{}").status;
    }
    assert_eq!(last, 429);
    // A fresh server, through a tunnel (loopback peer, forwarder header): 4 a minute, however many "different" callers.
    let k = kit("pair-rate-tunnel", None);
    let mut statuses = Vec::new();
    for i in 0..8 {
        let ip = format!("203.0.113.{i}");
        let mut r = req("POST", "/api/pair", &[("cf-connecting-ip", &ip), ("host", "quiet-fox.trycloudflare.com")], "{}");
        r.peer = "127.0.0.1".parse().unwrap();
        *k.env.hosts.lock() = Hosts { tunnel: Some("quiet-fox.trycloudflare.com".into()), ..Hosts::default() };
        statuses.push(super::gate::dispatch(&k.ctx, &r).status);
    }
    assert_eq!(statuses.iter().filter(|s| **s == 429).count(), 4, "{statuses:?}");
    assert!(statuses[..4].iter().all(|s| *s != 429));
}

#[test]
fn a_right_code_over_a_tunnel_still_works_within_the_limit() {
    let k = kit("pair-tunnel-ok", None);
    *k.env.hosts.lock() = Hosts { tunnel: Some("quiet-fox.trycloudflare.com".into()), ..Hosts::default() };
    let code = mint(&k);
    let mut r = req("POST", "/api/pair", &[("cf-connecting-ip", "203.0.113.5"), ("host", "quiet-fox.trycloudflare.com"), ("origin", "https://quiet-fox.trycloudflare.com")], &pair_body(&code));
    r.peer = "127.0.0.1".parse().unwrap();
    assert_eq!(super::gate::dispatch(&k.ctx, &r).status, 200);
}

#[test]
fn a_relink_gives_the_same_phone_a_new_token_and_the_old_one_stops() {
    let k = kit("relink", None);
    let (id, old) = k.pair();
    let code = k.pairing.lock().mint_phone_code("http://y".into(), Caps::all(), Some(id.clone())).unwrap().code;
    let out = body_of(&k.call("POST", "/api/pair", &[], &pair_body(&code)));
    assert_eq!(out["deviceId"], id.as_str());
    assert_eq!(k.authed(&old, "GET", "/api/me", "").status, 401);
    assert_eq!(k.authed(out["token"].as_str().unwrap(), "GET", "/api/me", "").status, 200);
}

// ---- hello: is this my PC -----------------------------------------------------------------------------------------

fn hello(k: &Kit, body: serde_json::Value) -> serde_json::Value {
    body_of(&k.call("POST", "/api/hello", &[], &body.to_string()))
}

#[test]
fn the_pair_hello_proves_knowledge_of_the_code_and_a_wrong_code_proves_nothing() {
    use sha2::{Digest, Sha256};
    let k = kit("hello-pair", None);
    let code = mint(&k);
    let nonce = "00112233445566778899aabbccddeeff";
    let answer = hello(&k, json!({ "kind": "pair", "nonce": nonce }));
    let right = proof::proof(&Sha256::digest(code.as_bytes()), nonce);
    assert_eq!(answer["proof"], right.as_str());
    assert_eq!(answer["pcName"], "TEST-PC");
    let wrong = proof::proof(&Sha256::digest(b"AAAAAAAA"), nonce);
    assert_ne!(answer["proof"], wrong.as_str(), "a phone holding another code rejects this machine");
    // A fresh nonce gives a fresh proof (no replay).
    let other = hello(&k, json!({ "kind": "pair", "nonce": "ffeeddccbbaa99887766554433221100" }));
    assert_ne!(other["proof"], answer["proof"]);
}

#[test]
fn the_device_hello_is_bound_to_the_token_and_an_unknown_phone_gets_an_answer_that_cannot_verify() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let k = kit("hello-device", None);
    let (id, token) = k.pair();
    let nonce = "abcdefabcdefabcdefabcdefabcdefab";
    let raw = base64::engine::general_purpose::STANDARD.decode(&token).unwrap();
    let want = proof::proof(&Sha256::digest(&raw), nonce);
    assert_eq!(hello(&k, json!({ "kind": "device", "deviceId": id, "nonce": nonce }))["proof"], want.as_str());
    let stranger = hello(&k, json!({ "kind": "device", "deviceId": "nobody", "nonce": nonce }));
    assert_eq!(stranger["proof"].as_str().unwrap().len(), 64, "same shape as a real answer");
    assert_ne!(stranger["proof"], want.as_str());
    // No live code: the same, no tell.
    let none = hello(&k, json!({ "kind": "pair", "nonce": nonce }));
    assert_eq!(none["proof"].as_str().unwrap().len(), 64);
    assert_eq!(k.call("POST", "/api/hello", &[], r#"{"kind":"pair","nonce":"short"}"#).status, 400);
    assert_eq!(k.call("POST", "/api/hello", &[], r#"{"kind":"admin","nonce":"0123456789abcdef"}"#).status, 400);
}

#[test]
fn hello_is_rate_limited_too() {
    let k = kit("hello-rate", None);
    let mut last = 0;
    for _ in 0..100 {
        last = k.call("POST", "/api/hello", &[], r#"{"kind":"pair","nonce":"0123456789abcdef"}"#).status;
    }
    assert_eq!(last, 429);
}

// ---- revoke, abilities, expiry -------------------------------------------------------------------------------------

#[test]
fn a_revoked_phone_is_out_at_once() {
    let k = kit("revoke", None);
    let (id, token) = k.pair();
    assert_eq!(k.authed(&token, "GET", "/api/me", "").status, 200);
    k.pairing.lock().revoke_phone_device(&id).unwrap();
    assert_eq!(k.authed(&token, "GET", "/api/me", "").status, 401);
}

#[test]
fn a_phone_can_sign_itself_out() {
    let k = kit("self-revoke", None);
    let (_, token) = k.pair();
    assert_eq!(k.authed(&token, "DELETE", "/api/me", "").status, 200);
    assert_eq!(k.authed(&token, "GET", "/api/me", "").status, 401);
}

#[test]
fn abilities_decide_what_a_phone_may_do() {
    let k = kit("caps", None);
    let (_, watch_only) = k.pair_with(Caps::none().with(Cap::Watch, true));
    // The service is missing here, so an allowed route answers 503; a refused one answers 403.
    assert_eq!(k.authed(&watch_only, "GET", "/api/bots", "").status, 503);
    for (method, path) in [("POST", "/api/bots/nimbus/send"), ("POST", "/api/bots/nimbus/cards/c/answer"), ("POST", "/api/bots/nimbus/stop"), ("POST", "/api/pause-all"), ("POST", "/api/resume-all")] {
        let r = k.authed(&watch_only, method, path, r#"{"text":"x","answer":"deny"}"#);
        assert_eq!(r.status, 403, "{method} {path}");
        assert_eq!(body_of(&r)["error"], "not_allowed");
    }
    let (_, answer_only) = k.pair_with(Caps::none().with(Cap::Answer, true));
    assert_eq!(k.authed(&answer_only, "GET", "/api/bots", "").status, 403);
    assert_eq!(k.authed(&answer_only, "GET", "/api/me", "").status, 200, "any phone may ask who it is");
    assert_ne!(k.authed(&answer_only, "POST", "/api/bots/nimbus/cards/c/answer", r#"{"answer":"deny"}"#).status, 403);
}

#[test]
fn a_phone_unused_for_thirty_days_is_signed_out() {
    let k = kit("idle", None);
    let (id, token) = k.pair();
    k.pairing.lock().set_device_expiry_for_test(&id, 1);
    let r = k.authed(&token, "GET", "/api/me", "");
    assert_eq!(r.status, 401);
    assert!(body_of(&r)["message"].as_str().unwrap().contains("30 days"));
}

#[test]
fn many_wrong_tokens_from_one_place_are_slowed_down() {
    let k = kit("auth-rate", None);
    let mut last = 0;
    for i in 0..400 {
        last = k.call("GET", "/api/me", &[("authorization", &format!("Bearer guess{i}"))], "").status;
    }
    assert_eq!(last, 429);
}

// ---- the app files -------------------------------------------------------------------------------------------------

#[test]
fn the_app_files_cannot_be_walked_out_of() {
    let k = kit("static", None);
    assert_eq!(k.call("GET", "/", &[], "").body, b"<!doctype html><title>Potato</title>");
    assert_eq!(k.call("GET", "/sw.js", &[], "").status, 200);
    assert!(k.call("GET", "/sw.js", &[], "").headers.iter().any(|(n, v)| n == "Service-Worker-Allowed" && v == "/"));
    for path in ["/../secret.js", "/..%2Fsecret.js", "/%2e%2e/secret.js", "/tools/make.mjs", "/.hidden.js", "/app/../secret.js", "/a\\..\\secret.js", "/C:/Windows/win.ini", "/app.js/../../secret.js", "//secret.js", "/index.html.bak", "/pairing.json"] {
        let r = k.call("GET", path, &[], "");
        assert!(r.status == 404 || (r.status == 200 && r.body == b"<!doctype html><title>Potato</title>"), "{path} -> {} {:?}", r.status, String::from_utf8_lossy(&r.body));
        assert!(!r.body.windows(7).any(|w| w == b"outside") && !r.body.windows(6).any(|w| w == b"hidden") && !r.body.windows(11).any(|w| w == b"secret tool"), "{path} leaked a file");
    }
    assert_eq!(k.call("POST", "/", &[], "").status, 405);
}

// ---- live updates ---------------------------------------------------------------------------------------------------

#[test]
fn the_long_poll_answers_at_once_when_something_changed_and_wakes_on_a_change() {
    let k = kit("poll", None);
    let (_, token) = k.pair();
    let first = body_of(&k.authed(&token, "GET", "/api/changes?since=0&wait_ms=0", ""))["rev"].as_u64().unwrap();
    assert!(first >= 1);
    let began = std::time::Instant::now();
    let waiting = {
        let (ctx, token) = (k.ctx.clone(), token.clone());
        std::thread::spawn(move || {
            let r = super::gate::dispatch(&ctx, &req("GET", &format!("/api/changes?since={first}&wait_ms=20000"), &[("authorization", &format!("Bearer {token}"))], ""));
            r.body_json()["rev"].as_u64().unwrap()
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    k.ctx.changes.bump();
    let woke = waiting.join().unwrap();
    assert!(woke > first && began.elapsed() < std::time::Duration::from_secs(5), "woke after {:?}", began.elapsed());
    let quiet = std::time::Instant::now();
    let same = body_of(&k.authed(&token, "GET", &format!("/api/changes?since={woke}&wait_ms=300"), ""))["rev"].as_u64().unwrap();
    assert_eq!(same, woke);
    assert!(quiet.elapsed() >= std::time::Duration::from_millis(250));
}

#[test]
fn version_one_paths_and_plain_paths_are_the_same_routes() {
    let k = kit("v1", None);
    let (_, token) = k.pair();
    assert_eq!(body_of(&k.authed(&token, "GET", "/api/v1/me", "")), body_of(&k.authed(&token, "GET", "/api/me", "")));
    assert_eq!(k.call("GET", "/api/v1/health", &[], "").status, 200);
    assert_eq!(k.call("GET", "/api/v2/health", &[], "").status, 401, "an unknown version is just an unknown route, and routes need a token");
    let _ = Changes::new();
}
