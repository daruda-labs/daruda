//! The inline permission card and its choice buttons.

use super::*;

/// Inline permission card — title + one button per choice. Once resolved,
/// the buttons are gone and the chosen option is shown instead.
pub(in crate::render) fn permission_card(
    ix: usize,
    card: &PermissionItem,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = card
        .tool_title
        .clone()
        .unwrap_or_else(s::agent_chat::permission_title)
        .into();

    let mut root = div()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.banner_warning_bg)
        .child(
            div()
                .text_color(t.banner_warning_text)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat::permission_title())),
        )
        .child(
            div()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(
                    daruda_ui::selectable_text(("agent-chat-perm-title", ix), title)
                        .color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                        .text_size(px(theme::agent_chat_font_size(cx))),
                ),
        );

    match &card.resolved {
        Some(PermissionResolution::Chosen(option_id)) => {
            // Resolved: surface the chosen option's name (fall back to its
            // id) instead of the buttons.
            let chosen = card
                .options
                .iter()
                .find(|o| &o.option_id == option_id)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| option_id.clone());
            root = root.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(format!(
                        "{} {}",
                        s::agent_chat::permission_resolved_prefix(),
                        chosen
                    ))),
            );
        }
        Some(PermissionResolution::Cancelled) => {
            // The turn was cancelled before the user decided — drop the
            // buttons and surface that the request was cancelled.
            root = root.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat::permission_cancelled())),
            );
        }
        None => {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(theme::GAP_SM));
            for (choice_ix, choice) in card.options.iter().enumerate() {
                row = row.child(permission_button(ix, card.id, choice_ix, choice, cx));
            }
            root = root.child(row);
        }
    }

    root
}

/// One permission choice button. Allow kinds use the accent (primary)
/// treatment; reject kinds use the danger treatment.
fn permission_button(
    ix: usize,
    request_id: u64,
    choice_ix: usize,
    choice: &PermissionChoice,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    // Distinct id per (item, choice) pair; string-formatted rather than packed
    // arithmetically, so a card with many choices can't collide.
    let id = SharedString::from(format!("agent-chat-perm-{ix}-{choice_ix}"));
    let label: SharedString = choice.name.clone().into();
    let kind = choice.kind;
    let option_id = choice.option_id.clone();

    let button = match kind {
        PermissionKindView::AllowOnce | PermissionKindView::AllowAlways => {
            daruda_ui::button_primary(id, label)
        }
        PermissionKindView::RejectOnce | PermissionKindView::RejectAlways => {
            daruda_ui::button_danger(id, label)
        }
    };
    button.on_click(cx.listener(move |this, _, _window, cx| {
        this.respond_permission(request_id, option_id.clone(), kind, cx);
    }))
}
