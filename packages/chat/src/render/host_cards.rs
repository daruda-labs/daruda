use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::host_cards::{HostCardEntry, HostCardStatus};
use crate::view::AgentChatView;
use daruda_ui::theme::{self, PaneSurfaceTokens};

const MAX_VISIBLE_TEXT_ROWS: f32 = 8.0;

pub(super) fn host_card_strip(
    view: &AgentChatView,
    cx: &mut Context<AgentChatView>,
) -> Option<AnyElement> {
    if view.host_cards.entries().is_empty() {
        return None;
    }
    let surface = PaneSurfaceTokens::agent_chat(cx).dimmed(view.dim_amount);
    let max_height =
        theme::agent_chat_font_size(cx) * theme::agent_chat_line_height(cx) * MAX_VISIBLE_TEXT_ROWS;
    let cards = view
        .host_cards
        .entries()
        .iter()
        .map(|entry| host_card(entry, &surface, cx))
        .collect::<Vec<_>>();

    Some(
        div()
            .id(("agent-chat-host-cards", view.pane_id as usize))
            .debug_selector(|| "agent-chat-host-cards".into())
            .flex_none()
            .w_full()
            .max_h(px(max_height))
            .overflow_y_scroll()
            .track_scroll(&view.host_cards_scroll)
            .flex()
            .flex_col()
            .gap(px(theme::GAP_SM))
            .px(px(theme::PAD_STANDARD))
            .py(px(theme::PAD_XS))
            .border_t_1()
            .border_color(surface.border_tint)
            .children(cards)
            .into_any_element(),
    )
}

fn host_card(
    entry: &HostCardEntry,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let card = &entry.card;
    let id = card.id;
    let selector = format!("agent-chat-host-card-{id}");
    let mut root = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .px(px(theme::PAD_STANDARD))
        .py(px(theme::PAD_XS))
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(surface.border_tint)
        .bg(surface.tint)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .text_color(surface.foreground)
        .child(SharedString::from(card.summary.clone()));

    if matches!(entry.status, HostCardStatus::Pending) {
        let mut actions = div().flex().flex_wrap().gap(px(theme::GAP_SM));
        for (index, action) in card.actions.iter().enumerate() {
            let selector = format!("agent-chat-host-card-action-{id}-{index}");
            let action_id = action.id.clone();
            actions = actions.child(
                daruda_ui::button_chip_on_surface(
                    SharedString::from(selector.clone()),
                    SharedString::from(action.label.clone()),
                    surface,
                    cx,
                )
                .debug_selector(move || selector.clone())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.choose_host_card(id, action_id.clone(), cx)
                })),
            );
        }
        root = root.child(actions);
    } else if let Some(label) = entry.status.label() {
        let selector = format!("agent-chat-host-card-result-{id}");
        root = root.child(
            div()
                .debug_selector(move || selector.clone())
                .text_color(surface.foreground_muted)
                .child(SharedString::from(label.to_owned())),
        );
    }
    root.into_any_element()
}

#[cfg(test)]
mod tests;
