//! "Always allow" on approval cards, through the engine with fakes. What it shows: the button is offered for one exact,
//! non-destructive class only (never rm or force-push, never a command with two classes); the answer comes only from the
//! app's own command, so a forged `always_allow` from any other channel is refused and writes no rule; a later command
//! of that bot and that class runs with a plain "auto-allowed" line in the thread, and nothing else does; the rule
//! survives a restart; and removing it takes effect at once.

use serde_json::json;

use super::bot_settings;
use super::cards;
use super::risk;
use super::service_card_tests::{code, ok, one_card, open, rig, terminal, tool, DM};
use super::service_tests::Rig;
use super::store::{self, MsgKind};

fn rules(rig: &Rig) -> Vec<String> {
    bot_settings::load(&rig.root, "nimbus").unwrap().always_allow.iter().map(|r| r.class.clone()).collect()
}

fn card_for(rig: &Rig, id: &str, command: &str) -> cards::Card {
    assert_eq!(code(&terminal(rig, id, command)), "APPROVAL_PENDING", "{command}");
    cards::open_cards(&rig.root, "nimbus").into_iter().find(|c| c.action == command).expect("a card for the command")
}

fn card_json(rig: &Rig, card: &cards::Card) -> serde_json::Value {
    let row = rig.thread().into_iter().find(|m| m.kind == MsgKind::DecisionCard && m.request_id.as_deref() == Some(card.id.as_str())).unwrap();
    serde_json::to_value(&row).unwrap()["card"].clone()
}

#[test]
fn the_button_is_offered_for_one_ordinary_class_only() {
    let (rig, _) = rig("always-offer");
    let push = card_for(&rig, "t-1", "git push origin dev");
    assert!(push.can_always_allow);
    assert_eq!(card_json(&rig, &push)["canAlwaysAllow"], json!(true));
    // two classes in one command: not one exact class
    let both = card_for(&rig, "t-2", "git push && npm publish");
    assert!(!both.can_always_allow);
}

#[test]
fn rm_and_force_push_never_offer_it_and_the_backend_refuses_it() {
    let (rig, _) = rig("always-never");
    for (n, command) in ["rm -rf /etc/stuff", "git push --force origin main", "git reset --hard HEAD~3", "gh repo delete me/x --yes"].iter().enumerate() {
        let card = card_for(&rig, &format!("t-{n}"), command);
        assert!(!card.can_always_allow, "{command}");
        assert!(card_json(&rig, &card).get("canAlwaysAllow").is_none(), "{command}");
        let refused = rig.svc.answer_card_always("nimbus", &card.id).unwrap_err();
        assert!(refused.to_string().to_lowercase().contains("always allow"), "{refused}");
        assert_eq!(cards::find(&rig.root, "nimbus", &card.id).unwrap().state, cards::State::Open, "{command}");
    }
    assert!(rules(&rig).is_empty());
    for class in ["rm -rf", "git push --force", "git reset --hard", "gh repo delete", "drop database", "any change", "nested command", "run piped script"] {
        assert!(!risk::always_allowable(class), "{class}");
    }
    assert!(risk::always_allowable("git push") && risk::always_allowable("publish"));
}

#[test]
fn a_forged_always_answer_from_any_other_channel_is_refused() {
    let (rig, _) = rig("always-forged");
    let card = card_for(&rig, "t-1", "git push origin dev");
    // the plain answer path (the one every other button uses), in several spellings
    for forged in ["always_allow", "always", "ALWAYS_ALLOW", " always_allow "] {
        assert!(rig.svc.answer_card("nimbus", &card.id, forged).is_err(), "{forged}");
        assert_eq!(cards::answer(&rig.root, "nimbus", &card.id, forged, None, 9_000).unwrap_err().code, "POTATO_CARD_BAD_ANSWER", "{forged}");
    }
    // words from the chat are only text, and the brain has no tool that answers a card
    rig.svc.send("nimbus", "always_allow card").unwrap();
    let tried = tool(&rig, DM, "b-1", "answer_card", json!({"cardId": card.id, "answer": "always_allow"}));
    assert!(!ok(&tried), "{tried:?}");
    assert_eq!(cards::find(&rig.root, "nimbus", &card.id).unwrap().state, cards::State::Open);
    assert!(rules(&rig).is_empty());
    assert_eq!(code(&terminal(&rig, "t-2", "git push origin dev")), "APPROVAL_PENDING");
}

#[test]
fn always_allow_lets_a_later_command_of_that_class_run_and_logs_it_plainly() {
    let (rig, fake) = rig("always-apply");
    let card = card_for(&rig, "t-1", "git push origin dev");
    let done = rig.svc.answer_card_always("nimbus", &card.id).unwrap();
    assert_eq!(done.answer, "always_allow");
    assert_eq!(rules(&rig), vec!["git push"]);
    // the command that raised the card, and another of the same class, run without a card
    assert!(ok(&terminal(&rig, "t-2", "git push origin dev")));
    assert!(ok(&terminal(&rig, "t-3", "git push origin feature-x")));
    assert!(open(&rig).is_empty());
    assert_eq!(fake.calls().len(), 2);
    let notes = rig.notes();
    assert!(notes.iter().any(|n| n.contains("Auto-allowed") && n.contains("git push origin feature-x") && n.contains("pushes to a remote")), "{notes:?}");
    // only that exact class: another class, a mix, and a force push still ask
    assert_eq!(code(&terminal(&rig, "t-4", "npm publish")), "APPROVAL_PENDING");
    assert_eq!(code(&terminal(&rig, "t-5", "git push origin main && npm publish")), "APPROVAL_PENDING");
    assert_eq!(code(&terminal(&rig, "t-6", "git push --force origin dev")), "APPROVAL_PENDING");
    assert_eq!(fake.calls().len(), 2);
}

#[test]
fn a_rule_belongs_to_one_bot_and_one_class() {
    let (rig, _) = rig("always-bot");
    store::add_bot(&rig.root, "mochi", "potato", 2).unwrap();
    let card = card_for(&rig, "t-1", "git push origin dev");
    rig.svc.answer_card_always("nimbus", &card.id).unwrap();
    assert!(bot_settings::always_allows(&rig.root, "nimbus", &["git push"]));
    assert!(!bot_settings::always_allows(&rig.root, "mochi", &["git push"]));
    assert!(!bot_settings::always_allows(&rig.root, "nimbus", &["publish"]));
    assert!(!bot_settings::always_allows(&rig.root, "nimbus", &["git push", "publish"]));
    assert!(!bot_settings::always_allows(&rig.root, "nimbus", &[]));
}

#[test]
fn a_rule_for_a_never_class_in_the_file_is_not_applied() {
    let (rig, fake) = rig("always-edited");
    bot_settings::update(&rig.root, "nimbus", |s| {
        s.always_allow = vec![bot_settings::AlwaysRule { class: "rm -rf".into(), at: 1 }, bot_settings::AlwaysRule { class: "git push --force".into(), at: 2 }];
        Ok(())
    })
    .unwrap();
    assert_eq!(code(&terminal(&rig, "t-1", "rm -rf /etc/stuff")), "APPROVAL_PENDING");
    assert_eq!(code(&terminal(&rig, "t-2", "git push --force origin main")), "APPROVAL_PENDING");
    assert!(fake.calls().is_empty());
}

#[test]
fn the_rule_is_saved_and_survives_a_restart() {
    let (rig, _) = rig("always-saved");
    let card = card_for(&rig, "t-1", "git push origin dev");
    rig.svc.answer_card_always("nimbus", &card.id).unwrap();
    let text = std::fs::read_to_string(bot_settings::path(&rig.root, "nimbus").unwrap()).unwrap();
    assert!(text.contains("alwaysAllow") && text.contains("git push"), "{text}");
    assert_eq!(bot_settings::load(&rig.root, "nimbus").unwrap().always_allow.len(), 1);
    let options = rig.svc.bot_options("nimbus").unwrap();
    assert_eq!(options.always_allow.len(), 1);
    assert_eq!(options.always_allow[0].class, "git push");
    assert_eq!(options.always_allow[0].description, "pushes to a remote");
    // the same card cannot be answered again, and a second rule is not added
    assert!(rig.svc.answer_card_always("nimbus", &card.id).is_err());
    assert_eq!(rules(&rig).len(), 1);
}

#[test]
fn removing_a_rule_works_and_takes_effect_at_once() {
    let (rig, fake) = rig("always-remove");
    let card = card_for(&rig, "t-1", "git push origin dev");
    rig.svc.answer_card_always("nimbus", &card.id).unwrap();
    assert!(ok(&terminal(&rig, "t-2", "git push origin one")));
    let after = rig.svc.remove_always_rule("nimbus", "git push").unwrap();
    assert!(after.always_allow.is_empty());
    assert!(rules(&rig).is_empty());
    assert_eq!(code(&terminal(&rig, "t-3", "git push origin two")), "APPROVAL_PENDING");
    assert_eq!(fake.calls().len(), 1);
    // removing what is not there is harmless
    assert!(rig.svc.remove_always_rule("nimbus", "git push").is_ok());
}

#[test]
fn always_allow_is_refused_on_a_question_and_on_a_card_that_does_not_exist() {
    let (rig, _) = rig("always-bad");
    let asked = tool(&rig, DM, "q-1", "ask_user", json!({"question": "Which?", "choices": ["a", "b"]}));
    assert!(ok(&asked));
    let q = one_card(&rig);
    assert!(!q.can_always_allow);
    assert!(rig.svc.answer_card_always("nimbus", &q.id).is_err());
    assert!(rig.svc.answer_card_always("nimbus", "card-000000000000").is_err());
    assert!(rules(&rig).is_empty());
}
