//! The open form: one row per option, a row that hands free text to the
//! composer, and skip and send. The card draws no text box.

use daruda_acp::{QuestionField, QuestionFieldKind, ToolCallItem};
use daruda_ui::theme::{self, PaneSurfaceTokens};
use daruda_ui::{ButtonCustomVariant, ButtonVariants as _, Disableable as _, icons};
use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, SharedString, Stateful, div, prelude::*, px,
};

use super::{GLYPH_HIGHLIGHT, GLYPH_WRITE, choice_glyph, field_heading, row_frame};
use crate::strings as s;
use crate::view::AgentChatView;
use crate::view::questions::{OpenQuestion, QuestionCountdown, QuestionHighlight, primary_fields};

/// Which field a write row answers in the composer.
struct WriteTarget {
    question: u64,
    field: usize,
}

/// The open form on the row of the tool call that asked it.
pub(in crate::render) fn question_card(
    tc: &ToolCallItem,
    open: &OpenQuestion,
    may_write: bool,
    agent: &str,
    dim: f32,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let tool = tc.id.clone();
    question_form(open, may_write, agent, dim, surface, cx)
        .id(SharedString::from(format!(
            "agent-chat-question-card-{tool}"
        )))
        .debug_selector(move || format!("agent-chat-question-card-{tool}"))
        .into_any_element()
}

/// The card itself, shared with the panel that shows a question no row
/// holds: header, the agent's message, each question, then skip and send.
/// `may_write` is whether the question may take the composer: only the
/// oldest open one can, so a newer one's write rows are hints, not buttons.
pub(in crate::render) fn question_form(
    open: &OpenQuestion,
    may_write: bool,
    agent: &str,
    dim: f32,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .p(px(theme::PAD_XL))
        .rounded(px(theme::RADIUS_MD))
        .border_1()
        .border_color(surface.border_tint)
        .bg(surface.tint)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .text_color(surface.foreground)
        .child(card_header(open, agent, surface))
        .child(super::message::render(open, surface, cx))
        .children(
            primary_fields(&open.request)
                .enumerate()
                .map(|(index, field)| field_block(open, may_write, index, field, surface, cx)),
        )
        .child(actions(open, dim, surface, cx))
}

/// `? Claude asks` on the left; on the right that it waits, which question
/// has the keyboard when there are several, and the time left when the
/// answer timeout runs. Its last half minute turns brighter and heavier.
fn card_header(open: &OpenQuestion, agent: &str, surface: &PaneSurfaceTokens) -> Div {
    let total = primary_fields(&open.request).count();
    let mut status = s::agent_chat::question_waiting();
    if total > 1 {
        status.push_str(&s::agent_chat::question_separator());
        status.push_str(&s::agent_chat::question_position(
            open.focus.field + 1,
            total,
        ));
    }
    let time_left = open
        .countdown()
        .map(|deadline| QuestionCountdown::until(deadline, std::time::Instant::now()))
        .map(|left| {
            let final_stretch = left.is_final_stretch();
            div()
                .flex_none()
                .when(final_stretch, |d| {
                    d.text_color(surface.foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!final_stretch, |d| d.text_color(surface.foreground_subtle))
                .child(SharedString::from(format!(
                    "{}{}",
                    s::agent_chat::question_separator(),
                    left.label()
                )))
        });
    div()
        .flex()
        .items_center()
        .text_size(px(theme::FONT_SIZE_SM))
        .gap(px(theme::GAP_SM))
        .child(
            div()
                .flex_none()
                .text_color(surface.foreground_muted)
                .child(icons::icon(icons::SESSION)),
        )
        .child(
            div()
                .flex_none()
                .child(SharedString::from(s::agent_chat::question_asked_by(agent))),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex_none()
                .flex()
                .child(
                    div()
                        .text_color(surface.foreground_subtle)
                        .child(SharedString::from(status)),
                )
                .children(time_left),
        )
}

/// Primary field `field_index`: its title, its options as rows, and the row
/// that answers it in the composer. The field with the keyboard carries the
/// heavier rail on its left.
fn field_block(
    open: &OpenQuestion,
    may_write: bool,
    field_index: usize,
    field: &QuestionField,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let id = open.id;
    let current = open.focus.field == field_index;
    let highlighted =
        |row: usize| current && open.focus.highlight == Some(QuestionHighlight::Row(row));
    let mut block = field_heading(field, surface)
        .pl(px(theme::PAD_SM))
        .border_l_2()
        .border_color(if current {
            surface.control_border
        } else {
            surface.border_tint
        });
    let rows = match &field.kind {
        QuestionFieldKind::SingleSelect(options) | QuestionFieldKind::MultiSelect(options) => {
            let multi = matches!(field.kind, QuestionFieldKind::MultiSelect(_));
            for (index, option) in options.iter().enumerate() {
                let picked = open.draft.is_picked(&field.key, &option.value);
                let row = option_row(
                    format!("agent-chat-question-{id}-{}-{index}", field.key),
                    OptionRow {
                        number: index + 1,
                        glyph: choice_glyph(multi, picked),
                        label: &option.label,
                        description: option.description.as_deref(),
                        picked,
                        highlighted: highlighted(index),
                    },
                    surface,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.pick_question_option(id, field_index, index, cx)
                }));
                block = block.child(row);
            }
            options.len()
        }
        QuestionFieldKind::Boolean => {
            let flag = open.draft.flag(&field.key);
            for (index, (value, label)) in [
                (true, s::agent_chat::question_yes()),
                (false, s::agent_chat::question_no()),
            ]
            .into_iter()
            .enumerate()
            {
                let picked = flag == Some(value);
                let row = option_row(
                    format!("agent-chat-question-{id}-{}-{value}", field.key),
                    OptionRow {
                        number: index + 1,
                        glyph: choice_glyph(false, picked),
                        label: &label,
                        description: None,
                        picked,
                        highlighted: highlighted(index),
                    },
                    surface,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.set_question_flag(id, field_index, value, cx)
                }));
                block = block.child(row);
            }
            2
        }
        QuestionFieldKind::Text => 0,
    };
    if let Some(key) = open.text_key(field) {
        let typed = open.draft.text(key).map(|text| {
            // Text beside a single pick is a note on it; anywhere else it is
            // the answer, or joins the picks.
            if open.is_note(field) {
                s::agent_chat::question_note(text)
            } else {
                s::agent_chat::question_typed(text)
            }
        });
        let target = may_write.then_some(WriteTarget {
            question: id,
            field: field_index,
        });
        block = block.child(write_row(
            target,
            rows + 1,
            typed,
            highlighted(rows),
            surface,
            cx,
        ));
    }
    block.into_any_element()
}

struct OptionRow<'a> {
    number: usize,
    glyph: &'static str,
    label: &'a str,
    description: Option<&'a str>,
    picked: bool,
    highlighted: bool,
}

/// One clickable option. Picked and highlighted rows take the selected fill;
/// the glyphs, not the fill, tell the two apart.
fn option_row(selector: String, row: OptionRow<'_>, surface: &PaneSurfaceTokens) -> Stateful<Div> {
    let filled = row.picked || row.highlighted;
    let hover = surface.hover_tint;
    let debug = selector.clone();
    row_frame(None, row.highlighted, row.glyph, surface)
        .id(SharedString::from(selector))
        .debug_selector(move || debug.clone())
        .cursor_pointer()
        .py(px(theme::PAD_SM))
        .border_l_2()
        .border_color(if row.picked {
            surface.control_border
        } else {
            surface.border_tint.opacity(0.0)
        })
        .when(filled, |el| el.bg(surface.active_tint))
        .when(!filled, |el| el.hover(move |style| style.bg(hover)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_XS))
                .child(
                    div()
                        .whitespace_normal()
                        .when(row.picked, |el| el.font_weight(FontWeight::SEMIBOLD))
                        .child(SharedString::from(row.label.to_owned())),
                )
                .children(row.description.map(|description| {
                    div()
                        .whitespace_normal()
                        .text_color(surface.foreground_subtle)
                        .child(SharedString::from(description.to_owned()))
                })),
        )
        .child(
            div()
                .flex_none()
                .px(px(theme::PAD_XS))
                .border_1()
                .border_color(surface.border_tint)
                .rounded(px(theme::RADIUS_XS))
                .text_size(px(theme::FONT_SIZE_XS))
                .text_color(surface.foreground_subtle)
                .child(SharedString::from(row.number.to_string())),
        )
}

/// The row that answers in the composer: with a `target`, a click binds the
/// composer to that field and focuses it; without one it is a static hint.
/// What was committed shows in its place.
fn write_row(
    target: Option<WriteTarget>,
    number: usize,
    typed: Option<String>,
    highlighted: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let frame = row_frame(None, highlighted, GLYPH_WRITE, surface)
        .border_t_1()
        .border_color(surface.border_tint)
        .pt(px(theme::PAD_STANDARD))
        .when(highlighted, |el| el.bg(surface.active_tint));
    let row = write_row_content(frame, typed, surface).child(
        div()
            .flex_none()
            .px(px(theme::PAD_XS))
            .border_1()
            .border_color(surface.border_tint)
            .rounded(px(theme::RADIUS_XS))
            .text_size(px(theme::FONT_SIZE_XS))
            .text_color(surface.foreground_subtle)
            .child(SharedString::from(number.to_string())),
    );
    let Some(WriteTarget { question, field }) = target else {
        return row.into_any_element();
    };
    let selector = format!("agent-chat-question-{question}-{field}-write");
    let debug = selector.clone();
    let hover = surface.hover_tint;
    row.id(SharedString::from(selector))
        .debug_selector(move || debug.clone())
        .cursor_pointer()
        .when(!highlighted, |el| el.hover(move |style| style.bg(hover)))
        .on_click(
            cx.listener(move |view, _, _, cx| view.request_question_composer(question, field, cx)),
        )
        .into_any_element()
}

fn write_row_content(frame: Div, typed: Option<String>, surface: &PaneSurfaceTokens) -> Div {
    match typed {
        Some(typed) => frame.child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .child(SharedString::from(typed)),
        ),
        None => frame.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .gap(px(theme::GAP_SM))
                .child(
                    div()
                        .flex_none()
                        .text_color(surface.foreground_muted)
                        .child(SharedString::from(s::agent_chat::question_write())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(surface.foreground_subtle)
                        .whitespace_normal()
                        .child(SharedString::from(s::agent_chat::question_write_hint())),
                ),
        ),
    }
}

/// Skip and send, with a word on what is still unanswered. A form that is one
/// select sends on the pick, so it offers send only once text is committed
/// beside that select: a pick would then no longer send on its own.
fn actions(
    open: &OpenQuestion,
    dim: f32,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> Div {
    let id = open.id;
    let ready = open.draft.answer(&open.request).is_some();
    let offers_send = open.offers_send();
    let unanswered = primary_fields(&open.request)
        .filter(|field| !open.draft.is_answered(&open.request, &field.key))
        .count();
    let skip = format!("agent-chat-question-{id}-skip");
    let submit = format!("agent-chat-question-{id}-submit");
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_end()
        .border_t_1()
        .border_color(surface.border_tint)
        .pt(px(theme::PAD_STANDARD))
        .gap(px(theme::GAP_SM))
        .when(offers_send && ready && unanswered > 0, |row| {
            row.child(
                div()
                    .text_color(surface.foreground_subtle)
                    .child(SharedString::from(s::agent_chat::question_unanswered(
                        unanswered,
                    ))),
            )
        })
        .child(
            daruda_ui::button_on_surface(
                SharedString::from(skip.clone()),
                SharedString::from(s::agent_chat::question_skip()),
                surface,
                cx,
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(surface.foreground_muted)
                    .hover(surface.hover_tint)
                    .active(surface.active_tint),
            )
            .debug_selector(move || skip.clone())
            .on_click(cx.listener(move |view, _, _, cx| view.skip_question(id, cx))),
        )
        .when(offers_send, |row| {
            // Lit by the keyboard, send reads as a lit row does: the cursor
            // and the active fill around it. The cursor keeps its place while
            // hidden, so lighting send does not shift the buttons.
            let lit = open.send_highlighted();
            row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::GAP_SM))
                    .px(px(theme::PAD_XS))
                    .rounded(px(theme::RADIUS_SM))
                    .when(lit, |el| el.bg(surface.active_tint))
                    .child(
                        div()
                            .flex_none()
                            .when(!lit, |el| el.invisible())
                            .child(GLYPH_HIGHLIGHT),
                    )
                    // The primary action uses the workspace interaction palette.
                    .child(
                        daruda_ui::button(
                            SharedString::from(submit.clone()),
                            SharedString::from(s::agent_chat::question_submit()),
                        )
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(theme::dim_toward_gray(theme::PRIMARY, dim))
                                .foreground(if ready {
                                    theme::dim_toward_gray(theme::TEXT_PRIMARY, dim)
                                } else {
                                    surface.foreground_subtle
                                })
                                .hover(theme::dim_toward_gray(theme::ACCENT_HOVER, dim))
                                .active(theme::dim_toward_gray(theme::PRIMARY, dim)),
                        )
                        .when(!ready, |button| {
                            button
                                .child(
                                    div().text_color(surface.foreground_subtle).child(
                                        SharedString::from(s::agent_chat::question_submit()),
                                    ),
                                )
                                .label(SharedString::default())
                        })
                        .disabled(!ready)
                        .debug_selector(move || submit.clone())
                        .on_click(
                            cx.listener(move |view, _, _, cx| {
                                view.submit_question_from_card(id, cx)
                            }),
                        ),
                    ),
            )
        })
}
