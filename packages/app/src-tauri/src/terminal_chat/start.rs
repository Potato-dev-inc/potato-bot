//! Starting a terminal chat: the ordinary new-pane path, then the pane is tagged `lane = "terminal_chat"` so the UI
//! opens it on its Chat tab. Desktop app only: no bridge route and no MCP tool, so a phone or an agent cannot start
//! one. No conversation record is made (the pane, the session file and the resume plan are enough).



#[cfg(test)]
mod tests {
    use crate::terminal_chat::available_for;

    #[test]
    fn a_shell_cannot_start() {
        // `available_for` is what the command checks first; a shell is not an agent.
        let shell = crate::tui::registry::snapshot().iter().find(|p| p.features().kind == crate::tui::Kind::Shell).map(|p| p.id());
        if let Some(id) = shell {
            assert!(available_for(id).is_err());
        }
    }
}
