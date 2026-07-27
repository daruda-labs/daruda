//! Slash-command completion provider for the bottom-dock input.
//!
//! When a `/`-prefixed token is typed at the start of the current line
//! while an AgentChat pane is focused, this provider feeds the native
//! completion menu with the ACP-advertised commands stored on
//! [`AgentChatView::available_commands`](super::super::agent_chat_pane). The
//! provider is installed on `Workspace.terminal_input` (the shared
//! bottom-dock `InputState`); the menu's accept hook routes back through
//! [`Workspace::complete_slash_command`](crate::workspace::Workspace) for the
//! adaptive send.
//!
//! The trigger / parsing logic is split into pure, GPUI-free helpers (tested
//! below); the trait impl only walks the rope and reads the focused pane's
//! command list off the workspace.

use anyhow::Result;
use daruda_acp::{SlashCommand, SlashCommandInput};
use gpui::{Context, Task, WeakEntity, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Range, TextEdit,
};

use super::super::agent_chat_pane::slash_dispatch::CLEAR_COMMAND_NAME;
use crate::surface::strings as s;
use crate::ui::{CompletionProvider, InputState, Rope, RopeExt as _};
use crate::workspace::Workspace;

/// Removes any agent-advertised `/clear` entry and prepends daruda's own
/// built-in one. Dispatch (`classify_slash`) always intercepts `/clear`
/// locally and ignores arguments — regardless of what the agent advertises —
/// so daruda's entry (with its own description and `NoInput`) must be the
/// only one shown, or the menu misrepresents what actually happens on
/// submit.
fn with_builtin_clear(mut commands: Vec<SlashCommand>) -> Vec<SlashCommand> {
    commands.retain(|c| c.name != CLEAR_COMMAND_NAME);
    commands.insert(
        0,
        SlashCommand {
            name: CLEAR_COMMAND_NAME.to_string(),
            description: s::agent_chat_clear_command_desc(),
            input: SlashCommandInput::NoInput,
        },
    );
    commands
}

/// Completion provider that surfaces ACP slash commands in the bottom-dock
/// input. Holds a weak reference to the workspace so it can read the focused
/// AgentChat pane's advertised commands at completion time.
pub(in crate::workspace) struct SlashCommandProvider {
    pub(in crate::workspace) workspace: WeakEntity<Workspace>,
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

impl CompletionProvider for SlashCommandProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let empty = || Task::ready(Ok(CompletionResponse::Array(vec![])));

        let before = text.slice(0..offset).to_string();
        let line = current_line_before_cursor(&before);
        if !is_command_position(line) {
            return empty();
        }
        // Chars after the `/` on the current line.
        let prefix = &line.trim_start()[1..];

        // The workspace is a different entity than this `InputState`, so reading
        // it (and, transitively, the focused AgentChat view) during the
        // InputState update is safe.
        let Some(ws) = self.workspace.upgrade() else {
            return empty();
        };
        let ws = ws.read(cx);
        let focused = ws.active_runtime().focused_pane_id;
        let commands: Vec<SlashCommand> = with_builtin_clear(
            ws.agent_chat_view(focused)
                .map(|v| v.read(cx).session_config.available_commands.clone())
                .unwrap_or_default(),
        );

        // Replace range = the `/<prefix>` token on the current line: from the
        // `/` (offset - prefix bytes - 1) through the end of the token.
        let start = offset - prefix.len() - 1;
        let start_pos = text.offset_to_position(start);
        // Replace through the end of the token under the cursor, not just up to it,
        // so accepting mid-token (e.g. "/com|mand") doesn't leave a trailing fragment.
        let after = text.slice(offset..).to_string();
        let token_end = offset + token_end_len(&after);
        let end_pos = text.offset_to_position(token_end);

        let items: Vec<CompletionItem> = matching_commands(&commands, prefix)
            .into_iter()
            .map(|c| CompletionItem {
                label: c.name.clone(),
                filter_text: Some(c.name.clone()),
                detail: Some(c.description.clone()),
                // `documentation` is the LSP "rich docs" aside panel. ACP commands
                // carry only a short description (already shown inline via `detail`)
                // and an argument `hint`. The `hint` is an INPUT placeholder
                // ("display when the input hasn't been provided yet"), not docs —
                // surfacing it here rendered a large side panel that also vanished
                // on accept, exactly when the argument is needed. Leave docs empty.
                kind: Some(CompletionItemKind::FUNCTION),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: Range {
                        start: start_pos,
                        end: end_pos,
                    },
                    new_text: match &c.input {
                        SlashCommandInput::NoInput => format!("/{}", c.name),
                        // Trailing space so the user lands on the argument.
                        SlashCommandInput::FreeText { .. } => format!("/{} ", c.name),
                    },
                })),
                ..Default::default()
            })
            .collect();

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        _new_text: &str,
        cx: &mut Context<InputState>,
    ) -> bool {
        // Trigger broadly while a chat pane is focused — `completions` returns
        // an empty list (closing the menu) when not in command position. The
        // InputState's own text is mid-update here, so the text check belongs in
        // `completions` (which receives the rope), not here.
        self.workspace.upgrade().is_some_and(|ws| {
            let ws = ws.read(cx);
            ws.is_agent_chat_pane(ws.active_runtime().focused_pane_id)
        })
    }
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
        assert_eq!(clear.description, s::agent_chat_clear_command_desc());
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
}
