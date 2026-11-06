//! `render_plot`: the bot passes numbers, the person sees a bar, line or pie chart in the chat. Nothing is drawn here
//! and no image is made: the message keeps the checked numbers (`Message.chart`) and the app draws them as an SVG
//! with its own code, so a label can never become markup and nothing leaves the computer.
//!
//! When to ask: this is not a card at any level. It reads nothing, runs nothing and sends nothing; it only puts the
//! bot's own chart into the owner's private chat. The caller checks that the turn is the owner's.
//!
//! Limits (refused with one plain sentence, never cut): 200 points (12 for a pie), a title of 80 characters, a label of
//! 40, numbers that are finite and below a trillion. Secrets and the home folder are taken out of the title and labels.

use std::path::Path;

use serde_json::{json, Value};

use crate::potato::store::{self, ChartCard, ChartKind, ChartPoint, FromKind, Message, MsgKind, StoreError, StoreResult};

pub const TOOL: &str = "render_plot";
pub const MAX_POINTS: usize = 200;
pub const MAX_PIE_POINTS: usize = 12;
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_LABEL_CHARS: usize = 40;
pub const MAX_ABS_VALUE: f64 = 1e12;
const BAD_ARGS: &str = "POTATO_PLOT_BAD_ARGS";

pub fn is_tool(name: &str) -> bool {
    name == TOOL
}

fn bad(text: impl Into<String>) -> StoreError {
    StoreError::new(BAD_ARGS, text)
}

fn plain(text: &str, home: Option<&str>) -> String {
    let no_controls: String = text.chars().filter(|c| !c.is_control()).collect();
    store::clean_text(no_controls.trim(), home)
}

/// Checks the bot's arguments and returns the chart to keep. The first problem found is the answer.
pub fn check(args: &Value, home: Option<&str>) -> StoreResult<ChartCard> {
    let kind = match args.get("type").and_then(Value::as_str).map(str::trim) {
        Some("bar") => ChartKind::Bar,
        Some("line") => ChartKind::Line,
        Some("pie") => ChartKind::Pie,
        _ => return Err(bad("Say which chart: type must be \"bar\", \"line\" or \"pie\".")),
    };
    let raw_title = args.get("title").and_then(Value::as_str).unwrap_or("");
    if raw_title.chars().count() > MAX_TITLE_CHARS {
        return Err(bad(format!("Use a title of {MAX_TITLE_CHARS} characters or fewer.")));
    }
    let title = plain(raw_title, home);
    let Some(list) = args.get("points").and_then(Value::as_array) else {
        return Err(bad("Give points: a list of {\"label\": text, \"value\": number}."));
    };
    let most = if kind == ChartKind::Pie { MAX_PIE_POINTS } else { MAX_POINTS };
    if list.is_empty() || list.len() > most {
        return Err(bad(format!("Give between 1 and {most} points for this chart.")));
    }
    if kind == ChartKind::Line && list.len() < 2 {
        return Err(bad("A line needs at least 2 points."));
    }
    let mut points = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let n = i + 1;
        let raw_label = item.get("label").and_then(Value::as_str).unwrap_or("");
        if raw_label.chars().count() > MAX_LABEL_CHARS {
            return Err(bad(format!("Point {n}: use a label of {MAX_LABEL_CHARS} characters or fewer.")));
        }
        let label = plain(raw_label, home);
        if label.is_empty() {
            return Err(bad(format!("Point {n} needs a label.")));
        }
        let Some(value) = item.get("value").and_then(Value::as_f64) else {
            return Err(bad(format!("Point {n}: value must be a number.")));
        };
        if !value.is_finite() || value.abs() > MAX_ABS_VALUE {
            return Err(bad(format!("Point {n}: value must be a number below a trillion.")));
        }
        if kind == ChartKind::Pie && value < 0.0 {
            return Err(bad(format!("Point {n}: a pie cannot have a negative value.")));
        }
        points.push(ChartPoint { label, value });
    }
    if kind == ChartKind::Pie && points.iter().map(|p| p.value).sum::<f64>() <= 0.0 {
        return Err(bad("A pie needs at least one value above zero."));
    }
    Ok(ChartCard { kind, title, points })
}

/// Puts the chart into the owner's private chat with this bot as a bot message. Other conversations are refused.
pub fn call(root: &Path, bot: &str, conversation: &str, who: &str, home: Option<&str>, args: &Value, now: i64) -> StoreResult<Value> {
    if conversation != store::app_conversation(bot) {
        return Err(StoreError::new("POTATO_PLOT_WHERE", "A chart can only be shown in your private chat with your owner."));
    }
    let chart = check(args, home)?;
    let text = if chart.title.is_empty() { "Sent a chart".to_string() } else { format!("Chart: {}", chart.title) };
    let count = chart.points.len();
    let mut message = Message::new(now, FromKind::Bot, Some(who), MsgKind::Text, &text);
    message.chart = Some(chart);
    store::append_message(root, conversation, message, home)?;
    Ok(json!({
        "ok": true,
        "points": count,
        "note": "The chart is now in the chat. Say what it shows in a line or two; do not repeat all the numbers.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pm-potato-chart-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        store::add_bot(&dir, "nimbus", "fish", 1).unwrap();
        dir
    }
    fn pts(n: usize) -> Value {
        Value::Array((0..n).map(|i| json!({ "label": format!("p{i}"), "value": i as f64 })).collect())
    }
    fn refused(args: Value) -> String {
        check(&args, None).unwrap_err().message
    }

    #[test]
    fn a_good_chart_of_each_kind_is_kept() {
        for kind in ["bar", "line", "pie"] {
            let c = check(&json!({ "type": kind, "title": "Sales", "points": [{"label":"a","value":1},{"label":"b","value":2.5}] }), None).unwrap();
            assert_eq!(c.points.len(), 2);
            assert_eq!(c.title, "Sales");
        }
    }

    #[test]
    fn bad_input_gets_one_plain_sentence() {
        assert!(refused(json!({ "type": "donut", "points": pts(2) })).contains("bar"));
        assert!(refused(json!({ "type": "bar" })).contains("points"));
        assert!(refused(json!({ "type": "bar", "points": [] })).contains("between 1 and"));
        assert!(refused(json!({ "type": "bar", "points": pts(201) })).contains("200"));
        assert!(refused(json!({ "type": "pie", "points": pts(13) })).contains("12"));
        assert!(refused(json!({ "type": "line", "points": pts(1) })).contains("2 points"));
        assert!(refused(json!({ "type": "bar", "title": "x".repeat(81), "points": pts(2) })).contains("80"));
        assert!(refused(json!({ "type": "bar", "points": [{"label":"y".repeat(41),"value":1}] })).contains("40"));
        assert!(refused(json!({ "type": "bar", "points": [{"label":"  ","value":1}] })).contains("label"));
        assert!(refused(json!({ "type": "bar", "points": [{"label":"a","value":"7"}] })).contains("number"));
        assert!(refused(json!({ "type": "bar", "points": [{"label":"a","value":1e13}] })).contains("trillion"));
        assert!(refused(json!({ "type": "bar", "points": [{"label":"a"}] })).contains("number"));
        assert!(refused(json!({ "type": "pie", "points": [{"label":"a","value":-1},{"label":"b","value":3}] })).contains("negative"));
        assert!(refused(json!({ "type": "pie", "points": [{"label":"a","value":0}] })).contains("above zero"));
    }

    #[test]
    fn two_hundred_points_are_fine_and_labels_lose_control_characters() {
        assert_eq!(check(&json!({ "type": "line", "points": pts(200) }), None).unwrap().points.len(), 200);
        let c = check(&json!({ "type": "bar", "title": "T\u{0007}x", "points": [{"label":"<b>\n\u{0}hi</b>","value":1}] }), None).unwrap();
        assert_eq!(c.title, "Tx");
        assert_eq!(c.points[0].label, "<b>hi</b>");
    }

    #[test]
    fn a_secret_in_a_label_is_taken_out() {
        let c = check(&json!({ "type": "bar", "points": [{"label":"sk-abcdefghijklmnopqrstuvwxyz0123456789","value":1}] }), None).unwrap();
        assert!(!c.points[0].label.contains("abcdefghijklmnop"), "{}", c.points[0].label);
    }

    #[test]
    fn the_chart_lands_in_the_owner_chat_as_a_bot_message_and_only_there() {
        let root = temp("land");
        let args = json!({ "type": "pie", "title": "Split", "points": [{"label":"a","value":1},{"label":"b","value":3}] });
        let out = call(&root, "nimbus", "app:dm:nimbus", "Nimbus", None, &args, 5).unwrap();
        assert_eq!(out["points"], 2);
        let all = store::read_messages(&root, "app:dm:nimbus", 10, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "Chart: Split");
        assert_eq!(all[0].chart.as_ref().unwrap().kind, ChartKind::Pie);
        let wire = serde_json::to_value(&all[0]).unwrap();
        assert_eq!(wire["chart"]["kind"], "pie");
        assert_eq!(wire["chart"]["points"][1]["value"], 3.0);
        // a group or another bot's chat is refused and writes nothing
        assert!(call(&root, "nimbus", "app:room:r000000000001", "Nimbus", None, &args, 6).is_err());
        assert!(call(&root, "nimbus", "app:dm:other", "Nimbus", None, &args, 6).is_err());
        assert_eq!(store::read_messages(&root, "app:dm:nimbus", 10, None).unwrap().len(), 1);
    }
}
