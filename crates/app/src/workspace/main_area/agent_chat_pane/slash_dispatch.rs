//! GPUI-free classifier for agent-chat submit text.
//!
//! On submit, the chat pane must decide whether the text is forwarded to the
//! agent as-is, or intercepted and handled locally (e.g. `/clear`, which
//! drives a full session reset instead of being sent over ACP). This module
//! does no validation beyond that single decision — every input other than a
//! recognized daruda-local command forwards unchanged.

/// Name of the daruda-local `/clear` command. Canonical source shared with
/// the completion provider (`bottom_dock::slash_command`), which surfaces
/// this exact name in the slash-command menu so the menu entry always
/// matches what `classify_slash` actually intercepts.
pub(in crate::workspace) const CLEAR_COMMAND_NAME: &str = "clear";

/// A daruda-local slash command — handled entirely on the client side and
/// never sent to the agent.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::workspace) enum LocalSlashCommand {
    /// Reset the current agent-chat session (`/clear`).
    Clear,
}

/// Where a submitted chat input should go.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::workspace) enum SlashDispatch {
    /// Forward the text to the agent unchanged.
    Forward,
    /// Handle locally; never sent to the agent.
    Local(LocalSlashCommand),
}

/// Classifies a submitted agent-chat input string.
///
/// Trims surrounding whitespace, then checks whether the first
/// whitespace-delimited token is a recognized daruda-local command (`/clear`).
/// Everything else — plain text, unrecognized slash commands, or a bare
/// `/` — forwards to the agent.
pub(in crate::workspace) fn classify_slash(text: &str) -> SlashDispatch {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return SlashDispatch::Forward;
    };
    let first_token = rest.split_whitespace().next().unwrap_or("");
    match first_token {
        CLEAR_COMMAND_NAME => SlashDispatch::Local(LocalSlashCommand::Clear),
        _ => SlashDispatch::Forward,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_forwards() {
        assert_eq!(classify_slash("hello"), SlashDispatch::Forward);
    }

    #[test]
    fn clear_is_local() {
        assert_eq!(
            classify_slash("/clear"),
            SlashDispatch::Local(LocalSlashCommand::Clear)
        );
    }

    #[test]
    fn clear_with_trailing_args_is_local() {
        assert_eq!(
            classify_slash("/clear now"),
            SlashDispatch::Local(LocalSlashCommand::Clear)
        );
    }

    #[test]
    fn clear_with_leading_whitespace_is_local() {
        assert_eq!(
            classify_slash("  /clear"),
            SlashDispatch::Local(LocalSlashCommand::Clear)
        );
    }

    #[test]
    fn unrecognized_slash_command_forwards() {
        assert_eq!(classify_slash("/foo"), SlashDispatch::Forward);
    }

    #[test]
    fn bare_slash_forwards() {
        assert_eq!(classify_slash("/"), SlashDispatch::Forward);
    }
}
