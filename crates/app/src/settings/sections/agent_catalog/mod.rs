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
use crate::ui::theme;
use daruda_config::PresetLaunchability;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, Window, div, prelude::*, px};

use super::super::{
    AgentCatalogItem, AgentCatalogRow, CardFold, SettingsView, settings_button as button,
    settings_button_danger as button_danger,
};

mod available;
mod card;
mod groups;

/// The `transport_select` value that means "run the command locally" — the only
/// transport a preset reference can carry (see [`daruda_config::AgentEntry`]).
const TRANSPORT_RAW: &str = "raw";

impl SettingsView {
    /// The `[[agents]]` catalog: the entries in use, then the built-in presets
    /// no entry uses yet — runnable ones first, then the ones that need an
    /// install — and the entries that resolve to nothing.
    pub(in crate::settings) fn render_agent_catalog(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let description_color = theme::current(cx).text_muted;
        let used: Vec<String> = self
            .agent_catalog
            .iter()
            .filter_map(|item| match item {
                AgentCatalogItem::Editable(row) => row.preset.clone(),
                AgentCatalogItem::Unresolved(entry) => entry.preset_id().map(str::to_string),
            })
            .collect();
        let used: Vec<&str> = used.iter().map(String::as_str).collect();
        let query = self.agent_catalog_search.read(cx).value().to_string();
        let presets = groups::preset_groups(daruda_config::agent_presets(), &used, &query);

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
            .child(Self::section_label(s::settings_agent_group_in_use(), cx));

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
        for (ordinal, (catalog_index, row)) in self.agent_editable_rows().enumerate() {
            body = body.child(self.render_agent_card(catalog_index, ordinal, row, cx));
        }
        body = body.child(div().flex().flex_row().child(
            button("settings-agent-add-custom", s::settings_agent_add_custom()).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.add_custom_agent_row(window, cx);
                }),
            ),
        ));

        if self.agent_unresolved_entries().next().is_some() {
            body = body.child(Self::section_label(
                s::settings_agent_unresolved_section(),
                cx,
            ));
            for (catalog_index, entry) in self.agent_unresolved_entries() {
                body = body.child(Self::render_unresolved_entry(catalog_index, entry, cx));
            }
        }

        body.child(self.render_preset_lists(presets, cx))
            .into_any_element()
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

    /// Switch a built-in preset on: append an entry that references it. The
    /// card arrives folded — picking an agent to use is the whole gesture. A
    /// preset that needs a manual install has no command, so it adds nothing.
    pub(in crate::settings) fn enable_agent_preset(
        &mut self,
        preset_id: &str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(definition) = daruda_config::AgentDefinition::registry_preset(preset_id) else {
            return;
        };
        self.add_agent_row(definition, Some(preset_id.to_string()), window, cx);
    }

    /// Switch an entry on or off, keeping every field it states. The last
    /// entry still on cannot be switched off — the card disables its switch —
    /// so this refuses it too rather than trust the caller.
    pub(in crate::settings) fn set_agent_enabled(
        &mut self,
        catalog_index: usize,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if !enabled && self.agent_is_last_enabled(catalog_index) {
            return;
        }
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        if row.enabled == enabled {
            return;
        }
        row.enabled = enabled;
        // Nothing reached disk, so the row must not go on claiming it did.
        if !self.persist_agent_catalog(cx)
            && let Some(row) = self.agent_editable_row_mut(catalog_index)
        {
            row.enabled = !enabled;
        }
        cx.notify();
    }

    /// Make an entry the default by moving it to the front: the first entry
    /// that is on is the one a new chat opens with.
    pub(in crate::settings) fn make_agent_default(
        &mut self,
        catalog_index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        // Only an entry that is on can be the one a new chat opens with; the
        // card hides the button otherwise, and this refuses it too.
        let switched_on = matches!(
            self.agent_catalog.get(catalog_index),
            Some(AgentCatalogItem::Editable(row)) if row.enabled
        );
        if catalog_index == 0 || !switched_on {
            return;
        }
        let item = self.agent_catalog.remove(catalog_index);
        self.agent_catalog.insert(0, item);
        if !self.persist_agent_catalog(cx) {
            let item = self.agent_catalog.remove(0);
            self.agent_catalog.insert(catalog_index, item);
        }
        cx.notify();
    }

    /// The catalog index of the entry a new chat opens with — the first
    /// editable one that is switched on.
    pub(in crate::settings) fn agent_default_index(&self) -> Option<usize> {
        self.agent_editable_rows()
            .find(|(_, row)| row.enabled)
            .map(|(index, _)| index)
    }

    /// Whether `catalog_index` is the only entry still switched on.
    pub(in crate::settings) fn agent_is_last_enabled(&self, catalog_index: usize) -> bool {
        let enabled: Vec<bool> = self
            .agent_catalog
            .iter()
            .map(|item| matches!(item, AgentCatalogItem::Editable(row) if row.enabled))
            .collect();
        groups::is_last_enabled(&enabled, catalog_index)
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

impl AgentCatalogRow {
    /// Whether the command field runs something other than the preset's
    /// command. A custom row has no preset to differ from.
    pub(in crate::settings) fn command_overridden(&self, cx: &gpui::App) -> bool {
        self.preset_definition()
            .is_some_and(|base| match base.launch {
                daruda_config::AgentLaunch::Raw(command) => {
                    differs(&self.command_input.read(cx).value(), &command)
                }
                _ => false,
            })
    }

    /// Whether the environment field states something other than the
    /// preset's own environment. A field that states nothing follows the
    /// preset; one that states pairs is compared on them, so reformatting a
    /// line is not a difference.
    pub(in crate::settings) fn env_overridden(&self, cx: &gpui::App) -> bool {
        self.preset.is_some()
            && !matches!(self.stated_env(cx), Ok(None))
            && !super::agent_env::env_follows_base(
                &self.env_input.read(cx).value(),
                &self.preset_env().unwrap_or_default(),
            )
    }

    fn preset_definition(&self) -> Option<daruda_config::AgentDefinition> {
        self.preset
            .as_deref()
            .and_then(daruda_config::AgentDefinition::registry_preset)
    }

    /// The environment this row writes — `Err` naming what the user has to
    /// fix first. Resolved against the environment the field was built from,
    /// since that is what decides whether an emptied field clears it or
    /// simply states none (see [`super::agent_env::stated_env`]).
    pub(in crate::settings) fn stated_env(
        &self,
        cx: &gpui::App,
    ) -> Result<Option<Vec<(String, String)>>, super::agent_env::EnvFieldError> {
        super::agent_env::stated_env(
            &self.env_input.read(cx).value(),
            self.env_field_base.as_deref(),
        )
    }

    /// The environment this row's preset ships, `None` for a custom row or a
    /// preset that ships none.
    fn preset_env(&self) -> Option<Vec<(String, String)>> {
        self.preset_definition().and_then(|preset| preset.env)
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

/// Whether a field's `current` text states something other than `base`.
/// Surrounding whitespace is trimmed on save, so it is no difference.
fn differs(current: &str, base: &str) -> bool {
    current.trim() != base
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
    /// Test-only entry into [`Self::add_custom_agent_row`] — the click
    /// handler that drives it lives inside a closure and isn't directly
    /// callable from tests.
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
    use super::{differs, transport_needs_local_path_check};

    #[test]
    fn an_untouched_field_reports_no_override() {
        assert!(!differs("Codex", "Codex"));
        // Trailing whitespace is trimmed on save, so it is not an override.
        assert!(!differs("  Codex  ", "Codex"));
    }

    #[test]
    fn a_changed_field_differs() {
        assert!(differs("My Codex", "Codex"));
        assert!(differs("", "Codex"));
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
