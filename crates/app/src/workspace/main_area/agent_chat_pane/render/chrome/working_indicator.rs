//! The turn's live progress, projected as the tail row of the conversation
//! rather than drawn in the pane chrome — the signal sits where the next
//! response will appear.

use daruda_acp::ChatItem;
use gpui::{Context, IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings as s;
use crate::ui::StatusPulseClock;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

/// Animated trailing dots (".", "..", "...") for any "in progress" label. Cycles
/// off the shared, CPU-gated `StatusPulseClock` — the pulse pump dirties each
/// agent chat view in the Working activity state (incl. background subagent
/// activity) every tick (`Workspace::notify_in_flight_agent_chats`), so callers
/// advance without a per-frame animation. Shared by the working footer /
/// indicator and the running tool-call badge.
pub(in crate::workspace::main_area::agent_chat_pane::render) fn pulse_dots(
    cx: &gpui::App,
) -> String {
    let tick = cx
        .try_global::<StatusPulseClock>()
        .map(|c| c.tick)
        .unwrap_or(0);
    ".".repeat((tick % 3) as usize + 1)
}

/// Title of the tool call currently in progress, if any — drives the
/// ExecutingTool footer label. The agent runs calls sequentially, so the last
/// `InProgress` call is the live one.
fn running_tool_title(items: &[ChatItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.status.is_live() => Some(tc.title.clone()),
        _ => None,
    })
}

/// Collapse an adapter-supplied tool-call title to a single clean line for the
/// status label: runs of whitespace (incl. newlines and tabs) become one space
/// and the ends are trimmed, so a multi-line command title can never force a
/// line break or leak a raw `\n`. Horizontal length is left to the label's
/// `overflow_hidden` + ellipsis, so no arbitrary character cap is imposed here.
fn single_line_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Human-readable elapsed time for the working indicator.
/// Formats as `"5s"` under a minute, `"1m05s"` at or over a minute.
fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

/// The live activity label this turn: blocked on a permission prompt, running
/// background subagents, running a named tool, or otherwise generating prose.
/// The animated trailing dots are appended by [`working_indicator`]. Subagent
/// activity outranks the running-tool title because during a subagent run the
/// live tool is a (noisy) child call — the count is the signal the user wants.
fn working_status(content: &AgentChatView) -> SharedString {
    if content.has_pending_permission() {
        s::agent_chat_awaiting_permission().into()
    } else if let Some(running) = content.subagent_progress() {
        s::agent_chat_subagent_progress(running).into()
    } else if let Some(title) = running_tool_title(&content.items) {
        s::agent_chat_working_tool(&single_line_title(&title)).into()
    } else {
        s::agent_chat_working().into()
    }
}

/// Inline "agent is working" indicator, projected as the tail row of the last
/// turn for the whole time a turn is in flight (through tool execution and
/// streaming) — see `rows::project`'s gate. It lives *in* the conversation
/// flow, so the progress signal sits where the next response will appear. The
/// label gets animated trailing dots (".", "..", "...") off the shared
/// `StatusPulseClock` — the pulse pump dirties this view while it is in the
/// Working activity state (incl. background subagent activity)
/// (`Workspace::notify_in_flight_agent_chats`), so they advance without a
/// per-frame animation. Cancelling is the bottom-dock Stop button.
pub(in crate::workspace::main_area::agent_chat_pane::render) fn working_indicator(
    content: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let base = working_status(content);
    let dots = pulse_dots(cx);
    let elapsed_label = content.activity_elapsed().map(format_elapsed);
    let mut row = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(content.dim(theme::agent_chat_fg_subtle(cx)))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(format!("{base}{dots}"))),
        );
    if let Some(elapsed) = elapsed_label {
        row = row.child(
            div()
                .flex_none()
                .text_color(content.dim(theme::agent_chat_fg_muted(cx)))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(elapsed)),
        );
    }
    row
}

#[cfg(test)]
mod tests {
    use super::{format_elapsed, running_tool_title, single_line_title};
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    fn tool(id: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            tool_name: None,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    #[test]
    fn single_line_title_collapses_newlines_tabs_and_runs() {
        assert_eq!(
            single_line_title("git commit -m \"line one\nline two\""),
            "git commit -m \"line one line two\""
        );
        assert_eq!(single_line_title("  foo\t\tbar \n baz  "), "foo bar baz");
        assert_eq!(single_line_title("already clean"), "already clean");
    }

    #[test]
    fn running_tool_title_cases() {
        let items = [
            ChatItem::AssistantText {
                text: "a".into(),
                streaming: true,
                message_id: None,
                phase: Default::default(),
            },
            tool("c1", ToolStatusView::Completed),
        ];
        assert_eq!(running_tool_title(&items), None);

        // Settled (Completed) calls are skipped; the latest *live* one wins.
        // `Pending` counts as live (see `ToolStatusView::is_live`), so a trailing
        // Pending call outranks an earlier InProgress one.
        let items = [
            tool("c1", ToolStatusView::Completed),
            tool("c2", ToolStatusView::InProgress),
            tool("c3", ToolStatusView::Pending),
        ];
        assert_eq!(running_tool_title(&items), Some("Tool c3".to_owned()));
    }

    #[test]
    fn format_elapsed_cases() {
        for (secs, expected) in [
            (0, "0s"),
            (5, "5s"),
            (60, "1m00s"),
            (65, "1m05s"),
            (600, "10m00s"),
        ] {
            assert_eq!(
                format_elapsed(std::time::Duration::from_secs(secs)),
                expected
            );
        }
    }
}
