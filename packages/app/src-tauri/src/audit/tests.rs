use super::*;

const T0: u64 = 1_791_979_200_000; // 2026-10-14 12:00 UTC

#[test]
fn month_names_come_from_utc_dates() {
    assert_eq!(month_of(0), "1970-01");
    assert_eq!(month_of(T0), "2026-10");
    assert_eq!(month_of(1_793_491_201_000), "2026-11");
    assert_eq!(month_of(1_709_251_140_000), "2024-02", "a leap day");
}

#[test]
fn keep_going_hand_over_has_its_own_audit_kind() {
    assert_eq!(Kind::KeepGoing.as_str(), "keep_going.handed_over");
}

#[test]
fn channel_kinds_have_stable_names() {
    assert_eq!(Kind::ChannelPaired.as_str(), "channel.paired");
    assert_eq!(Kind::ChannelUnpaired.as_str(), "channel.unpaired");
    assert_eq!(Kind::ChannelOn.as_str(), "channel.on");
    assert_eq!(Kind::ChannelOff.as_str(), "channel.off");
    assert_eq!(Kind::ChannelAnswer.as_str(), "channel.answer");
    assert_eq!(Kind::ChannelMessage.as_str(), "channel.message");
    assert_eq!(Kind::ChannelStranger.as_str(), "channel.stranger");
    assert_eq!(Kind::ChannelError.as_str(), "channel.error");
}

#[test]
fn issue_intake_has_its_own_audit_kind() {
    assert_eq!(Kind::IssueIntake.as_str(), "issue.intake");
}

#[test]
fn red_to_green_has_its_own_audit_kind() {
    assert_eq!(Kind::CiFix.as_str(), "ci.fix");
}

#[test]
fn a_channel_actor_is_its_name() {
    assert_eq!(Who::Channel("telegram".into()).parts(), ("channel", Some("telegram".to_string())));
    assert_eq!(Who::Desktop.parts(), ("desktop", None), "other actors are unchanged");
}

#[test]
fn night_shift_kind_has_a_stable_name() {
    assert_eq!(Kind::NightShift.as_str(), "night_shift");
}

#[test]
fn review_desk_kind_has_a_stable_name() {
    assert_eq!(Kind::ReviewDesk.as_str(), "review_desk.action");
}

#[test]
fn focus_kind_has_a_stable_name() {
    assert_eq!(Kind::Focus.as_str(), "focus");
}

#[test]
fn what_broke_it_kind_has_a_stable_name() {
    assert_eq!(Kind::WhatBrokeIt.as_str(), "what_broke_it.undone");
}

