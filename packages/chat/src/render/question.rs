//! The panel pinned under the conversation for a question no transcript row
//! holds: one asked without a tool call, or whose call has no row yet. A
//! question with a row is drawn on it instead (`tool::question`). Pinned for
//! the host card strip's reason: the turn cannot go on until it is answered,
//! so it must not scroll out of reach.

use daruda_ui::theme::{self, PaneSurfaceTokens};
use gpui::{AnyElement, Context, div, prelude::*, px};

use super::tool::question_form;
use crate::view::AgentChatView;

/// Rows of text the panel shows before it scrolls, so a long question cannot
/// take the conversation's whole height.
const MAX_VISIBLE_TEXT_ROWS: f32 = 14.0;

pub(super) fn question_panel(
    view: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> Option<AnyElement> {
    let question = view
        .questions()
        .open()
        .find(|q| view.question_row(q.tool_call_id()).is_none())?;
    let surface = PaneSurfaceTokens::agent_chat(cx).dimmed(view.dim_amount);
    let max_height =
        theme::agent_chat_font_size(cx) * theme::agent_chat_line_height(cx) * MAX_VISIBLE_TEXT_ROWS;
    let may_write = view.questions().first().map(|q| q.id) == Some(question.id);
    let card = question_form(question, may_write, view.agent_name(), &surface, cx)
        .id(("agent-chat-question", question.id as usize))
        .debug_selector(|| "agent-chat-question".into())
        .flex_none()
        .max_h(px(max_height))
        .overflow_y_scroll();
    Some(
        div()
            .flex_none()
            .w_full()
            .px(px(theme::PAD_STANDARD))
            .py(px(theme::PAD_XS))
            .border_t_1()
            .border_color(surface.border_tint)
            .child(card)
            .into_any_element(),
    )
}
