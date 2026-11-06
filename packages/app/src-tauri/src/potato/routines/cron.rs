//! A small standard cron reader for routines: five parts (minute, hour, day of the month, month, day of the week).
//!
//! - Each part takes `*`, a number, a list (`1,15`), a range (`9-17`), a step (`*/10`, `9-17/2`, `5/15`), or a mix.
//!   Months and weekdays also take names (`jan`, `mon-fri`); Sunday is 0 or 7.
//! - When both the day of the month and the day of the week are given (neither starts with `*`), a day matches if
//!   EITHER does, as in classic cron.
//! - Local wall-clock time only, like the other schedules. No zone is stored.
//! - The 15 minute floor of the other schedules holds: `check` refuses a cron whose runs can be closer than that,
//!   counting across midnight, and one that never happens.
//!
//! Errors are plain sentences for the person who typed the cron. Nothing here reads a clock or a file.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};

/// The closest two runs may be, in minutes (the same floor as "every few minutes").
pub const FLOOR_MINUTES: u32 = 15;
/// How far the day scan goes: long enough to reach the next 29 February.
const SCAN_DAYS: i64 = 366 * 9;
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const SHAPE: &str = "5 parts: minute, hour, day of the month, month, day of the week (for example 0 9 * * mon-fri)";

/// A parsed cron. Each part is a bit set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    days_star: bool,
    weekdays_star: bool,
}

struct Part {
    label: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
    name_base: u32,
}

const MINUTE: Part = Part { label: "minute", min: 0, max: 59, names: &[], name_base: 0 };
const HOUR: Part = Part { label: "hour", min: 0, max: 23, names: &[], name_base: 0 };
const DAY: Part = Part { label: "day of the month", min: 1, max: 31, names: &[], name_base: 0 };
const MONTH: Part = Part { label: "month", min: 1, max: 12, names: &MONTHS, name_base: 1 };
const WEEKDAY: Part = Part { label: "day of the week", min: 0, max: 7, names: &WEEKDAYS, name_base: 0 };

fn value(part: &Part, text: &str, whole: &str) -> Result<u32, String> {
    let lower = text.to_ascii_lowercase();
    let found = if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        match text.parse::<u32>() {
            Ok(n) => n,
            Err(_) => return Err(format!("The {} part \"{whole}\" has a number that is too big. Use {} to {}.", part.label, part.min, part.max)),
        }
    } else if let Some(i) = part.names.iter().position(|n| *n == lower) {
        i as u32 + part.name_base
    } else {
        let hint = if part.names.is_empty() { "a number" } else { "a number or a name like jan or mon" };
        return Err(format!("The {} part \"{whole}\" has \"{text}\", which is not {hint}.", part.label));
    };
    if found < part.min || found > part.max {
        return Err(format!("The {} part \"{whole}\" has {text}, which is outside {} to {}.", part.label, part.min, part.max));
    }
    Ok(found)
}

/// One part to a bit set (bit n = value n) and whether it started with `*`.
fn parse_part(part: &Part, text: &str) -> Result<(u64, bool), String> {
    let mut mask = 0u64;
    for item in text.split(',') {
        if item.is_empty() {
            return Err(format!("The {} part \"{text}\" has an empty entry between commas.", part.label));
        }
        let (base, step) = match item.split_once('/') {
            Some((base, step)) => (base, Some(step)),
            None => (item, None),
        };
        let step = match step {
            None => 1,
            Some(raw) => match raw.parse::<u32>() {
                Ok(0) => return Err(format!("The {} part \"{text}\" has a step of 0. A step must be 1 or more.", part.label)),
                Ok(n) => n,
                Err(_) => return Err(format!("The {} part \"{text}\" has a step \"{raw}\" that is not a number.", part.label)),
            },
        };
        let (from, to) = if base == "*" {
            (part.min, part.max)
        } else if let Some((a, b)) = base.split_once('-') {
            if b.contains('-') {
                return Err(format!("The {} part \"{text}\" has \"{base}\", which is not a range like 9-17.", part.label));
            }
            let (a, b) = (value(part, a, text)?, value(part, b, text)?);
            if a > b {
                return Err(format!("The {} range {base} runs backwards. Write the smaller one first.", part.label));
            }
            (a, b)
        } else {
            let single = value(part, base, text)?;
            if item.contains('/') {
                (single, part.max)
            } else {
                (single, single)
            }
        };
        let mut n = from;
        while n <= to {
            mask |= 1u64 << n;
            n += step;
        }
    }
    Ok((mask, text.starts_with('*')))
}

impl Cron {
    /// Reads the five parts. Only the shape is checked here; `check` adds the floor.
    pub fn parse(text: &str) -> Result<Cron, String> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.first().is_some_and(|p| p.starts_with('@')) {
            return Err(format!("Shortcuts like {} are not supported. Write the {SHAPE}.", parts[0]));
        }
        if parts.len() != 5 {
            return Err(format!("A cron needs {SHAPE}. This has {}.", parts.len()));
        }
        let (minutes, _) = parse_part(&MINUTE, parts[0])?;
        let (hours, _) = parse_part(&HOUR, parts[1])?;
        let (days, days_star) = parse_part(&DAY, parts[2])?;
        let (months, _) = parse_part(&MONTH, parts[3])?;
        let (mut weekdays, weekdays_star) = parse_part(&WEEKDAY, parts[4])?;
        // Sunday is written 0 or 7.
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays & !(1 << 7)) | 1;
        }
        Ok(Cron { minutes, hours: hours as u32, days: days as u32, months: months as u16, weekdays: weekdays as u8, days_star, weekdays_star })
    }

    fn day_matches(&self, day: NaiveDate) -> bool {
        if self.months & (1 << day.month()) == 0 {
            return false;
        }
        let of_month = self.days & (1 << day.day()) != 0;
        let of_week = self.weekdays & (1 << day.weekday().num_days_from_sunday()) != 0;
        if self.days_star || self.weekdays_star {
            of_month && of_week
        } else {
            of_month || of_week
        }
    }

    /// True when a run is due at that minute (the seconds are ignored). Used by the tests.
    #[cfg(test)]
    pub fn matches(&self, at: NaiveDateTime) -> bool {
        self.day_matches(at.date()) && self.hours & (1 << at.hour()) != 0 && self.minutes & (1 << at.minute()) != 0
    }

    /// The first time of day at or after `from` that the hour and minute parts allow.
    fn first_time_from(&self, from: NaiveTime) -> Option<NaiveTime> {
        for hour in from.hour()..24 {
            if self.hours & (1 << hour) == 0 {
                continue;
            }
            let lowest = if hour == from.hour() { from.minute() } else { 0 };
            if let Some(minute) = (lowest..60).find(|m| self.minutes & (1 << m) != 0) {
                return NaiveTime::from_hms_opt(hour, minute, 0);
            }
        }
        None
    }

    /// The first run strictly after `after` (a whole minute later at the soonest), or None when it never runs again.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let start = after.with_second(0)?.with_nanosecond(0)? + TimeDelta::minutes(1);
        let mut day = start.date();
        for i in 0..SCAN_DAYS {
            if self.day_matches(day) {
                let from = if i == 0 { start.time() } else { NaiveTime::MIN };
                if let Some(time) = self.first_time_from(from) {
                    return Some(day.and_time(time));
                }
            }
            day = day.succ_opt()?;
        }
        None
    }

    /// The closest two runs can be, in minutes, with the two minutes-of-day that show it.
    fn closest(&self) -> Option<(u32, u32, u32)> {
        let mut times: Vec<u32> = Vec::new();
        for hour in 0..24 {
            for minute in 0..60 {
                if self.hours & (1 << hour) != 0 && self.minutes & (1 << minute) != 0 {
                    times.push(hour * 60 + minute);
                }
            }
        }
        let (first, last) = (*times.first()?, *times.last()?);
        let mut best: Option<(u32, u32, u32)> = None;
        for pair in times.windows(2) {
            let gap = pair[1] - pair[0];
            if best.is_none_or(|(g, _, _)| gap < g) {
                best = Some((gap, pair[0], pair[1]));
            }
        }
        // Midnight: the last run of one day and the first of the next, if two days in a row can both run.
        let mut day = NaiveDate::from_ymd_opt(2026, 1, 1)?;
        for _ in 0..SCAN_DAYS {
            let tomorrow = day.succ_opt()?;
            if self.day_matches(day) && self.day_matches(tomorrow) {
                let gap = 1440 - (last - first);
                if best.is_none_or(|(g, _, _)| gap < g) {
                    best = Some((gap, last, first));
                }
                break;
            }
            day = tomorrow;
        }
        best
    }
}

fn clock(minutes_of_day: u32) -> String {
    format!("{:02}:{:02}", minutes_of_day / 60, minutes_of_day % 60)
}

/// Reads the cron and applies the rules for saving it: it must run at some point, and no two runs may be closer
/// than 15 minutes.
pub fn check(text: &str) -> Result<Cron, String> {
    let cron = Cron::parse(text)?;
    let reference = NaiveDate::from_ymd_opt(2026, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).ok_or("Could not read the date.")?;
    if cron.next_after(reference).is_none() {
        return Err("That schedule never happens (for example, a day that is not in that month). Check the day and the month.".to_string());
    }
    if let Some((gap, a, b)) = cron.closest() {
        if gap < FLOOR_MINUTES {
            let s = if gap == 1 { "" } else { "s" };
            return Err(format!(
                "That schedule can run twice only {gap} minute{s} apart (for example at {} and {}). Runs must be at least {FLOOR_MINUTES} minutes apart.",
                clock(a),
                clock(b)
            ));
        }
    }
    Ok(cron)
}

/// The next `count` runs after `after`, for the preview under the cron box.
pub fn next_runs(text: &str, after: NaiveDateTime, count: usize) -> Result<Vec<NaiveDateTime>, String> {
    let cron = check(text)?;
    let mut out = Vec::new();
    let mut from = after;
    while out.len() < count {
        let Some(next) = cron.next_after(from) else { break };
        out.push(next);
        from = next;
    }
    Ok(out)
}
