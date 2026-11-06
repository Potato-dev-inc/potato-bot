//! The cron schedule: parsing (lists, ranges, steps, names), the next runs, the 15 minute floor, and the plain-English
//! errors. Pure: no clock, no files.

use chrono::NaiveDateTime;

use super::cron::{check, next_runs, Cron};

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
}

fn next(expr: &str, after: &str) -> Option<String> {
    Cron::parse(expr).unwrap().next_after(at(after)).map(|t| t.format("%Y-%m-%d %H:%M").to_string())
}

fn bad(expr: &str) -> String {
    match check(expr) {
        Ok(_) => panic!("{expr:?} should have been refused"),
        Err(message) => message,
    }
}

#[test]
fn parse_reads_stars_lists_ranges_and_steps() {
    let cron = Cron::parse("0,30 9-17/2 * * *").unwrap();
    // 9, 11, 13, 15, 17 at :00 and :30.
    assert!(cron.matches(at("2026-11-09 09:00:00")));
    assert!(cron.matches(at("2026-11-09 11:30:00")));
    assert!(cron.matches(at("2026-11-09 17:30:00")));
    assert!(!cron.matches(at("2026-11-09 10:00:00")));
    assert!(!cron.matches(at("2026-11-09 09:15:00")));
    assert!(!cron.matches(at("2026-11-09 19:00:00")));
    let every = Cron::parse("*/20 * * * *").unwrap();
    assert!(every.matches(at("2026-11-09 04:40:00")));
    assert!(!every.matches(at("2026-11-09 04:41:00")));
    // A start with a step runs to the end of the field: 10, 25, 40, 55.
    let from = Cron::parse("10/15 * * * *").unwrap();
    assert!(from.matches(at("2026-11-09 04:55:00")));
    assert!(!from.matches(at("2026-11-09 04:05:00")));
}

#[test]
fn names_work_for_months_and_weekdays_in_any_case_and_sunday_is_zero_or_seven() {
    let cron = Cron::parse("0 8 1 JAN,jul mon-FRI").unwrap();
    assert!(cron.matches(at("2026-07-01 08:00:00"))); // Wednesday the 1st
    assert!(Cron::parse("0 8 * * sun").unwrap().matches(at("2026-11-08 08:00:00")));
    assert!(Cron::parse("0 8 * * 0").unwrap().matches(at("2026-11-08 08:00:00")));
    assert!(Cron::parse("0 8 * * 7").unwrap().matches(at("2026-11-08 08:00:00")));
    assert!(Cron::parse("0 8 * * 5-7").unwrap().matches(at("2026-11-08 08:00:00")));
}

#[test]
fn next_run_is_strictly_after_the_moment_given() {
    assert_eq!(next("0 9 * * *", "2026-11-09 08:59:30").as_deref(), Some("2026-11-09 09:00"));
    assert_eq!(next("0 9 * * *", "2026-11-09 09:00:00").as_deref(), Some("2026-11-10 09:00"));
    assert_eq!(next("0 9 * * *", "2026-11-09 09:00:30").as_deref(), Some("2026-11-10 09:00"));
    assert_eq!(next("*/30 * * * *", "2026-11-09 23:45:00").as_deref(), Some("2026-11-10 00:00"));
}

#[test]
fn weekday_cron_skips_the_weekend() {
    // Friday 6 November 2026 evening: the next weekday 9am is Monday.
    assert_eq!(next("0 9 * * 1-5", "2026-11-06 10:00:00").as_deref(), Some("2026-11-09 09:00"));
}

#[test]
fn day_of_month_and_day_of_week_both_given_means_either_one() {
    // The 13th, or any Friday.
    assert_eq!(next("0 0 13 * fri", "2026-11-06 12:00:00").as_deref(), Some("2026-11-13 00:00"));
    assert_eq!(next("0 0 13 * fri", "2026-11-13 12:00:00").as_deref(), Some("2026-11-20 00:00"));
    assert_eq!(next("0 0 13 * fri", "2026-11-27 12:00:00").as_deref(), Some("2026-12-04 00:00"));
    assert_eq!(next("0 0 13 * fri", "2026-12-04 12:00:00").as_deref(), Some("2026-12-11 00:00"));
    // The 13th is a Sunday in 2026-12? It is a Sunday: 13 Dec 2026.
    assert_eq!(next("0 0 13 * fri", "2026-12-11 12:00:00").as_deref(), Some("2026-12-13 00:00"));
}

#[test]
fn a_leap_day_waits_for_the_next_leap_year() {
    assert_eq!(next("0 12 29 2 *", "2026-03-01 00:00:00").as_deref(), Some("2028-02-29 12:00"));
}

#[test]
fn next_runs_lists_the_coming_runs_in_order() {
    let runs = next_runs("0 9 * * mon-fri", at("2026-11-06 10:00:00"), 3).unwrap();
    let text: Vec<String> = runs.iter().map(|t| t.format("%a %m-%d %H:%M").to_string()).collect();
    assert_eq!(text, ["Mon 11-09 09:00", "Tue 11-10 09:00", "Wed 11-11 09:00"]);
}

#[test]
fn the_floor_is_fifteen_minutes() {
    assert!(check("*/15 * * * *").is_ok());
    assert!(check("0 * * * *").is_ok());
    assert!(check("0,15,30,45 * * * *").is_ok());
    let every_five = bad("*/5 * * * *");
    assert!(every_five.contains("15 minutes"), "{every_five}");
    assert!(every_five.contains("5 minutes"), "{every_five}");
    assert!(bad("* * * * *").contains("15 minutes"));
    assert!(bad("0,10 * * * *").contains("10 minutes"));
    assert!(bad("0,14 9 * * *").contains("14 minutes"));
}

#[test]
fn the_floor_also_counts_across_midnight_but_only_on_days_that_follow_each_other() {
    // 23:55 then 00:05 the next day: 10 minutes apart.
    let across = bad("5,55 0,23 * * *");
    assert!(across.contains("10 minutes"), "{across}");
    // The same times only on Mondays: the next day is never a run day, so the closest runs are 50 minutes apart.
    assert!(check("5,55 0,23 * * mon").is_ok());
    // An hour apart over midnight is fine.
    assert!(check("0 23,0 * * *").is_ok());
}

#[test]
fn a_schedule_that_never_happens_is_refused_in_words() {
    let message = bad("0 0 31 2 *");
    assert!(message.contains("never"), "{message}");
}

#[test]
fn bad_input_says_what_is_wrong() {
    assert!(bad("").contains("5 parts"));
    assert!(bad("   ").contains("5 parts"));
    let four = bad("0 9 * *");
    assert!(four.contains("5 parts") && four.contains("4"), "{four}");
    let six = bad("0 0 9 * * *");
    assert!(six.contains("5 parts") && six.contains("6"), "{six}");
    let minute = bad("61 * * * *");
    assert!(minute.contains("minute") && minute.contains("61") && minute.contains("0 to 59"), "{minute}");
    let hour = bad("0 24 * * *");
    assert!(hour.contains("hour") && hour.contains("0 to 23"), "{hour}");
    let dom = bad("0 9 0 * *");
    assert!(dom.contains("day of the month") && dom.contains("1 to 31"), "{dom}");
    let month = bad("0 9 * 13 *");
    assert!(month.contains("month") && month.contains("1 to 12"), "{month}");
    let dow = bad("0 9 * * 8");
    assert!(dow.contains("day of the week") && dow.contains("0 to 7"), "{dow}");
    let word = bad("0 9 * * funday");
    assert!(word.contains("funday") && word.contains("day of the week"), "{word}");
    let zero = bad("*/0 * * * *");
    assert!(zero.contains("step") && zero.contains("1 or more"), "{zero}");
    let backwards = bad("0 17-9 * * *");
    assert!(backwards.contains("17-9") && backwards.contains("backwards"), "{backwards}");
    let empty = bad("0,,30 * * * *");
    assert!(empty.contains("empty"), "{empty}");
    assert!(bad("0 9 * * 1-2-3").contains("1-2-3"));
    assert!(bad("*/x * * * *").contains("x"));
    let shortcut = bad("@daily");
    assert!(shortcut.contains("@daily") && shortcut.contains("5 parts"), "{shortcut}");
    let question = bad("0 9 ? * *");
    assert!(question.contains('?'), "{question}");
}

#[test]
fn extra_spaces_between_the_parts_are_fine() {
    assert!(check("  0   9 *  * mon-fri ").is_ok());
}
