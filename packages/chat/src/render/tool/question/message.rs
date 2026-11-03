//! Disclosure for long question messages; fields and actions stay outside it.

use daruda_ui::{
    button_chip_on_surface,
    theme::{self, PaneSurfaceTokens},
};
use gpui::{Context, Div, SharedString, div, prelude::*, px};

use crate::{
    strings as s,
    view::{AgentChatView, questions::OpenQuestion},
};

const PREVIEW_LINES: usize = 3;
const LONG_MESSAGE_CHARS: usize = 240;

fn is_long(message: &str) -> bool {
    message.lines().count() > PREVIEW_LINES || message.chars().count() > LONG_MESSAGE_CHARS
}

pub(super) fn render(
    open: &OpenQuestion,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> Div {
    let long = is_long(&open.request.message);
    let id = open.id;
    let selector = format!("agent-chat-question-{id}-message-toggle");
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme::GAP_XS))
        .child(
            div()
                .whitespace_normal()
                .when(long && !open.message_expanded, |body| {
                    // Ellipsis enables wrapped truncation across logical lines.
                    body.line_clamp(PREVIEW_LINES).text_ellipsis()
                })
                .child(SharedString::from(open.request.message.clone())),
        )
        .when(long, |body| {
            body.child(
                div().flex().child(
                    button_chip_on_surface(
                        SharedString::from(selector.clone()),
                        SharedString::from(if open.message_expanded {
                            s::agent_chat::question_message_less()
                        } else {
                            s::agent_chat::question_message_more()
                        }),
                        surface,
                        cx,
                    )
                    .debug_selector(move || selector.clone())
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.toggle_question_message(id, cx)),
                    ),
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_need_disclosure_only_when_the_preview_is_long() {
        assert!(!is_long("A short question"));
        assert!(is_long("a\nb\nc\nd"));
        assert!(is_long(&"한".repeat(LONG_MESSAGE_CHARS + 1)));
        assert!(!is_long(&"한".repeat(LONG_MESSAGE_CHARS)));
    }
}
