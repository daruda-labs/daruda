//! "Agent integration disabled" banner shown at the top of the
//! lanes view. Click → install action via the workspace confirm
//! dialog.

use crate::ui::theme;
use gpui::{ClickEvent, Context, IntoElement, div, prelude::*, px};

use crate::surface::strings as surface_strings;
use crate::ui::dialog::ButtonVariant;
use crate::workspace::layout::{Dock, LeftDockSnapshot};

/// "Agent integration disabled" banner. Click → install action.
pub(in crate::workspace) fn agent_install_banner(
    snap: &LeftDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement + use<> {
    let workspace = snap.workspace.clone();
    let t = theme::current(cx);
    let bg = t.agent_banner_bg;
    let border = t.agent_banner_border;
    let hover_bg = t.agent_banner_hover_bg;
    let text = t.text_body;
    let icon = t.agent_banner_icon;
    let hint_text = t.text_subtle;
    div()
        .id("agent-install-banner")
        .mx(px(theme::AGENT_BANNER_MARGIN_X))
        .my(px(theme::AGENT_BANNER_MARGIN_Y))
        .px(px(theme::AGENT_BANNER_PAD_X))
        .py(px(theme::AGENT_BANNER_PAD_Y))
        .rounded(px(theme::AGENT_BANNER_RADIUS))
        .bg(bg)
        .border_1()
        .border_color(border)
        .hover(move |d| d.bg(hover_bg))
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_BANNER_GAP))
        .text_size(px(theme::AGENT_BANNER_FONT_SIZE))
        .text_color(text)
        .child(
            div()
                .flex_none()
                .text_color(icon)
                .child(surface_strings::AGENT_BANNER_ICON),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .child(surface_strings::agent_banner_title())
                .child(
                    div()
                        .text_color(hint_text)
                        .child(surface_strings::agent_banner_hint()),
                ),
        )
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            let Some(_ws) = workspace.upgrade() else {
                return;
            };
            let weak = workspace.clone();
            crate::workspace::dialog_helpers::open_confirm_dialog(
                surface_strings::agent_consent_title(),
                surface_strings::agent_consent_body(),
                surface_strings::agent_consent_confirm(),
                ButtonVariant::Primary,
                move |_, window, app_cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(app_cx, |ws, cx| {
                            ws.on_install_agent_hooks(
                                &crate::workspace::InstallAgentHooks,
                                window,
                                cx,
                            );
                        });
                    }
                },
                window,
                cx,
            );
        }))
}
