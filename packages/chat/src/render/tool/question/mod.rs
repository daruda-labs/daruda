//! A question the agent asked from a tool call, drawn on that call's row: the
//! open form while it waits (`card`), and once settled a folded summary bar
//! (`record`). Options are list rows (number, glyph, label), not chips: the
//! card is conversation structure, and a picked or highlighted state reads in
//! its glyph as well as its fill.

use daruda_acp::{QuestionField, QuestionFieldKind};
use daruda_ui::theme::{self, PaneSurfaceTokens};
use gpui::{Div, SharedString, div, prelude::*, px};

use crate::strings as s;

mod card;
mod record;

pub(in crate::render) use card::{question_card, question_form};
pub(in crate::render) use record::question_record;

const GLYPH_QUESTION: &str = "?";
pub(super) const GLYPH_HIGHLIGHT: &str = "▸";
const GLYPH_RADIO_OFF: &str = "○";
const GLYPH_RADIO_ON: &str = "◉";
const GLYPH_CHECK_OFF: &str = "☐";
const GLYPH_CHECK_ON: &str = "☑";
const GLYPH_WRITE: &str = "✎";

/// A field's title (with the multi-select marker) and description, as the
/// top of its column.
pub(super) fn field_heading(field: &QuestionField, surface: &PaneSurfaceTokens) -> Div {
    let multi = matches!(field.kind, QuestionFieldKind::MultiSelect(_));
    let mut title = field.title.clone().unwrap_or_default();
    if multi {
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(&s::agent_chat::question_multi());
    }
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_XS))
        .when(!title.is_empty(), |block| {
            block.child(
                div()
                    .text_color(surface.foreground_muted)
                    .whitespace_normal()
                    .child(SharedString::from(title)),
            )
        })
        .children(field.description.as_ref().map(|description| {
            div()
                .text_color(surface.foreground_subtle)
                .whitespace_normal()
                .child(SharedString::from(description.clone()))
        }))
}

/// The leading cells every row shares: the highlight cursor (kept in the
/// layout while hidden, so rows do not shift), the number, the glyph.
pub(super) fn row_frame(
    number: Option<usize>,
    highlighted: bool,
    glyph: &'static str,
    surface: &PaneSurfaceTokens,
) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(theme::GAP_SM))
        .px(px(theme::PAD_XS))
        .rounded(px(theme::RADIUS_SM))
        .child(
            div()
                .flex_none()
                .when(!highlighted, |el| el.invisible())
                .child(GLYPH_HIGHLIGHT),
        )
        .children(number.map(|number| {
            div()
                .flex_none()
                .text_color(surface.foreground_subtle)
                .child(SharedString::from(number.to_string()))
        }))
        .child(div().flex_none().child(glyph))
}

pub(super) fn choice_glyph(multi: bool, picked: bool) -> &'static str {
    match (multi, picked) {
        (false, false) => GLYPH_RADIO_OFF,
        (false, true) => GLYPH_RADIO_ON,
        (true, false) => GLYPH_CHECK_OFF,
        (true, true) => GLYPH_CHECK_ON,
    }
}

#[cfg(test)]
mod tests;
