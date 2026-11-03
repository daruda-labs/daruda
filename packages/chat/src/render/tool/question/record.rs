//! A settled question: a bar saying how it ended, folding the form as it was
//! answered.

use std::collections::BTreeMap;

use daruda_acp::{QuestionField, QuestionFieldKind, QuestionValue, ToolCallItem};
use daruda_ui::theme::{self, PaneSurfaceTokens};
use gpui::{AnyElement, Context, Div, FontWeight, IntoElement, SharedString, div, prelude::*, px};

use super::{GLYPH_QUESTION, GLYPH_WRITE, choice_glyph, field_heading};
use crate::fold::FoldKey;
use crate::render::fold_header::{FoldHeader, FoldRow};
use crate::strings as s;
use crate::view::AgentChatView;
use crate::view::questions::{QuestionOutcome, QuestionRecord, companion_key, primary_fields};

/// Answer labels the summary bar names before it counts the rest.
const SUMMARY_LABELS: usize = 2;

/// A settled question: a bar that says how it ended, folding the form as it
/// was answered. `key` is the call's own fold key, read for `expanded`.
pub(in crate::render) fn question_record(
    tc: &ToolCallItem,
    record: &QuestionRecord,
    key: FoldKey,
    expanded: bool,
    surface: &PaneSurfaceTokens,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let font = px(theme::agent_chat_font_size(cx));
    let tool = tc.id.clone();
    // A title rather than a collapsed-only summary: how the question ended is
    // not restated by the frozen form below it.
    let header = FoldHeader::with_title(
        div()
            .text_color(surface.foreground_muted)
            .text_size(font)
            .child(SharedString::from(record_summary(record)))
            .into_any_element(),
    )
    .leading(
        div()
            .flex_none()
            .text_color(surface.foreground_muted)
            .text_size(font)
            .child(GLYPH_QUESTION)
            .into_any_element(),
    );
    let surface = *surface;
    div()
        .debug_selector(move || format!("agent-chat-question-record-{tool}"))
        .w_full()
        .min_w_0()
        // The frozen form inherits this; the card sets the same size.
        .text_size(font)
        .child(
            FoldRow::block(
                SharedString::from(format!("agent-chat-question-record-{}", tc.id)),
                key,
                expanded,
                header,
                move |_| frozen_form(record, &surface).into_any_element(),
            )
            .render(dim, cx),
        )
        .into_any_element()
}

/// The form as it was settled: picked options in the foreground, the rest
/// subtle, typed answers on their own line. Nothing here responds to the
/// pointer.
fn frozen_form(record: &QuestionRecord, surface: &PaneSurfaceTokens) -> Div {
    let empty = BTreeMap::new();
    let values = match &record.outcome {
        QuestionOutcome::Answered(values) => values,
        _ => &empty,
    };
    let request = &record.request;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .pl(px(theme::PAD_STANDARD))
        .text_color(surface.foreground)
        .child(
            div()
                .whitespace_normal()
                .child(SharedString::from(request.message.clone())),
        )
        .children(primary_fields(request).map(|field| frozen_field(record, values, field, surface)))
}

/// One question of a settled form, in [`frozen_form`]'s terms.
fn frozen_field(
    record: &QuestionRecord,
    values: &BTreeMap<String, QuestionValue>,
    field: &QuestionField,
    surface: &PaneSurfaceTokens,
) -> Div {
    let value = values.get(&field.key);
    let choices: Vec<(String, bool)> = match &field.kind {
        QuestionFieldKind::SingleSelect(options) | QuestionFieldKind::MultiSelect(options) => {
            options
                .iter()
                .map(|o| (o.label.clone(), value_picks(value, &o.value)))
                .collect()
        }
        QuestionFieldKind::Boolean => [
            (s::agent_chat::question_yes(), true),
            (s::agent_chat::question_no(), false),
        ]
        .into_iter()
        .map(|(label, flag)| (label, value == Some(&QuestionValue::Bool(flag))))
        .collect(),
        QuestionFieldKind::Text => Vec::new(),
    };
    let multi = matches!(field.kind, QuestionFieldKind::MultiSelect(_));
    let single_picked = !multi && !choices.is_empty() && choices.iter().any(|(_, picked)| *picked);
    let typed = match field.kind {
        QuestionFieldKind::Text => value,
        _ => companion_key(&record.request, &field.key).and_then(|key| values.get(key)),
    };
    let typed = match typed {
        Some(QuestionValue::Text(text)) if single_picked => {
            Some(s::agent_chat::question_note(text))
        }
        Some(QuestionValue::Text(text)) => Some(s::agent_chat::question_typed(text)),
        _ => None,
    };
    let choice = |(label, picked): (String, bool)| {
        div()
            .flex()
            .gap(px(theme::GAP_SM))
            .text_color(if picked {
                surface.foreground
            } else {
                surface.foreground_subtle
            })
            .when(picked, |el| el.font_weight(FontWeight::SEMIBOLD))
            .child(choice_glyph(multi, picked))
            .child(SharedString::from(label))
    };
    field_heading(field, surface)
        .when(!choices.is_empty(), |block| {
            block.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(theme::GAP_LG))
                    .children(choices.into_iter().map(choice)),
            )
        })
        .children(typed.map(|typed| {
            div()
                .flex()
                .gap(px(theme::GAP_SM))
                .child(GLYPH_WRITE)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .child(SharedString::from(typed)),
                )
        }))
}

fn value_picks(value: Option<&QuestionValue>, option: &str) -> bool {
    match value {
        Some(QuestionValue::Text(picked)) => picked == option,
        Some(QuestionValue::Choices(picked)) => picked.iter().any(|p| p == option),
        _ => false,
    }
}

/// The summary bar's line: how many questions and how they ended, and for an
/// answer the first few labels it gave.
pub(super) fn record_summary(record: &QuestionRecord) -> String {
    let asked = record.asked();
    let ended = |outcome: String| s::agent_chat::question_record_ended(asked, outcome);
    match &record.outcome {
        QuestionOutcome::Answered(_) => {
            let separator = s::agent_chat::question_separator();
            let (labels, rest) = record.answer_preview(SUMMARY_LABELS);
            let mut line = s::agent_chat::question_record_answered(record.answered());
            for label in labels {
                line.push_str(&separator);
                line.push_str(&label);
            }
            if rest > 0 {
                line = s::agent_chat::question_record_more(line, rest);
            }
            line
        }
        QuestionOutcome::Skipped => s::agent_chat::question_record_skipped(asked),
        QuestionOutcome::SkippedForPrompt => {
            ended(s::agent_chat::question_record_skipped_for_prompt())
        }
        QuestionOutcome::Cancelled => ended(s::agent_chat::question_record_cancelled()),
        QuestionOutcome::Withdrawn => ended(s::agent_chat::question_record_withdrawn()),
        QuestionOutcome::Expired { after_secs } => ended(s::agent_chat::question_record_expired(
            wait_label(*after_secs),
        )),
    }
}

/// How long a question waited: whole minutes as such ("5m"), anything else
/// in the pane's elapsed-time format.
fn wait_label(secs: u32) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        s::agent_chat::question_timeout_minutes(secs / 60)
    } else {
        crate::render::format_elapsed(std::time::Duration::from_secs(u64::from(secs)))
    }
}
