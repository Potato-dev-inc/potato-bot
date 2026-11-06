//! The hard rule: a companion never answers a card. Not its own, and above all not its helpers'.
//!
//! A companion can start helpers (`run_agent`) and so, by the usual ownership rules, it "owns" their
//! panes. That ownership must never turn into the right to say yes to a helper's trust, permission or
//! question card: those go to the person, always. The companion may SEE that a helper is waiting (the
//! waiting status of `wait_agents` / `list_agents`); it has no tool and no route to answer.
//!
//! The bridge router calls `check` for every MCP request, with whether the caller is a companion
//! (`caller::Verdict::suspected`: where it runs, or where it says it runs). The routes are the ones that
//! answer a dialog or press a key into a pane. The person's own answers (the desktop app, a paired
//! phone) are not MCP requests and never come through here.

use crate::mcp_sessions::route_matches;

/// `(method, route)` of every catalog route that answers a prompt, a permission or a question, or types
/// into a pane (which could do the same). `answer_prompt`, `reply_opencode_permission`,
/// `reply_opencode_question`, `write_terminal_input`, `press_key`.
pub const ANSWERING_ROUTES: &[(&str, &str)] = &[
    ("POST", "/agents/answer"),
    ("POST", "/panes/{pane_id}/opencode/permissions/{request_id}/reply"),
    ("POST", "/panes/{pane_id}/opencode/question/reply"),
    ("POST", "/panes/{pane_id}/input"),
    ("POST", "/panes/{pane_id}/key"),
];

/// Whether this request is one that answers a card or types into a pane.
pub fn is_answering_route(method: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    ANSWERING_ROUTES.iter().any(|(m, template)| *m == method && route_matches(template, path))
}

/// Whether this request answers or dismisses a card, or types into a pane, by an MCP tool route or a loopback one.
/// The loopback handlers also take extra path segments (`/panes/{id}/key/...`), so they are matched by their first
/// segments, not by the exact route.
pub fn is_card_route(method: &str, path: &str) -> bool {
    if is_answering_route(method, path) {
        return true;
    }
    if method != "POST" {
        return false;
    }
    let route = path.split('?').next().unwrap_or(path);
    let parts: Vec<&str> = route.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        ["questions", _, "answer" | "dismiss", ..] => true,
        ["panes", _, "input" | "key" | "messages", ..] => true,
        // Any reply route of a pane (a native agent's permission or question reply): by the verb, not by the agent.
        ["panes", _, rest @ ..] if rest.contains(&"reply") => true,
        ["agents", "answer", ..] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_registry::{tools, ToolSafety};

    /// A new MCP tool that answers a card or types into a pane must be added to `ANSWERING_ROUTES`, or this fails.
    #[test]
    fn every_tool_that_could_answer_a_card_is_guarded() {
        let known = ["answer_prompt", "reply_opencode_permission", "reply_opencode_question", "write_terminal_input", "press_key"];
        for tool in tools() {
            let looks_like_answering = tool.path.contains("answer")
                || tool.path.ends_with("/reply")
                || tool.path.ends_with("/key")
                || tool.path.ends_with("/input")
                || tool.name.contains("answer")
                || tool.name.starts_with("reply_");
            if looks_like_answering && tool.safety != ToolSafety::ReadOnly {
                assert!(is_answering_route(tool.method, tool.path), "{} ({} {}) can answer a card but is not guarded", tool.name, tool.method, tool.path);
            }
        }
        for name in known {
            let tool = tools().into_iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name} left the catalog"));
            assert!(is_answering_route(tool.method, tool.path), "{name}");
        }
        assert_eq!(ANSWERING_ROUTES.len(), known.len());
    }

    #[test]
    fn the_loopback_card_routes_are_known_next_to_the_mcp_ones() {
        for (method, path) in [
            ("POST", "/questions/q1/answer"),
            ("POST", "/questions/q1/dismiss"),
            ("POST", "/panes/p1/messages"),
            ("POST", "/agents/answer"),
            ("POST", "/panes/p1/key"),
            // Extra path segments reach the same handlers, so they count too.
            ("POST", "/panes/p1/key/enter"),
            ("POST", "/panes/p1/input/x"),
            ("POST", "/panes/p1/opencode/question/reply"),
            ("POST", "/questions/q1/answer/x"),
            ("POST", "/panes/p1/some-agent/permissions/r1/reply"),
        ] {
            assert!(is_card_route(method, path), "{method} {path}");
        }
        for (method, path) in [("GET", "/questions"), ("POST", "/questions/ask"), ("GET", "/panes/p1/messages"), ("POST", "/agents/run"), ("DELETE", "/panes/p1/messages/m1")] {
            assert!(!is_card_route(method, path), "{method} {path}");
        }
    }
}
