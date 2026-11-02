//! The panel pinned under the conversation while the agent waits on an
//! answer: the question, a chip per option, and the send/skip pair.
//!
//! Pinned rather than placed in the transcript for the reason the host card
//! strip is: the turn cannot go on until it is answered, so it must not
//! scroll out of reach.

use daruda_acp::{QuestionField, QuestionFieldKind};
use daruda_ui::theme::{self, PaneSurfaceTokens};
use daruda_ui::{Disableable as _, Selectable as _, button_chip_on_surface};
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::strings as s;
use crate::view::AgentChatView;
use crate::view::questions::PendingQuestion;

/// Rows of text the panel shows before it scrolls, so a long question cannot
/// take the conversation's whole height.
const MAX_VISIBLE_TEXT_ROWS: f32 = 14.0;

pub(super) fn question_panel(
    view: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> Option<AnyElement> {
    let questions = view.questions();
    let question = questions.first()?;
    let surface = PaneSurfaceTokens::agent_chat(cx).dimmed(view.dim_amount);
    let font = theme::agent_chat_font_size(cx);
    let max_height = font * theme::agent_chat_line_height(cx) * MAX_VISIBLE_TEXT_ROWS;
    let waiting = questions.len().saturating_sub(1);

    let header = div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_SM))
        .child(
            div()
                .text_color(surface.foreground_muted)
                .child(SharedString::from(s::agent_chat::question_title())),
        )
        .children((waiting > 0).then(|| {
            div()
                .text_color(surface.foreground_subtle)
                .child(SharedString::from(s::agent_chat::question_more(waiting)))
        }));

    let card = div()
        .id(("agent-chat-question", question.id as usize))
        .debug_selector(|| "agent-chat-question".into())
        .flex_none()
        .w_full()
        .max_h(px(max_height))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .px(px(theme::PAD_STANDARD))
        .py(px(theme::PAD_XS))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(surface.control_border)
        .bg(surface.tint)
        .text_size(px(font))
        .text_color(surface.foreground)
        .child(header)
        .child(
            div()
                .whitespace_normal()
                .child(SharedString::from(question.request.message.clone())),
        )
        .children(
            question
                .request
                .fields
                .iter()
                .filter(|f| f.companion_of.is_none())
                .map(|field| field_block(question, field, &surface, cx)),
        )
        .child(actions(question, &surface, cx));

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

/// One field: its label, its control, and the "Other" box of a select.
fn field_block(
    question: &PendingQuestion,
    field: &QuestionField,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let id = question.id;
    let mut block = div().flex().flex_col().gap(px(theme::GAP_XS));
    if let Some(title) = &field.title {
        block = block.child(
            div()
                .text_color(surface.foreground_muted)
                .child(SharedString::from(title.clone())),
        );
    }
    if let Some(description) = &field.description {
        block = block.child(
            div()
                .text_color(surface.foreground_subtle)
                .whitespace_normal()
                .child(SharedString::from(description.clone())),
        );
    }
    let mut chips = div().flex().flex_wrap().gap(px(theme::GAP_SM));
    match &field.kind {
        QuestionFieldKind::SingleSelect(options) | QuestionFieldKind::MultiSelect(options) => {
            for (index, option) in options.iter().enumerate() {
                let selector = format!("agent-chat-question-{id}-{}-{index}", field.key);
                let key = field.key.clone();
                let value = option.value.clone();
                let picked = question.is_picked(&field.key, &option.value);
                let mut chip = button_chip_on_surface(
                    SharedString::from(selector.clone()),
                    chip_label(&option.label, picked),
                    surface,
                    cx,
                )
                .selected(picked)
                .debug_selector(move || selector.clone())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.pick_question_option(id, key.clone(), value.clone(), cx)
                }));
                if let Some(description) = &option.description {
                    chip = chip.tooltip(SharedString::from(description.clone()));
                }
                chips = chips.child(chip);
            }
        }
        QuestionFieldKind::Boolean => {
            for (value, label) in [
                (true, s::agent_chat::question_yes()),
                (false, s::agent_chat::question_no()),
            ] {
                let selector = format!("agent-chat-question-{id}-{}-{value}", field.key);
                let key = field.key.clone();
                let picked = question.flag(&field.key) == Some(value);
                chips = chips.child(
                    button_chip_on_surface(
                        SharedString::from(selector.clone()),
                        chip_label(&label, picked),
                        surface,
                        cx,
                    )
                    .selected(picked)
                    .debug_selector(move || selector.clone())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_question_flag(id, key.clone(), value, cx)
                    })),
                );
            }
        }
        QuestionFieldKind::Text => {}
    }
    // A select's own "Other" boxes, then a free-text field's own box.
    let companions = question
        .request
        .fields
        .iter()
        .filter(|f| f.companion_of.as_deref() == Some(field.key.as_str()));
    let boxes: Vec<&QuestionField> = if matches!(field.kind, QuestionFieldKind::Text) {
        vec![field]
    } else {
        companions.collect()
    };
    let mut inputs = Vec::new();
    for text_field in boxes {
        match question.text(&text_field.key) {
            Some(state) => inputs
                .push(daruda_ui::input_on(state, surface.background, cx, ()).into_any_element()),
            None => {
                let selector = format!("agent-chat-question-{id}-{}-open", text_field.key);
                let key = text_field.key.clone();
                chips = chips.child(
                    button_chip_on_surface(
                        SharedString::from(selector.clone()),
                        SharedString::from(s::agent_chat::question_type_answer()),
                        surface,
                        cx,
                    )
                    .debug_selector(move || selector.clone())
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_question_text(id, key.clone(), window, cx)
                    })),
                );
            }
        }
    }
    block.child(chips).children(inputs).into_any_element()
}

/// A picked chip carries a check as well as the active fill: on a surface
/// mirrored from the terminal palette the fill alone is a faint difference.
fn chip_label(label: &str, picked: bool) -> SharedString {
    if picked {
        SharedString::from(s::agent_chat::question_picked(label))
    } else {
        SharedString::from(label.to_owned())
    }
}

/// Send and skip. A form that is a single select sends on the pick, so its
/// send appears only once a typed answer needs one.
fn actions(
    question: &PendingQuestion,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let id = question.id;
    let mut row = div()
        .flex()
        .flex_wrap()
        .justify_end()
        .gap(px(theme::GAP_SM));
    let skip = format!("agent-chat-question-{id}-skip");
    row = row.child(
        button_chip_on_surface(
            SharedString::from(skip.clone()),
            SharedString::from(s::agent_chat::question_skip()),
            surface,
            cx,
        )
        .debug_selector(move || skip.clone())
        .on_click(cx.listener(move |view, _, _, cx| view.skip_question(id, cx))),
    );
    if !question.picks_answer() || question.has_open_text() {
        let submit = format!("agent-chat-question-{id}-submit");
        row = row.child(
            button_chip_on_surface(
                SharedString::from(submit.clone()),
                SharedString::from(s::agent_chat::question_submit()),
                surface,
                cx,
            )
            .disabled(question.answer(cx).is_none())
            .debug_selector(move || submit.clone())
            .on_click(cx.listener(move |view, _, _, cx| view.submit_question(id, cx))),
        );
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::chip_label;

    /// The picked state shows in the label too, not only in the fill.
    #[test]
    fn a_picked_chip_says_so_in_its_label() {
        assert_eq!(chip_label("Redis", false).as_ref(), "Redis");
        let picked = chip_label("Redis", true);
        assert_ne!(picked.as_ref(), "Redis");
        assert!(picked.contains("Redis"));
    }
}
