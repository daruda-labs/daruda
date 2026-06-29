//! Pane chrome around the conversation: the top status banner, the activity
//! bar (title + expand/collapse), and the inline "agent is working" indicator
//! with its animated pulse dots and elapsed clock.

use daruda_acp::{ChatItem, ToolStatusView};
use gpui::{Hsla, IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Sizable as _, StatusPulseClock};
use crate::workspace::main_area::agent_chat_pane::view::{AgentChatView, AgentSessionStatus};
use crate::workspace::main_area::pane_tree::PaneId;

/// Pane activity bar: session title on the LEFT, "Expand all" / "Collapse all"
/// ghost buttons on the RIGHT. Always rendered — it holds the title even while
/// the conversation is empty or still connecting. The fold buttons appear only
/// when `has_items` is true (render purity: no logic here, just `.when()`).
/// A bottom hairline separates the bar from the conversation body.
pub(super) fn activity_bar(
    pane_id: PaneId,
    session_title: Option<&str>,
    has_items: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = session_title
        .map(|s| SharedString::from(s.to_string()))
        .unwrap_or_else(|| SharedString::from(s::agent_chat_activity_bar_title()));

    let expand = crate::ui::button(
        ("agent-chat-expand-all", pane_id as usize),
        SharedString::from(s::agent_chat_expand_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |this, _ev, _window, cx| this.set_all_folds(true, cx)));
    let collapse = crate::ui::button(
        ("agent-chat-collapse-all", pane_id as usize),
        SharedString::from(s::agent_chat_collapse_all()),
    )
    .ghost()
    .xsmall()
    .on_click(cx.listener(move |this, _ev, _window, cx| this.set_all_folds(false, cx)));

    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .justify_between()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .text_color(t.text_primary)
                .child(title),
        )
        .when(has_items, |row| {
            row.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::AGENT_CHAT_MSG_GAP))
                    .text_color(t.text_muted)
                    .child(expand)
                    .child(collapse),
            )
        })
}

/// The thin top banner — shown while connecting or on error; hidden once
/// the session is live (the conversation itself signals readiness).
pub(super) fn status_banner(
    status: &AgentSessionStatus,
    t: &theme::DarudaTheme,
) -> Option<impl IntoElement + use<>> {
    let (text, bg, fg): (SharedString, Hsla, Hsla) = match status {
        AgentSessionStatus::Idle => (
            s::agent_chat_idle().into(),
            t.banner_info_bg,
            t.banner_info_text,
        ),
        AgentSessionStatus::Connecting => (
            s::agent_chat_connecting().into(),
            t.banner_info_bg,
            t.banner_info_text,
        ),
        AgentSessionStatus::Connected => return None,
        AgentSessionStatus::Error(message) => (
            format!("{} {}", s::agent_chat_error_prefix(), message).into(),
            t.banner_error_bg,
            t.banner_error_text,
        ),
    };
    Some(
        div()
            .flex_none()
            .w_full()
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
            .bg(bg)
            .text_color(fg)
            .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
            .child(text),
    )
}

/// Title of the tool call currently in progress, if any — drives the
/// ExecutingTool footer label. The agent runs calls sequentially, so the last
/// `InProgress` call is the live one.
fn running_tool_title(items: &[ChatItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if matches!(tc.status, ToolStatusView::InProgress) => {
            Some(tc.title.clone())
        }
        _ => None,
    })
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

/// Animated trailing dots (".", "..", "...") for any "in progress" label. Cycles
/// off the shared, CPU-gated `StatusPulseClock` — the pulse pump dirties the
/// in-flight agent chat view each tick (`Workspace::notify_in_flight_agent_chats`),
/// so callers advance without a per-frame animation. Shared by the working
/// footer / indicator and the running tool-call badge.
pub(super) fn pulse_dots(cx: &gpui::App) -> String {
    let tick = cx
        .try_global::<StatusPulseClock>()
        .map(|c| c.tick)
        .unwrap_or(0);
    ".".repeat((tick % 3) as usize + 1)
}

/// The live activity label this turn: blocked on a permission prompt, running a
/// named tool, or otherwise generating prose. The animated trailing dots are
/// appended by [`working_indicator`].
fn working_status(content: &AgentChatView) -> SharedString {
    if content.pending_permission.is_some() {
        s::agent_chat_awaiting_permission().into()
    } else if let Some(title) = running_tool_title(&content.items) {
        s::agent_chat_working_tool(&title).into()
    } else {
        s::agent_chat_working().into()
    }
}

/// Inline "agent is working" indicator, projected as the tail row of the last
/// turn while a turn is in flight but nothing is streaming yet (the gap after a
/// tool group settles, before the next assistant text). It lives *in* the
/// conversation flow, so the progress signal sits where the next response will
/// appear. The label gets animated trailing dots (".", "..", "...") off the
/// shared `StatusPulseClock` — the pulse pump dirties this view while the turn
/// is in flight (`Workspace::notify_in_flight_agent_chats`), so they advance
/// without a per-frame animation. Cancelling is the bottom-dock Stop button.
pub(super) fn working_indicator(
    content: &AgentChatView,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let base = working_status(content);
    let dots = pulse_dots(cx);
    let elapsed_label = content
        .turn_started_at
        .map(|start| format_elapsed(start.elapsed()));
    let row = div()
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
                .text_color(t.text_subtle)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(format!("{base}{dots}"))),
        );
    if let Some(elapsed) = elapsed_label {
        row.child(
            div()
                .flex_none()
                .text_color(t.text_muted)
                .text_size(px(theme::AGENT_CHAT_LABEL_FONT_SIZE))
                .child(SharedString::from(elapsed)),
        )
    } else {
        row
    }
}

#[cfg(test)]
mod tests {
    use super::{format_elapsed, running_tool_title};
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    fn tool(id: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
        })
    }

    #[test]
    fn running_tool_title_is_none_without_an_in_progress_call() {
        let items = [
            ChatItem::AssistantText {
                text: "a".into(),
                streaming: true,
                message_id: None,
            },
            tool("c1", ToolStatusView::Completed),
        ];
        assert_eq!(running_tool_title(&items), None);
    }

    #[test]
    fn running_tool_title_picks_the_last_in_progress_call() {
        // Completed earlier calls are skipped; the latest in-progress one wins.
        let items = [
            tool("c1", ToolStatusView::Completed),
            tool("c2", ToolStatusView::InProgress),
            tool("c3", ToolStatusView::Pending),
        ];
        assert_eq!(running_tool_title(&items), Some("Tool c2".to_owned()));
    }

    #[test]
    fn format_elapsed_zero_seconds() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(0)), "0s");
    }

    #[test]
    fn format_elapsed_five_seconds() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(5)), "5s");
    }

    #[test]
    fn format_elapsed_sixty_five_seconds() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(65)), "1m05s");
    }

    #[test]
    fn format_elapsed_at_one_minute_boundary() {
        assert_eq!(format_elapsed(std::time::Duration::from_secs(60)), "1m00s");
    }

    #[test]
    fn format_elapsed_six_hundred_seconds() {
        assert_eq!(
            format_elapsed(std::time::Duration::from_secs(600)),
            "10m00s"
        );
    }
}
