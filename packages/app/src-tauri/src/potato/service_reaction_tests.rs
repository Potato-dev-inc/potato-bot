//! Reactions through the engine: the owner's app command toggles one on a real message of the bot's thread, the
//! listing carries it on the message, an unknown message or emoji is refused, and nothing reaches the model.

use super::service_tests::Rig;

fn first_id(rig: &Rig) -> String {
    rig.thread().into_iter().next().expect("a message").id
}

#[test]
fn a_reaction_shows_on_the_message_and_toggles_off() {
    let rig = Rig::ready("react-toggle");
    let id = first_id(&rig);
    assert!(rig.thread().iter().all(|m| m.reactions.is_empty()));
    assert_eq!(rig.svc.toggle_reaction("nimbus", &id, "🎉").unwrap(), vec!["🎉".to_string()]);
    let shown = rig.thread().into_iter().find(|m| m.id == id).unwrap();
    assert_eq!(shown.reactions, vec!["🎉".to_string()]);
    assert_eq!(serde_json::to_value(&shown).unwrap()["reactions"], serde_json::json!(["🎉"]));
    assert!(rig.svc.toggle_reaction("nimbus", &id, "🎉").unwrap().is_empty());
    let cleared = rig.thread().into_iter().find(|m| m.id == id).unwrap();
    assert!(cleared.reactions.is_empty());
    assert!(serde_json::to_value(&cleared).unwrap().get("reactions").is_none());
}

#[test]
fn an_unknown_message_or_emoji_is_refused() {
    let rig = Rig::ready("react-refuse");
    let id = first_id(&rig);
    assert_eq!(rig.svc.toggle_reaction("nimbus", "m-nothing", "👍").unwrap_err().code, "POTATO_MESSAGE_NOT_FOUND");
    assert_eq!(rig.svc.toggle_reaction("nimbus", &id, "💀").unwrap_err().code, "POTATO_REACTION_INVALID");
    assert_eq!(rig.svc.toggle_reaction("ghost", &id, "👍").unwrap_err().code, "POTATO_BOT_NOT_FOUND");
    assert!(rig.thread().iter().all(|m| m.reactions.is_empty()));
}

#[test]
fn reactions_stay_in_the_files_and_out_of_the_conversation() {
    let rig = Rig::ready("react-files");
    let id = first_id(&rig);
    rig.svc.toggle_reaction("nimbus", &id, "❤️").unwrap();
    let raw = super::store::read_messages(&rig.root, &super::store::app_conversation("nimbus"), 500, None).unwrap();
    assert!(raw.iter().all(|m| m.reactions.is_empty()), "the conversation file holds no reactions");
    assert!(rig.root.join("reactions.json").exists());
}
