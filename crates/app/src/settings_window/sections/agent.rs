//! Agent page: send-key policy, the `[[agents]]` catalog, and the Claude
//! status hook toggle.
//!
//! The catalog editor reads the **persisted** layer (`Config.agents`, split at
//! window-open time into [`SettingsWindow::agent_rows`] plus
//! [`SettingsWindow::agent_unresolved_entries`]) rather than the resolved
//! runtime catalog — an entry that resolves to nothing has to stay visible, or
//! the user has no way to find out why an agent never shows up.
//!
//! Method visibility is `pub(in crate::settings_window)` so `render` can
//! dispatch here, mirroring the [`super::plugin`] submodule.

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{checkbox, checkbox_row, field_row};
use daruda_config::PresetLaunchability;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, Window, div, prelude::*, px};

use super::super::{
    AgentCatalogRow, BoolSetting, SettingsWindow, settings_button as button,
    settings_button_danger as button_danger,
};

/// The `transport_select` value that means "run the command locally" — the only
/// transport a preset reference can carry (see [`daruda_config::AgentEntry`]).
const TRANSPORT_RAW: &str = "raw";

impl SettingsWindow {
    pub(in crate::settings_window) fn render_agent(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let description_color = theme::current(cx).text_muted;
        let use_modifier_to_send = self.agent_use_modifier_to_send;
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_agent(), cx))
            .child(checkbox_row(
                checkbox(
                    "settings-agent-use-modifier-to-send",
                    s::settings_label_agent_use_modifier_to_send(),
                    0,
                )
                .checked(use_modifier_to_send)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::AgentUseModifierToSend, *checked, cx)
                    {
                        this.agent_use_modifier_to_send = *checked;
                        cx.notify();
                    }
                })),
            ))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_agent_use_modifier_to_send_description()),
            );

        body = body.child(self.render_agent_catalog(cx));

        let claude_status_enable = self.claude_status_enable;
        body = body
            .child(Self::section_label(s::settings_section_claude_status(), cx))
            .child(checkbox_row(
                checkbox(
                    "settings-claude-status-enable",
                    s::settings_label_claude_status_enable(),
                    0,
                )
                .checked(claude_status_enable)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if this.persist_bool_setting(BoolSetting::ClaudeStatusEnabled, *checked, cx) {
                        this.claude_status_enable = *checked;
                        cx.notify();
                    }
                })),
            ));

        body.into_any_element()
    }

    /// The `[[agents]]` catalog: preset picker, editable rows, and the entries
    /// that resolve to nothing.
    fn render_agent_catalog(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let description_color = theme::current(cx).text_muted;
        let needs_install = self.selected_preset_needs_install(cx);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_agent_catalog(), cx))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_agent_catalog_description()),
            )
            .child(field_row(
                s::settings_agent_preset(),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(div().flex_1().child(crate::ui::select::select(
                        &self.agent_preset_select,
                        cx,
                        0,
                    )))
                    .child(match needs_install {
                        Some((_, install_url)) => button(
                            "settings-agent-preset-install",
                            s::settings_agent_preset_install_page(),
                        )
                        .on_click(cx.listener(
                            move |_this, _: &ClickEvent, _window, cx| {
                                cx.open_url(install_url);
                            },
                        )),
                        None => button("settings-agent-add-preset", s::settings_agent_add_preset())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_selected_preset_row(window, cx);
                            })),
                    })
                    .child(
                        button("settings-agent-add-custom", s::settings_agent_add_custom())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_custom_agent_row(window, cx);
                            })),
                    ),
            ));

        if let Some((name, _)) = needs_install {
            body = body.child(crate::ui::alert::info(
                "settings-agent-preset-needs-install",
                s::settings_agent_preset_needs_install_hint(name),
            ));
        }

        // Same predicate catalog validation uses, so the placeholder cannot
        // claim an empty catalog while the same catalog is valid.
        if self.agent_catalog_is_empty() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_agent_catalog_empty()),
            );
        }
        // Editable rows first, non-editable ones grouped under their own header:
        // a visual grouping, while the model keeps both at their config position.
        for (ordinal, (catalog_index, row)) in self.agent_editable_rows().enumerate() {
            body = body.child(self.render_agent_catalog_row(catalog_index, ordinal, row, cx));
        }

        if self.agent_unresolved_entries().next().is_some() {
            body = body.child(Self::section_label(
                s::settings_agent_unresolved_section(),
                cx,
            ));
            for (catalog_index, entry) in self.agent_unresolved_entries() {
                body = body.child(Self::render_unresolved_entry(catalog_index, entry, cx));
            }
        }

        body.into_any_element()
    }

    /// A catalog entry with no editable row: it names a preset daruda cannot
    /// launch, so nothing in the app offers this agent. Saving keeps it, which
    /// is exactly why it needs to be visible here.
    fn render_unresolved_entry(
        index: usize,
        entry: &daruda_config::AgentEntry,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // Only a preset reference can fail to resolve — a `Custom` entry carries
        // its own definition — so `preset_id` is always set here.
        let preset_id = entry.preset_id().unwrap_or_default();
        let (message, install_url) = match daruda_config::agent_preset(preset_id)
            .map(|preset| (preset.name, preset.launchability))
        {
            Some((name, PresetLaunchability::NeedsManualInstall { install_url })) => (
                s::settings_agent_unresolved_needs_install(preset_id, name),
                Some(install_url),
            ),
            // No preset carries that id; a `Runnable` one would have resolved.
            _ => (s::settings_agent_unresolved_unknown(preset_id), None),
        };

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(crate::ui::alert::warning(
                SharedString::from(format!("settings-agent-unresolved-{index}")),
                message,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .when_some(install_url, |row, url| {
                        row.child(
                            button(
                                SharedString::from(format!(
                                    "settings-agent-unresolved-install-{index}"
                                )),
                                s::settings_agent_preset_install_page(),
                            )
                            .on_click(cx.listener(
                                move |_this, _: &ClickEvent, _window, cx| {
                                    cx.open_url(url);
                                },
                            )),
                        )
                    })
                    // Removable like any other entry: without this the user's
                    // only way to drop a preset daruda can no longer launch is
                    // hand-editing `config.toml`.
                    .child(
                        button_danger(
                            SharedString::from(format!("settings-agent-unresolved-remove-{index}")),
                            s::settings_agent_remove(),
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _window, cx| {
                                this.remove_agent_catalog_item(index, cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    /// `catalog_index` addresses the entry for removal; `ordinal` is its
    /// position among the editable rows, which is what the "Agent N" label
    /// shows. They differ as soon as a non-editable entry sits in between.
    fn render_agent_catalog_row(
        &self,
        catalog_index: usize,
        ordinal: usize,
        row: &AgentCatalogRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // Built before the theme borrow: these need `&mut Context` to downgrade
        // the window entity their editors dispatch through.
        let fold_control =
            super::agent_transcript::editor::fold_mode_control(catalog_index, row, cx);
        let filter_control =
            super::agent_transcript::editor::display_filter_control(catalog_index, row, cx);
        let t = theme::current(cx);
        let remove_id = format!("settings-agent-remove-{catalog_index}");
        let transport_kind = row
            .transport_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string())
            .unwrap_or_else(|| TRANSPORT_RAW.to_string());
        let provenance = row.provenance(cx);

        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(t.text_muted)
            .child(provenance.source_label());
        if provenance.is_overridden() {
            header = header.child(
                div()
                    .text_color(t.text_body)
                    .child(s::settings_agent_row_overridden()),
            );
        }

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .p(px(theme::MODAL_PANEL_GAP))
            .border_1()
            .border_color(t.border)
            .rounded(px(theme::RADIUS_MD))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(t.text_primary)
                            .child(s::settings_agent_catalog_row_label(ordinal + 1)),
                    )
                    .child(
                        button_danger(remove_id, s::settings_agent_remove()).on_click(cx.listener(
                            move |this, _: &ClickEvent, _window, cx| {
                                this.remove_agent_catalog_item(catalog_index, cx);
                            },
                        )),
                    ),
            )
            .child(header)
            .child(field_row(
                s::settings_agent_field_id(),
                crate::ui::input(&row.id_input, cx, 0),
            ))
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_name(),
                    crate::ui::input(&row.name_input, cx, 0),
                    provenance.name_base.clone(),
                    cx,
                )
            })
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_command(),
                    crate::ui::input(&row.command_input, cx, 0),
                    provenance.command_base.clone(),
                    cx,
                )
            })
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
            )
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_default_mode(),
                    crate::ui::select::select(&row.default_mode_select, cx, 0),
                    provenance.default_mode_base.clone(),
                    cx,
                )
            })
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_default_model(),
                    crate::ui::select::select(&row.default_model_select, cx, 0),
                    provenance.default_model_base.clone(),
                    cx,
                )
            })
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
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_fold_mode(),
                    fold_control,
                    provenance.fold_mode_base.clone(),
                    cx,
                )
            })
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_tail_window(),
                    crate::ui::select::select(&row.tail_window_select, cx, 0),
                    provenance.tail_window_base.clone(),
                    cx,
                )
            })
            .map(|body| {
                Self::field_with_base(
                    body,
                    s::settings_agent_field_display_filter(),
                    filter_control,
                    provenance.display_filter_base.clone(),
                    cx,
                )
            });

        // A preset reference is `Raw`-only, so picking a remote transport
        // detaches the row into a custom copy on commit — say so before commit
        // silently drops the preset link.
        if provenance.follows_preset() && transport_kind != TRANSPORT_RAW {
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

        body.into_any_element()
    }

    /// A labelled control plus the value it inherits when the row states none.
    /// Seven fields render this trio; the base line is what tells an override
    /// from a row that is simply following its preset.
    fn field_with_base(
        body: gpui::Div,
        label: String,
        control: impl IntoElement,
        base: Option<String>,
        cx: &gpui::App,
    ) -> gpui::Div {
        body.child(field_row(label, control))
            .when_some(base, |body, base| {
                body.child(Self::preset_base_value(base, cx))
            })
    }

    /// The preset's own value for a field the row above overrides — muted, so
    /// the editable value stays the one that reads as current.
    fn preset_base_value(label: String, cx: &gpui::App) -> impl IntoElement {
        div()
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(theme::current(cx).text_muted)
            .child(label)
    }

    /// The preset picked in the dropdown, launchable or not.
    fn selected_preset(&self, cx: &gpui::App) -> Option<daruda_config::AgentPreset> {
        self.agent_preset_select
            .read(cx)
            .selected_value()
            .and_then(|id| daruda_config::agent_preset(id.as_ref()))
    }

    /// `(display name, install page)` when the picked preset ships binaries
    /// instead of a command daruda can run. `Some` is exactly the state in which
    /// the section swaps the Add button for that install page and explains why —
    /// leaving Add in place would make it a button that does nothing.
    pub(in crate::settings_window) fn selected_preset_needs_install(
        &self,
        cx: &gpui::App,
    ) -> Option<(&'static str, &'static str)> {
        let preset = self.selected_preset(cx)?;
        match preset.launchability {
            PresetLaunchability::NeedsManualInstall { install_url } => {
                Some((preset.name, install_url))
            }
            PresetLaunchability::Runnable { .. } => None,
        }
    }

    /// Append a row for the preset currently picked in the dropdown. A preset
    /// that needs a manual install has no command, so it adds nothing — the
    /// section renders its install page instead of this button.
    fn add_selected_preset_row(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(id) = self
            .agent_preset_select
            .read(cx)
            .selected_value()
            .map(|id| id.to_string())
        else {
            return;
        };
        let Some(definition) = daruda_config::AgentDefinition::registry_preset(&id) else {
            return;
        };
        self.add_agent_row(definition, Some(id), window, cx);
    }

    /// Append a blank row the user fills in by hand — it references no preset.
    fn add_custom_agent_row(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.add_agent_row(
            daruda_config::AgentDefinition {
                id: String::new(),
                name: String::new(),
                launch: daruda_config::AgentLaunch::Raw(String::new()),
                default_mode: None,
                default_model: None,
                fold_mode: None,
                tail_window: None,
                display_filter: None,
            },
            None,
            window,
            cx,
        );
    }
}

/// Where a catalog row's values come from, and which of them the row states
/// differently from that source. Each `*_base` holds the ready-to-render label
/// for the preset's own value, and is `None` when the field still follows the
/// preset (or when the row has no preset at all).
pub(in crate::settings_window) struct RowProvenance {
    /// The preset this row references, `None` for a custom row.
    pub(in crate::settings_window) preset: Option<String>,
    pub(in crate::settings_window) name_base: Option<String>,
    pub(in crate::settings_window) command_base: Option<String>,
    pub(in crate::settings_window) default_mode_base: Option<String>,
    pub(in crate::settings_window) default_model_base: Option<String>,
    pub(in crate::settings_window) fold_mode_base: Option<String>,
    pub(in crate::settings_window) tail_window_base: Option<String>,
    pub(in crate::settings_window) display_filter_base: Option<String>,
}

impl RowProvenance {
    fn follows_preset(&self) -> bool {
        self.preset.is_some()
    }

    pub(in crate::settings_window) fn is_overridden(&self) -> bool {
        self.name_base.is_some()
            || self.command_base.is_some()
            || self.default_mode_base.is_some()
            || self.default_model_base.is_some()
            || self.fold_mode_base.is_some()
            || self.tail_window_base.is_some()
            || self.display_filter_base.is_some()
    }

    fn source_label(&self) -> String {
        match &self.preset {
            Some(preset) => s::settings_agent_row_source_preset(preset),
            None => s::settings_agent_row_source_custom(),
        }
    }
}

impl AgentCatalogRow {
    /// Diff this row's current field values against the preset it references.
    pub(in crate::settings_window) fn provenance(&self, cx: &gpui::App) -> RowProvenance {
        let Some(preset) = self.preset.clone() else {
            return RowProvenance {
                preset: None,
                name_base: None,
                command_base: None,
                default_mode_base: None,
                default_model_base: None,
                fold_mode_base: None,
                tail_window_base: None,
                display_filter_base: None,
            };
        };
        // A row only carries a preset id it resolved from, so the lookup holds.
        let base = daruda_config::AgentDefinition::registry_preset(&preset);
        let base_command = match base.as_ref().map(|b| &b.launch) {
            Some(daruda_config::AgentLaunch::Raw(command)) => command.clone(),
            _ => String::new(),
        };
        let base_name = base.map(|b| b.name).unwrap_or_default();
        // Presets state none of the mode, model or transcript axes, so any
        // value on one of them is an override — labelled "not set" rather than
        // shown as an empty preset value.
        RowProvenance {
            preset: Some(preset),
            name_base: overridden_base(&self.name_input.read(cx).value(), &base_name)
                .map(s::settings_agent_override_preset_value),
            command_base: overridden_base(&self.command_input.read(cx).value(), &base_command)
                .map(s::settings_agent_override_preset_value),
            default_mode_base: self
                .default_mode(cx)
                .map(|_| s::settings_agent_override_preset_value_unset()),
            default_model_base: self
                .default_model(cx)
                .map(|_| s::settings_agent_override_preset_value_unset()),
            fold_mode_base: self
                .fold_mode()
                .map(|_| s::settings_agent_override_preset_value_unset()),
            tail_window_base: self
                .tail_window(cx)
                .map(|_| s::settings_agent_override_preset_value_unset()),
            display_filter_base: self
                .display_filter()
                .map(|_| s::settings_agent_override_preset_value_unset()),
        }
    }
}

/// `Some(base)` when the row's `current` value differs from the preset's `base`,
/// i.e. the field is overridden and the preset value is worth showing.
fn overridden_base<'a>(current: &str, base: &'a str) -> Option<&'a str> {
    (current.trim() != base).then_some(base)
}

/// Whether a catalog row's local-PATH warning should be shown for `kind`. An
/// `"ssh"`/`"docker"` row runs its command on a remote host or inside a
/// container, so its own `PATH` — not this machine's — is what matters; any
/// other `kind` (`"raw"`, or an unrecognized/absent select value) runs here.
fn transport_needs_local_path_check(kind: &str) -> bool {
    !matches!(kind, "ssh" | "docker")
}

#[cfg(test)]
impl SettingsWindow {
    /// Test-only entry into [`Self::add_selected_preset_row`] — the click
    /// handler that drives it lives inside a closure and isn't directly
    /// callable from tests.
    pub(in crate::settings_window) fn add_selected_preset_row_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.add_selected_preset_row(window, cx);
    }

    /// Test-only entry into [`Self::add_custom_agent_row`] — same reason as
    /// [`Self::add_selected_preset_row_for_test`].
    pub(in crate::settings_window) fn add_custom_agent_row_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.add_custom_agent_row(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{overridden_base, transport_needs_local_path_check};

    #[test]
    fn an_untouched_field_reports_no_override() {
        assert_eq!(overridden_base("Codex", "Codex"), None);
        // Trailing whitespace is trimmed on save, so it is not an override.
        assert_eq!(overridden_base("  Codex  ", "Codex"), None);
    }

    #[test]
    fn a_changed_field_reports_the_preset_value() {
        assert_eq!(overridden_base("My Codex", "Codex"), Some("Codex"));
        assert_eq!(overridden_base("", "Codex"), Some("Codex"));
    }

    #[test]
    fn ssh_and_docker_rows_never_need_a_local_path_check() {
        assert!(!transport_needs_local_path_check("ssh"));
        assert!(!transport_needs_local_path_check("docker"));
    }

    #[test]
    fn raw_and_unrecognized_kinds_need_the_local_path_check() {
        assert!(transport_needs_local_path_check("raw"));
        assert!(transport_needs_local_path_check(""));
        assert!(transport_needs_local_path_check("bogus"));
    }
}
