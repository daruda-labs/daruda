//! Configuration success is not evidence of a live MCP connection.
use crate::agent::configuration::Provider;
use crate::surface::strings;
use crate::ui::{WindowExt as _, button, button_primary, theme};
use crate::workspace::{ModalView, dialog_helpers};
use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, Render, Window, div, prelude::*, px,
};

pub(super) fn guide_url(provider: Option<Provider>) -> &'static str {
    match provider {
        Some(Provider::Codex) => "https://developers.openai.com/codex/mcp",
        _ => "https://code.claude.com/docs/en/mcp",
    }
}

pub(super) fn guide_button(provider: Option<Provider>, cx: &App) -> crate::ui::Button {
    crate::ui::button_icon("mcp-connection-guide", crate::ui::icons::INFO, cx)
        .tooltip(strings::mcp::connection_guide())
        .tab_stop(true)
        .tab_index(18)
        .on_click(move |_, _, cx| cx.open_url(guide_url(provider)))
}

pub(in crate::workspace::right_dock) fn open(
    name: String,
    provider: Option<Provider>,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
) {
    dialog_helpers::open_form_modal(
        strings::mcp::saved_title(),
        Some(px(theme::MODAL_PANEL_WIDTH)),
        move |_, cx| Saved {
            focus: cx.focus_handle(),
            name,
            provider,
            disabled,
        },
        window,
        cx,
    );
}

struct Saved {
    focus: FocusHandle,
    name: String,
    provider: Option<Provider>,
    disabled: bool,
}

impl Focusable for Saved {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for Saved {
    fn guards_close(&self) -> bool {
        true
    }
}

impl Render for Saved {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let provider = self.provider;
        let client = if provider == Some(Provider::Codex) {
            strings::library::codex()
        } else {
            strings::library::claude()
        };
        div()
            .id("mcp-saved")
            .debug_selector(|| "mcp-saved".into())
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .tab_group()
            .text_size(px(theme::FONT_SIZE_MD))
            .gap(px(theme::FORM_MODAL_FIELD_SPACING))
            .child(crate::ui::readonly_form_input(self.name.clone(), cx))
            .child(crate::ui::alert::info(
                "mcp-connection-state",
                if self.disabled {
                    strings::mcp::connection_disabled()
                } else {
                    strings::mcp::connection_unverified()
                },
            ))
            .when(!self.disabled, |body| {
                body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(theme::GAP_SM))
                        .child(strings::mcp::connection_client(client))
                        .child(
                            crate::ui::copy_button(
                                "copy-mcp-status",
                                "/mcp".into(),
                                crate::ui::icons::icon(crate::ui::icons::COPY),
                                strings::code_block::copy().into(),
                                crate::ui::icons::icon(crate::ui::icons::CHECK),
                                strings::code_block::copied().into(),
                                window,
                                cx,
                            )
                            .tab_stop(true)
                            .tab_index(0),
                        ),
                )
            })
            .child(
                super::super::form_layout::footer(cx)
                    .child(
                        button("mcp-saved-guide", strings::mcp::connection_guide())
                            .tab_stop(true)
                            .tab_index(1)
                            .on_click(move |_, _, cx| cx.open_url(guide_url(provider))),
                    )
                    .child(
                        button_primary("mcp-saved-close", strings::common::btn_close())
                            .tab_stop(true)
                            .tab_index(2)
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_guides_follow_the_saved_provider() {
        assert_eq!(guide_url(None), guide_url(Some(Provider::Claude)));
        assert_eq!(
            guide_url(Some(Provider::Codex)),
            "https://developers.openai.com/codex/mcp"
        );
        assert_eq!(
            guide_url(Some(Provider::Claude)),
            "https://code.claude.com/docs/en/mcp"
        );
    }
}
