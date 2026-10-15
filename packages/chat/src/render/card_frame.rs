//! Shared pane-local chrome for tools, subagents, and compaction cards.

use daruda_ui::theme;
use gpui::{App, Div, Hsla, div, prelude::*, px};

#[derive(Clone, Copy)]
pub(super) enum CardTone {
    Standard,
    Error,
}

pub(super) struct CardFrame {
    tone: CardTone,
}

impl CardFrame {
    pub(super) fn new(tone: CardTone) -> Self {
        Self { tone }
    }

    pub(super) fn status_color(&self, dim: f32, cx: &App) -> Hsla {
        let color = match self.tone {
            CardTone::Standard => theme::agent_chat_fg_muted(cx),
            CardTone::Error => error_color(cx),
        };
        theme::dim_toward_gray(color, dim)
    }

    pub(super) fn render(self, dim: f32, cx: &App) -> Div {
        let border = match self.tone {
            CardTone::Standard => theme::agent_chat_border_tint(cx),
            CardTone::Error => error_color(cx),
        };
        div()
            .w_full()
            .min_w_0()
            .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
            .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
            .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
            .bg(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
            .border_1()
            .border_color(theme::dim_toward_gray(border, dim))
            .when(matches!(self.tone, CardTone::Error), |card| {
                card.border_dashed()
            })
    }
}

fn error_color(cx: &App) -> Hsla {
    // UI and pane presets are independent; retain the error hue while
    // adapting its lightness to the surface the card actually occupies.
    let mut color = theme::current(cx).banner_error_text;
    color.l = if theme::agent_chat_syntax_is_light(cx) {
        color.l.min(theme::PANE_ERROR_MAX_L_ON_LIGHT)
    } else {
        color.l.max(theme::PANE_ERROR_MIN_L_ON_DARK)
    };
    color
}

#[cfg(test)]
mod tests;
