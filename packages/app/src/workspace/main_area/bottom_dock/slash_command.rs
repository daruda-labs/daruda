//! Slash-command completion for the bottom-dock composer.
//!
//! When a `/`-prefixed token is typed at the start of the current line while
//! an AgentChat pane is focused, the composer lists the ACP-advertised
//! commands stored on `AgentChatView::available_commands` (`daruda_chat`).
//! The list is daruda's own: [`slash_match`] computes it from the text and
//! cursor, the composer ops keep it on the input dock, and the dock renders
//! it above the field. Accepting routes back through
//! [`Workspace::complete_slash_command`](crate::workspace::Workspace) for the
//! adaptive send.
//!
//! Everything here is pure and GPUI-free, so it is tested below.

use std::ops::Range;

use daruda_acp::{SlashCommand, SlashCommandInput};

use crate::surface::strings as s;
use daruda_chat::slash_dispatch::CLEAR_COMMAND_NAME;

/// Removes any agent-advertised `/clear` entry and prepends daruda's own
/// built-in one. Dispatch (`classify_slash`) always intercepts `/clear`
/// locally and ignores arguments — regardless of what the agent advertises —
/// so daruda's entry (with its own description and `NoInput`) must be the
/// only one shown, or the menu misrepresents what actually happens on
/// submit.
pub(in crate::workspace) fn with_builtin_clear(
    mut commands: Vec<SlashCommand>,
) -> Vec<SlashCommand> {
    commands.retain(|c| c.name != CLEAR_COMMAND_NAME);
    commands.insert(
        0,
        SlashCommand {
            name: CLEAR_COMMAND_NAME.to_string(),
            description: s::agent_chat::clear_command_desc(),
            input: SlashCommandInput::NoInput,
        },
    );
    commands
}

/// The substring after the last `\n` — the line the cursor is on. The
/// bottom-dock input is multi-line, so the command match must be scoped to the
/// current line rather than the whole buffer.
fn current_line_before_cursor(text_before_cursor: &str) -> &str {
    match text_before_cursor.rfind('\n') {
        Some(nl) => &text_before_cursor[nl + 1..],
        None => text_before_cursor,
    }
}

/// True iff the line matches `^\s*/[^\s]*$` — optional leading whitespace, a
/// `/`, then zero or more non-whitespace chars, and nothing else. No regex: a
/// command is being typed when, after trimming leading whitespace, the line
/// starts with `/` and the remainder contains no whitespace.
fn is_command_position(line_before_cursor: &str) -> bool {
    let trimmed = line_before_cursor.trim_start();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return false;
    };
    !rest.chars().any(char::is_whitespace)
}

/// Byte length from the cursor to the end of the current token: stops at the
/// first whitespace or newline (or end of text). 0 when the cursor is already
/// at a token boundary (the common end-of-input case).
fn token_end_len(after_cursor: &str) -> usize {
    after_cursor
        .find(|c: char| c.is_whitespace())
        .unwrap_or(after_cursor.len())
}

/// Commands whose name starts with `prefix` (the chars after the `/`).
fn matching_commands<'a>(commands: &'a [SlashCommand], prefix: &str) -> Vec<&'a SlashCommand> {
    commands
        .iter()
        .filter(|c| c.name.starts_with(prefix))
        .collect()
}

/// One command the list offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct SlashItem {
    pub(in crate::workspace) name: String,
    pub(in crate::workspace) description: String,
    /// What accepting writes over the token: `/name`, plus a trailing space
    /// when the command takes an argument, so the user lands on it.
    pub(in crate::workspace) insert: String,
}

/// The commands matching the `/`-token under the cursor, and the byte range
/// of that token (from the `/` through the end of the token, so accepting
/// mid-token leaves no trailing fragment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct SlashMatch {
    pub(in crate::workspace) token: Range<usize>,
    pub(in crate::workspace) items: Vec<SlashItem>,
}

/// The list for `text` with the cursor at byte `cursor`, or `None` when the
/// cursor is not in command position or nothing matches.
pub(in crate::workspace) fn slash_match(
    text: &str,
    cursor: usize,
    commands: &[SlashCommand],
) -> Option<SlashMatch> {
    let before = text.get(..cursor)?;
    let line = current_line_before_cursor(before);
    if !is_command_position(line) {
        return None;
    }
    // Chars after the `/` on the current line.
    let prefix = &line.trim_start()[1..];
    let start = cursor - prefix.len() - 1;
    let end = cursor + token_end_len(&text[cursor..]);
    let items: Vec<SlashItem> = matching_commands(commands, prefix)
        .into_iter()
        .map(|c| SlashItem {
            name: c.name.clone(),
            description: c.description.clone(),
            insert: match &c.input {
                SlashCommandInput::NoInput => format!("/{}", c.name),
                SlashCommandInput::FreeText { .. } => format!("/{} ", c.name),
            },
        })
        .collect();
    (!items.is_empty()).then_some(SlashMatch {
        token: start..end,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str, input: SlashCommandInput) -> SlashCommand {
        SlashCommand {
            name: name.to_string(),
            description: String::new(),
            input,
        }
    }

    #[test]
    fn current_line_before_cursor_takes_last_line() {
        assert_eq!(current_line_before_cursor(""), "");
        assert_eq!(current_line_before_cursor("/com"), "/com");
        assert_eq!(current_line_before_cursor("hello\n/com"), "/com");
        assert_eq!(current_line_before_cursor("a\nb\n  /x"), "  /x");
        // A trailing newline yields an empty current line.
        assert_eq!(current_line_before_cursor("/com\n"), "");
    }

    #[test]
    fn is_command_position_matches_lone_slash_token() {
        assert!(!is_command_position(""));
        assert!(is_command_position("/"));
        assert!(is_command_position("/com"));
        assert!(is_command_position("  /x"));
        assert!(!is_command_position("/com arg"));
        assert!(!is_command_position("a /x"));
        assert!(!is_command_position("x"));
    }

    #[test]
    fn token_end_len_stops_at_whitespace() {
        // Empty input: cursor already at boundary.
        assert_eq!(token_end_len(""), 0);
        // Entire token after cursor.
        assert_eq!(token_end_len("mand"), 4);
        // Stops before the space.
        assert_eq!(token_end_len("mand foo"), 4);
        // Newline counts as whitespace: immediate boundary.
        assert_eq!(token_end_len("\nx"), 0);
        // Leading spaces: immediate boundary.
        assert_eq!(token_end_len("  x"), 0);
    }

    #[test]
    fn with_builtin_clear_prepends_when_absent() {
        let cmds = vec![cmd("commit", SlashCommandInput::NoInput)];
        let merged = with_builtin_clear(cmds);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "clear");
        assert!(matches!(merged[0].input, SlashCommandInput::NoInput));
        assert_eq!(merged[1].name, "commit");
    }

    #[test]
    fn with_builtin_clear_replaces_agent_advertised_entry() {
        let cmds = vec![
            cmd("commit", SlashCommandInput::NoInput),
            cmd(
                "clear",
                SlashCommandInput::FreeText {
                    hint: "what".to_string(),
                },
            ),
        ];
        let merged = with_builtin_clear(cmds);
        // Daruda's own `/clear` replaces the agent-advertised entry — dispatch
        // always intercepts `/clear` locally with no argument, regardless of
        // what the agent advertises, so exactly one `clear` remains and it's
        // daruda's `NoInput` entry with daruda's description.
        assert_eq!(merged.len(), 2);
        let clear_count = merged.iter().filter(|c| c.name == "clear").count();
        assert_eq!(clear_count, 1);
        let clear = merged.iter().find(|c| c.name == "clear").unwrap();
        assert!(matches!(clear.input, SlashCommandInput::NoInput));
        assert_eq!(clear.description, s::agent_chat::clear_command_desc());
    }

    #[test]
    fn matching_commands_filters_by_prefix() {
        let cmds = vec![
            cmd("commit", SlashCommandInput::NoInput),
            cmd("compact", SlashCommandInput::NoInput),
            cmd(
                "clear",
                SlashCommandInput::FreeText {
                    hint: "what".to_string(),
                },
            ),
        ];
        let names: Vec<&str> = matching_commands(&cmds, "com")
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["commit", "compact"]);

        // Empty prefix matches everything.
        assert_eq!(matching_commands(&cmds, "").len(), 3);
        // No match.
        assert!(matching_commands(&cmds, "zzz").is_empty());
    }

    #[test]
    fn slash_match_covers_the_whole_token_under_the_cursor() {
        let cmds = vec![
            cmd("commit", SlashCommandInput::NoInput),
            cmd(
                "review",
                SlashCommandInput::FreeText {
                    hint: "what".to_string(),
                },
            ),
        ];
        let text = "hello\n/com|mand rest".replace('|', "");
        let cursor = "hello\n/com".len();
        let matched = slash_match(&text, cursor, &cmds).expect("in command position");
        assert_eq!(&text[matched.token.clone()], "/command");
        assert_eq!(matched.items.len(), 1);
        assert_eq!(matched.items[0].insert, "/commit");

        let matched = slash_match("/re", 3, &cmds).expect("matches review");
        assert_eq!(matched.items[0].insert, "/review ");
    }

    #[test]
    fn slash_match_is_none_outside_command_position_or_without_matches() {
        let cmds = vec![cmd("commit", SlashCommandInput::NoInput)];
        assert!(slash_match("say /com", 8, &cmds).is_none());
        assert!(slash_match("/zzz", 4, &cmds).is_none());
        assert!(slash_match("/commit now", 11, &cmds).is_none());
    }
}
