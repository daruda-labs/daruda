//! Agent page: send-key policy, the `[[agents]]` catalog, and the Claude
//! status hook toggle.
//!
//! The catalog editor reads the **persisted** layer (`Config.agents`, split at
//! window-open time into [`SettingsView::agent_rows`] plus
//! [`SettingsView::agent_unresolved_entries`]) rather than the resolved
//! runtime catalog — an entry that resolves to nothing has to stay visible, or
//! the user has no way to find out why an agent never shows up.
//!
//! Method visibility is `pub(in crate::settings)` so `render` can
//! dispatch here, mirroring the [`super::plugin`] submodule.

use crate::surface::strings as s;
use crate::ui::field_row;
use crate::ui::theme;
use daruda_config::PresetLaunchability;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, Window, div, prelude::*, px};

use super::super::{
    AgentCatalogRow, CardFold, SettingsView, settings_button as button,
    settings_button_danger as button_danger,
};

mod card;

/// The `transport_select` value that means "run the command locally" — the only
/// transport a preset reference can carry (see [`daruda_config::AgentEntry`]).
const TRANSPORT_RAW: &str = "raw";

impl SettingsView {
    /// The `[[agents]]` catalog: preset picker, editable rows, and the entries
    /// that resolve to nothing.
    pub(in crate::settings) fn render_agent_catalog(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let description_color = theme::current(cx).text_muted;
        let needs_install = self.selected_preset_needs_install(cx);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
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
            body = body.child(self.render_agent_card(catalog_index, ordinal, row, cx));
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
                            move |this, _: &ClickEvent, window, cx| {
                                this.request_remove_agent_catalog_item(index, window, cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    /// A labelled control plus the value it inherits when the row states none.
    /// Eight fields render this trio; the base line is what tells an override
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
    pub(in crate::settings) fn selected_preset_needs_install(
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
        self.open_last_agent_card(
            CardFold {
                expanded: true,
                advanced: false,
            },
            cx,
        );
    }

    /// Append a blank row the user fills in by hand — it references no preset.
    fn add_custom_agent_row(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.add_agent_row(
            daruda_config::AgentDefinition::new(
                String::new(),
                String::new(),
                daruda_config::AgentLaunch::Raw(String::new()),
            ),
            None,
            window,
            cx,
        );
        // A custom row has no command yet, and the command lives in the
        // advanced block — so that block opens with it.
        self.open_last_agent_card(
            CardFold {
                expanded: true,
                advanced: true,
            },
            cx,
        );
    }

    /// Set the fold of the row just appended, so an added agent opens ready
    /// to edit instead of arriving folded.
    fn open_last_agent_card(&mut self, fold: CardFold, cx: &mut gpui::Context<Self>) {
        if let Some(index) = self.agent_catalog.len().checked_sub(1)
            && let Some(row) = self.agent_editable_row_mut(index)
        {
            row.fold = fold;
            cx.notify();
        }
    }
}

/// Where a catalog row's values come from, and which of them the row states
/// differently from that source. Each `*_base` holds the ready-to-render label
/// for the preset's own value, and is `None` when the field still follows the
/// preset (or when the row has no preset at all).
pub(in crate::settings) struct RowProvenance {
    /// The preset this row references, `None` for a custom row.
    pub(in crate::settings) preset: Option<String>,
    pub(in crate::settings) name_base: Option<String>,
    pub(in crate::settings) command_base: Option<String>,
    pub(in crate::settings) default_mode_base: Option<String>,
    pub(in crate::settings) default_model_base: Option<String>,
    pub(in crate::settings) fold_mode_base: Option<String>,
    pub(in crate::settings) tail_window_base: Option<String>,
    pub(in crate::settings) tail_window_calls_base: Option<String>,
    pub(in crate::settings) display_filter_base: Option<String>,
    pub(in crate::settings) env_base: Option<String>,
}

impl RowProvenance {
    fn follows_preset(&self) -> bool {
        self.preset.is_some()
    }

    /// Whether any field departs from the preset — the aggregate the
    /// provenance tests assert on.
    #[cfg(test)]
    pub(in crate::settings) fn is_overridden(&self) -> bool {
        self.name_base.is_some()
            || self.command_base.is_some()
            || self.default_mode_base.is_some()
            || self.default_model_base.is_some()
            || self.fold_mode_base.is_some()
            || self.tail_window_base.is_some()
            || self.tail_window_calls_base.is_some()
            || self.display_filter_base.is_some()
            || self.env_base.is_some()
    }
}

impl AgentCatalogRow {
    /// Diff this row's current field values against the preset it references.
    pub(in crate::settings) fn provenance(&self, cx: &gpui::App) -> RowProvenance {
        let Some(preset) = self.preset.clone() else {
            return RowProvenance {
                preset: None,
                name_base: None,
                command_base: None,
                default_mode_base: None,
                default_model_base: None,
                fold_mode_base: None,
                tail_window_base: None,
                tail_window_calls_base: None,
                display_filter_base: None,
                env_base: None,
            };
        };
        // A row only carries a preset id it resolved from, so the lookup holds.
        let base = daruda_config::AgentDefinition::registry_preset(&preset);
        let base_command = match base.as_ref().map(|b| &b.launch) {
            Some(daruda_config::AgentLaunch::Raw(command)) => command.clone(),
            _ => String::new(),
        };
        let base_env = base
            .as_ref()
            .and_then(|b| b.env.clone())
            .unwrap_or_default();
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
            tail_window_calls_base: self
                .tail_window_calls(cx)
                .map(|_| s::settings_agent_override_preset_value_unset()),
            display_filter_base: self
                .display_filter()
                .map(|_| s::settings_agent_override_preset_value_unset()),
            // Diffed on parsed pairs rather than on the raw text, so
            // reformatting a line is not reported as an override — and a
            // preset that ships no environment shows "not set" like the
            // other stateless axes rather than an empty value.
            env_base: (!super::agent_env::env_follows_base(
                &self.env_input.read(cx).value(),
                &base_env,
            ))
            .then(|| {
                if base_env.is_empty() {
                    s::settings_agent_override_preset_value_unset()
                } else {
                    s::settings_agent_override_preset_value(&super::agent_env::env_base_summary(
                        &base_env,
                    ))
                }
            }),
        }
    }

    /// The environment this row writes — `Err` naming what the user has to
    /// fix first. Resolved against the preset's own environment, since that
    /// is what decides whether an emptied field clears it or simply states
    /// none (see [`super::agent_env::stated_env`]).
    pub(in crate::settings) fn stated_env(
        &self,
        cx: &gpui::App,
    ) -> Result<Option<Vec<(String, String)>>, super::agent_env::EnvFieldError> {
        super::agent_env::stated_env(
            &self.env_input.read(cx).value(),
            self.preset_env().as_deref(),
        )
    }

    /// The environment this row's preset ships, `None` for a custom row or a
    /// preset that ships none.
    fn preset_env(&self) -> Option<Vec<(String, String)>> {
        self.preset
            .as_deref()
            .and_then(daruda_config::AgentDefinition::registry_preset)
            .and_then(|preset| preset.env)
    }

    /// Whether the adapter this row launches gets codex's native-subagent
    /// overlay — the row's own environment when it states one, its preset's
    /// otherwise.
    ///
    /// Keyed on the variable, not on a preset id: a `claude` row must not be
    /// told what the Codex preset ships, a hand-built codex row should be,
    /// and a second preset carrying the same overlay needs no second
    /// condition here.
    pub(in crate::settings) fn ships_codex_subagent_overlay(&self, cx: &gpui::App) -> bool {
        fn carries(env: &[(String, String)]) -> bool {
            env.iter()
                .any(|(name, _)| name == daruda_config::CODEX_CONFIG_ENV)
        }
        match self.stated_env(cx) {
            Ok(Some(env)) => carries(&env),
            // The field states nothing, or is mid-edit and does not parse; the
            // launch falls back to the preset's own environment either way.
            Ok(None) | Err(_) => self.preset_env().is_some_and(|env| carries(&env)),
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
impl SettingsView {
    /// Test-only entry into [`Self::add_selected_preset_row`] — the click
    /// handler that drives it lives inside a closure and isn't directly
    /// callable from tests.
    pub(in crate::settings) fn add_selected_preset_row_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.add_selected_preset_row(window, cx);
    }

    /// Test-only entry into [`Self::add_custom_agent_row`] — same reason as
    /// [`Self::add_selected_preset_row_for_test`].
    pub(in crate::settings) fn add_custom_agent_row_for_test(
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
