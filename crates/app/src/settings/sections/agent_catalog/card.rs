//! One catalog entry as a card: a header that names the agent and what it
//! will run, the everyday fields below it, and the advanced block — command,
//! transport, environment — folded away until asked for.
//!
//! Fold state lives on [`AgentCatalogRow::fold`]; every toggle here is a
//! one-line dispatch into the `SettingsView` methods at the bottom.

use crate::surface::strings as s;
use crate::ui::field_row;
use crate::ui::theme;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};

use super::super::super::{
    AgentCatalogRow, SettingsView, settings_button as button,
    settings_button_danger as button_danger,
};
use super::{TRANSPORT_RAW, transport_needs_local_path_check};

impl SettingsView {
    /// `catalog_index` addresses the entry; `ordinal` is its position among
    /// the editable rows, which is what the "Agent N" fallback title shows.
    pub(super) fn render_agent_card(
        &self,
        catalog_index: usize,
        ordinal: usize,
        row: &AgentCatalogRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let header = self.render_agent_card_header(catalog_index, ordinal, row, cx);
        let t = theme::current(cx);
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .p(px(theme::SETTINGS_CARD_PAD))
            .border_1()
            .border_color(t.border)
            .rounded(px(theme::RADIUS_MD))
            // A switched-off entry keeps its card, dimmed, so switching it
            // back on is where the user left it.
            .when(!row.enabled, |card| {
                card.opacity(theme::SETTINGS_DEPENDENT_OFF_OPACITY)
            })
            .child(header);
        if row.fold.expanded {
            card = card.child(self.render_agent_card_details(catalog_index, row, cx));
        }
        card.into_any_element()
    }

    /// Icon, name, badges and the one-line summary, with the chevron that
    /// opens the details.
    fn render_agent_card_header(
        &self,
        catalog_index: usize,
        ordinal: usize,
        row: &AgentCatalogRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        let id = row.id_input.read(cx).value().trim().to_string();
        let name = row.name_input.read(cx).value().trim().to_string();
        let title = if name.is_empty() {
            s::settings_agent_catalog_row_label(ordinal + 1)
        } else {
            name
        };
        let icon = row
            .preset
            .as_deref()
            .and_then(crate::agent::icons::icon_for_agent)
            .or_else(|| crate::agent::icons::icon_for_agent(&id));
        let command = row.command_input.read(cx).value().trim().to_string();
        let (model, mode) = super::super::agent_vocabulary::agent_row_summary(
            &self.agent_vocabulary,
            &id,
            &command,
            &row.default_mode(cx).unwrap_or_default(),
            &row.default_model(cx).unwrap_or_default(),
        );

        let mut title_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.text_primary)
                    .child(title),
            );
        if self.agent_default_index() == Some(catalog_index) {
            title_row = title_row.child(crate::ui::badge::Badge::new(
                s::settings_agent_card_default(),
            ));
        }
        if row.advanced_overridden(cx) {
            title_row = title_row.child(crate::ui::badge::Badge::new(
                s::settings_agent_card_modified(),
            ));
        }

        let enabled = row.enabled;
        let locked = self.agent_is_last_enabled(catalog_index);
        let mut switch = crate::ui::switch(
            SharedString::from(format!("settings-agent-card-enabled-{catalog_index}")),
            enabled,
            cx,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.set_agent_enabled(catalog_index, !enabled, cx);
        }));
        if locked {
            switch = crate::ui::Disableable::disabled(switch, true)
                .tooltip(s::settings_agent_card_last_enabled());
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::MODAL_FOOTER_GAP))
            // The icon and text are the fold target; the switch and chevron
            // beside them are controls of their own, outside it.
            .child(
                div()
                    .id(SharedString::from(format!(
                        "settings-agent-card-header-{catalog_index}"
                    )))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_agent_card_expanded(catalog_index, cx);
                    }))
                    .child(crate::ui::agent_icon(
                        icon,
                        px(theme::SETTINGS_AGENT_CARD_ICON_SIZE),
                        t.text_body,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(title_row)
                            .child(
                                div()
                                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                                    .text_color(t.text_muted)
                                    .truncate()
                                    .child(s::settings_agent_card_summary(&model, &mode)),
                            ),
                    ),
            )
            .child(switch)
            .child(
                div()
                    .debug_selector(move || format!("agent-card-fold-{catalog_index}"))
                    .child(
                        crate::ui::disclosure::disclosure(
                            SharedString::from(format!("settings-agent-card-fold-{catalog_index}")),
                            row.fold.expanded,
                        )
                        .axis(crate::ui::disclosure::DisclosureAxis::Vertical)
                        .color(t.text_muted)
                        .on_toggle(cx.listener(
                            move |this, _: &ClickEvent, _window, cx| {
                                this.toggle_agent_card_expanded(catalog_index, cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    /// The fields most users change — name, session mode, model, transcript —
    /// then the advanced toggle and, when open, the advanced block.
    fn render_agent_card_details(
        &self,
        catalog_index: usize,
        row: &AgentCatalogRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // Built before the theme borrow: these need `&mut Context` to downgrade
        // the window entity their editors dispatch through.
        let fold_control =
            super::super::agent_transcript::editor::fold_mode_control(catalog_index, row, cx);
        let filter_control =
            super::super::agent_transcript::editor::display_filter_control(catalog_index, row, cx);
        let range_control =
            super::super::agent_transcript::editor::range_control(catalog_index, row, cx);
        let advanced = row
            .fold
            .advanced
            .then(|| self.render_agent_card_advanced(catalog_index, row, cx));
        let t = theme::current(cx);

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(field_row(
                s::settings_agent_field_name(),
                crate::ui::input(&row.name_input, cx, 0),
            ))
            .child(field_row(
                s::settings_agent_field_default_mode(),
                crate::ui::select::select(&row.default_mode_select, cx, 0),
            ))
            .child(field_row(
                s::settings_agent_field_default_model(),
                crate::ui::select::select(&row.default_model_select, cx, 0),
            ))
            .child(Self::section_label(
                s::settings_agent_section_transcript(),
                cx,
            ))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.text_muted)
                    .child(s::settings_agent_transcript_description()),
            )
            .child(field_row(s::settings_agent_field_fold_mode(), fold_control))
            .child(field_row(
                s::settings_agent_field_display_filter(),
                filter_control,
            ))
            .child(field_row(s::agent_chat_recent_steps_label(), range_control))
            .when(
                row.enabled && self.agent_default_index() != Some(catalog_index),
                |body| {
                    body.child(
                        div().flex().flex_row().child(
                            button(
                                SharedString::from(format!(
                                    "settings-agent-card-make-default-{catalog_index}"
                                )),
                                s::settings_agent_card_make_default(),
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _window, cx| {
                                    this.make_agent_default(catalog_index, cx);
                                },
                            )),
                        ),
                    )
                },
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "settings-agent-card-advanced-toggle-{catalog_index}"
                    )))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .cursor_pointer()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.text_body)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_agent_card_advanced(catalog_index, cx);
                    }))
                    .child(s::settings_agent_card_advanced())
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("agent-card-advanced-fold-{catalog_index}")
                            })
                            .child(
                                // No click of its own: the row above owns it,
                                // and gpui bubbles a click to every hovered
                                // ancestor, so a second listener would undo it.
                                crate::ui::disclosure::disclosure(
                                    SharedString::from(format!(
                                        "settings-agent-card-advanced-fold-{catalog_index}"
                                    )),
                                    row.fold.advanced,
                                )
                                .color(t.text_muted),
                            ),
                    ),
            )
            .children(advanced)
            .into_any_element()
    }

    /// Id, launch command, transport and environment — what decides which
    /// process runs and how — plus removing the entry outright.
    fn render_agent_card_advanced(
        &self,
        catalog_index: usize,
        row: &AgentCatalogRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        let transport_kind = row
            .transport_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string())
            .unwrap_or_else(|| TRANSPORT_RAW.to_string());
        let remove_id = format!("settings-agent-remove-{catalog_index}");

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .pl(px(theme::SETTINGS_CARD_PAD))
            .border_l_1()
            .border_color(t.border)
            .child(field_row(
                s::settings_agent_field_id(),
                crate::ui::input(&row.id_input, cx, 0),
            ))
            .child(field_row(
                s::settings_agent_field_command(),
                crate::ui::input(&row.command_input, cx, 0),
            ))
            // ssh/docker rows run on a remote host or inside a container, so
            // a command missing from *this* machine's PATH is expected — the
            // cached warning ignores transport (see `AgentCatalogRow::path_warning`),
            // so the exemption is applied here instead of a fresh `which` call.
            .when(transport_needs_local_path_check(&transport_kind), |body| {
                body.when_some(row.path_warning.as_deref(), |body, command| {
                    body.child(crate::ui::alert::warning(
                        SharedString::from(format!("settings-agent-path-warning-{catalog_index}")),
                        s::settings_agent_row_command_not_on_path(command),
                    ))
                })
            })
            .child(field_row(
                s::settings_agent_field_transport(),
                crate::ui::select::select(&row.transport_select, cx, 0),
            ))
            .when(
                transport_kind == "ssh" || transport_kind == "docker",
                |body| {
                    body.child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(t.text_muted)
                            .child(s::settings_agent_transport_deprecated_hint()),
                    )
                },
            );

        // A preset reference is `Raw`-only, so picking a remote transport
        // detaches the row into a custom copy on commit — say so before commit
        // silently drops the preset link.
        if row.preset.is_some() && transport_kind != TRANSPORT_RAW {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.banner_warning_text)
                    .child(s::settings_agent_row_detach_hint()),
            );
        }

        // Only one of host/container is meaningful per transport kind — show
        // just that field, plus a hint pointing at the Lane's own Session
        // Host setting: unless the lane's session_host is unanswered (the
        // legacy fallback), this agent-side host/container is ignored in
        // favor of the lane's — see `Lane::effective_session_host`.
        if transport_kind == "ssh" {
            body = body
                .child(field_row(
                    s::settings_agent_field_host(),
                    crate::ui::input(&row.host_input, cx, 0),
                ))
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(s::settings_agent_remote_path_hint()),
                );
        } else if transport_kind == "docker" {
            body = body
                .child(field_row(
                    s::settings_agent_field_container(),
                    crate::ui::input(&row.container_input, cx, 0),
                ))
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(s::settings_agent_remote_path_hint()),
                );
        }

        body.child(field_row(
            s::settings_agent_field_env(),
            crate::ui::input(&row.env_input, cx, 0),
        ))
        .child(
            div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(t.text_muted)
                .child(s::settings_agent_env_description()),
        )
        // What the overlay does, and what it does not do. Its own banner
        // rather than a trailing clause of the muted paragraph above: the
        // reader this exists for is the one who turned it on, saw no
        // subagents, and needs to be told the setting is not the thing
        // that spawns them. `info`, not `warning` — nothing is wrong here,
        // it is a default-on state whose scope is easy to misread.
        .when(row.ships_codex_subagent_overlay(cx), |body| {
            body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.text_muted)
                    .child(s::settings_agent_env_codex_note()),
            )
            .child(crate::ui::alert::info(
                SharedString::from(format!("settings-agent-env-codex-{catalog_index}")),
                s::settings_agent_env_codex_caveat(),
            ))
        })
        .child(div().flex().flex_row().child(
            button_danger(remove_id, s::settings_agent_remove()).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    this.request_remove_agent_catalog_item(catalog_index, window, cx);
                },
            )),
        ))
        .into_any_element()
    }

    /// Open or fold a card's details. View state only — nothing is saved.
    pub(in crate::settings) fn toggle_agent_card_expanded(
        &mut self,
        catalog_index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(row) = self.agent_editable_row_mut(catalog_index) {
            row.fold.expanded = !row.fold.expanded;
            cx.notify();
        }
    }

    /// Open or fold a card's advanced block. View state only.
    pub(in crate::settings) fn toggle_agent_card_advanced(
        &mut self,
        catalog_index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(row) = self.agent_editable_row_mut(catalog_index) {
            row.fold.advanced = !row.fold.advanced;
            cx.notify();
        }
    }
}

impl AgentCatalogRow {
    /// Whether this row runs something other than its preset would — a
    /// different command, environment or transport. A custom row has no
    /// preset to differ from. The id is left out: it names the entry, it does
    /// not change what runs.
    pub(in crate::settings) fn advanced_overridden(&self, cx: &gpui::App) -> bool {
        let remote = self
            .transport_select
            .read(cx)
            .selected_value()
            .is_some_and(|kind| kind.as_ref() != TRANSPORT_RAW);
        self.preset.is_some() && (self.command_overridden(cx) || self.env_overridden(cx) || remote)
    }
}

#[cfg(test)]
impl SettingsView {
    /// Test-only read of a card's fold state.
    pub(in crate::settings) fn agent_card_fold_for_test(
        &self,
        catalog_index: usize,
    ) -> Option<crate::settings::CardFold> {
        match self.agent_catalog.get(catalog_index)? {
            crate::settings::AgentCatalogItem::Editable(row) => Some(row.fold),
            crate::settings::AgentCatalogItem::Unresolved(_) => None,
        }
    }
}
