//! Where a catalog row's mode / model pickers get their options.
//!
//! Two sources, consulted **per axis independently**: what the agent last
//! advertised ([`AgentVocabularyCache`], keyed on the row's id and command) and the
//! build-time seed for the adapter the row's command names
//! ([`daruda_config::agent_vocabulary_seed`]). A live list on one axis must
//! not erase the seed on the other.
//!
//! Both keys are editable after the row was built, so the lists are rebuilt in
//! place by [`SettingsWindow::refresh_agent_row_vocabulary`] rather than at
//! render time — building them in `render` would mean mutating select state
//! mid-paint.

use crate::surface::strings as s;
use crate::ui::select::{SelectOption, SelectState};
use daruda_store::agent_vocabulary::{AgentVocabularyCache, VocabEntry};
use gpui::{Entity, SharedString, Window};

use super::super::{AgentCatalogItem, AgentCatalogRow, SettingsWindow};

impl SettingsWindow {
    /// Rebuild one row's mode/model option lists from the row's current id and
    /// command. Both are editable after the row was constructed, so the lists
    /// are re-sourced here rather than at render time (building them in
    /// `render` would mean mutating the select state mid-paint). The
    /// `SelectState` entities are reused, never replaced, so the row's
    /// subscriptions stay wired to them.
    pub(in crate::settings_window) fn refresh_agent_row_vocabulary(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(AgentCatalogItem::Editable(row)) = self.agent_catalog.get(index) else {
            return;
        };
        let row = row.clone();
        let agent_id = row.id_input.read(cx).value().trim().to_string();
        let command = row.command_input.read(cx).value().trim().to_string();
        let mode = selected_value(&row.default_mode_select, cx);
        let model = selected_value(&row.default_model_select, cx);
        let (mode_options, model_options) = agent_row_vocabulary_options(
            &self.agent_vocabulary,
            &agent_id,
            &command,
            &mode,
            &model,
        );
        set_options(&row.default_mode_select, mode_options, &mode, window, cx);
        set_options(&row.default_model_select, model_options, &model, window, cx);
        cx.notify();
    }
}

impl AgentCatalogRow {
    /// The pinned session mode, or `None` for the empty "agent default"
    /// sentinel. The one reading both collect paths and `provenance` share,
    /// so none of them can disagree about what "no override" looks like.
    pub(in crate::settings_window) fn default_mode(&self, cx: &gpui::App) -> Option<String> {
        selected_override(&self.default_mode_select, cx)
    }

    /// The pinned model, same sentinel as [`Self::default_mode`].
    pub(in crate::settings_window) fn default_model(&self, cx: &gpui::App) -> Option<String> {
        selected_override(&self.default_model_select, cx)
    }
}

/// A picker's value as an override — `None` for the empty sentinel that means
/// "let the agent use its own default".
fn selected_override(state: &Entity<SelectState>, cx: &gpui::App) -> Option<String> {
    let value = state.read(cx).selected_value()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// A picker's raw value, empty string when nothing is selected — the shape
/// [`vocabulary_options`] takes as `saved` and [`set_options`] re-selects.
fn selected_value(state: &Entity<SelectState>, cx: &gpui::App) -> SharedString {
    state.read(cx).selected_value().cloned().unwrap_or_default()
}

/// Swap a picker's options and re-select `value`. `set_items` alone leaves the
/// selection pointing at the old list's index, so the value is re-resolved
/// against the new one; [`vocabulary_options`] always carries `value`, so this
/// never clears the row's pick.
fn set_options(
    state: &Entity<SelectState>,
    options: Vec<SelectOption>,
    value: &SharedString,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    state.update(cx, |state, cx| {
        state.set_items(options, window, cx);
        state.set_selected_value(value, window, cx);
    });
}

/// The `(modes, models)` option lists for a row whose id is `agent_id` and
/// whose command is `command`. Cache first, then seed, per axis.
pub(in crate::settings_window) fn agent_row_vocabulary_options(
    vocabulary: &AgentVocabularyCache,
    agent_id: &str,
    command: &str,
    saved_mode: &str,
    saved_model: &str,
) -> (Vec<SelectOption>, Vec<SelectOption>) {
    let seed = daruda_config::agent_vocabulary_seed(command);
    let (seed_modes, seed_models) = match seed.as_ref() {
        Some(seed) => (seed.modes.as_slice(), seed.models.as_slice()),
        None => (&[][..], &[][..]),
    };
    (
        vocabulary_options(
            known_axis(vocabulary.known_modes_for(agent_id, command), seed_modes),
            seed.as_ref().and_then(|seed| seed.default_mode.as_deref()),
            saved_mode,
        ),
        vocabulary_options(
            known_axis(vocabulary.known_models_for(agent_id, command), seed_models),
            seed.as_ref().and_then(|seed| seed.default_model.as_deref()),
            saved_model,
        ),
    )
}

/// What the agent last advertised on one axis, or the adapter seed until it
/// has advertised anything there.
fn known_axis<'a>(cached: Option<&'a [VocabEntry]>, seeded: &'a [VocabEntry]) -> &'a [VocabEntry] {
    cached.unwrap_or(seeded)
}

/// The `(agent default[ — name])` entry first, then the vocabulary, then
/// `saved` if the vocabulary does not list it — so a value set before the
/// agent was ever connected is never silently dropped.
fn vocabulary_options(
    entries: &[VocabEntry],
    adapter_default: Option<&str>,
    saved: &str,
) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new(
        "",
        agent_default_label(entries, adapter_default),
    )];
    options.extend(
        entries
            .iter()
            .map(|entry| SelectOption::new(entry.id.clone(), entry.name.clone())),
    );
    if !saved.is_empty() && !entries.iter().any(|entry| entry.id == saved) {
        options.push(SelectOption::new(saved.to_string(), saved.to_string()));
    }
    options
}

/// Label for the "no override" entry. It names the adapter's own default when
/// daruda knows it, so picking it is a stated choice rather than a blank.
fn agent_default_label(entries: &[VocabEntry], adapter_default: Option<&str>) -> String {
    let Some(id) = adapter_default else {
        return s::settings_agent_vocabulary_agent_default();
    };
    let name = entries
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.name.as_str())
        .unwrap_or(id);
    s::settings_agent_vocabulary_agent_default_named(name)
}

#[cfg(test)]
impl SettingsWindow {
    /// Test-only — install a known vocabulary cache and re-source every row's
    /// pickers from it, so a test never depends on the developer's real
    /// `agent_vocabulary.json`.
    pub(in crate::settings_window) fn set_agent_vocabulary_for_test(
        &mut self,
        vocabulary: AgentVocabularyCache,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.agent_vocabulary = vocabulary;
        for index in 0..self.agent_catalog.len() {
            self.refresh_agent_row_vocabulary(index, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VocabEntry, agent_row_vocabulary_options, vocabulary_options};
    use daruda_store::agent_vocabulary::AgentVocabularyCache;

    fn entries(pairs: &[(&str, &str)]) -> Vec<VocabEntry> {
        pairs
            .iter()
            .map(|(id, name)| VocabEntry::new(*id, *name))
            .collect()
    }

    fn values(options: &[super::SelectOption]) -> Vec<String> {
        options.iter().map(|o| o.value.to_string()).collect()
    }

    /// The empty sentinel leads the list and is what `collect_agent_catalog`
    /// maps back to "no override", so it must never be a real mode id.
    #[test]
    fn the_agent_default_entry_comes_first_and_carries_an_empty_value() {
        let options = vocabulary_options(&entries(&[("plan", "Plan")]), Some("plan"), "");
        assert_eq!(values(&options), vec!["", "plan"]);
        assert!(
            options[0].label.contains("Plan"),
            "the entry names the adapter's own default: {}",
            options[0].label
        );
    }

    #[test]
    fn an_unknown_adapter_default_leaves_the_entry_unnamed() {
        let unlisted = vocabulary_options(&entries(&[("plan", "Plan")]), None, "");
        let named = vocabulary_options(&entries(&[("plan", "Plan")]), Some("plan"), "");
        assert_ne!(unlisted[0].label, named[0].label);
        assert_eq!(values(&unlisted), vec!["", "plan"]);
    }

    /// A value pinned before the agent ever advertised anything has to stay
    /// selectable, or opening Settings would drop it on the next save.
    #[test]
    fn a_saved_value_outside_the_vocabulary_is_appended() {
        let options = vocabulary_options(&entries(&[("plan", "Plan")]), None, "legacy");
        assert_eq!(values(&options), vec!["", "plan", "legacy"]);
    }

    #[test]
    fn a_saved_value_already_in_the_vocabulary_is_not_duplicated() {
        let options = vocabulary_options(&entries(&[("plan", "Plan")]), None, "plan");
        assert_eq!(values(&options), vec!["", "plan"]);
    }

    #[test]
    fn an_empty_vocabulary_still_offers_the_agent_default() {
        assert_eq!(values(&vocabulary_options(&[], None, "")), vec![""]);
    }

    #[test]
    fn a_known_empty_axis_does_not_fall_back_to_the_seed() {
        let mut cache = AgentVocabularyCache::default();
        cache.record_models(
            "claude",
            "npx -y @agentclientprotocol/claude-agent-acp@latest",
            Vec::new(),
        );

        let (_modes, models) = agent_row_vocabulary_options(
            &cache,
            "claude",
            "npx -y @agentclientprotocol/claude-agent-acp@latest",
            "",
            "",
        );

        assert_eq!(
            values(&models),
            vec![""],
            "the agent connected and advertised no models, so the Claude seed must stay suppressed"
        );
    }
}
