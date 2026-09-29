//! Where a catalog row's mode / model pickers get their options.
//!
//! Two sources, consulted **per axis independently**: what the agent last
//! advertised ([`AgentVocabularyCache`], keyed on the row's id and command) and the
//! build-time seed for the adapter the row's command names
//! ([`daruda_config::agent_vocabulary_seed`]). A live list on one axis must
//! not erase the seed on the other.
//!
//! Both keys are editable after the row was built, so the lists are rebuilt in
//! place by [`SettingsView::refresh_agent_row_vocabulary`] rather than at
//! render time — building them in `render` would mean mutating select state
//! mid-paint.

use crate::surface::strings as s;
use crate::ui::select::{SelectOption, SelectState};
use daruda_store::agent_vocabulary::{AgentVocabularyCache, VocabEntry};
use gpui::{Entity, SharedString, Window};

use super::super::{AgentCatalogItem, AgentCatalogRow, SettingsView};

impl SettingsView {
    /// Rebuild one row's mode/model option lists from the row's current id and
    /// command. Both are editable after the row was constructed, so the lists
    /// are re-sourced here rather than at render time (building them in
    /// `render` would mean mutating the select state mid-paint). The
    /// `SelectState` entities are reused, never replaced, so the row's
    /// subscriptions stay wired to them.
    pub(in crate::settings) fn refresh_agent_row_vocabulary(
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
    /// sentinel. The one reading both collect paths share, so neither can
    /// disagree about what "no override" looks like.
    pub(in crate::settings) fn default_mode(&self, cx: &gpui::App) -> Option<String> {
        selected_override(&self.default_mode_select, cx)
    }

    /// The pinned model, same sentinel as [`Self::default_mode`].
    pub(in crate::settings) fn default_model(&self, cx: &gpui::App) -> Option<String> {
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
pub(in crate::settings) fn agent_row_vocabulary_options(
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
            MODE_LABELS,
        ),
        vocabulary_options(
            known_axis(vocabulary.known_models_for(agent_id, command), seed_models),
            seed.as_ref().and_then(|seed| seed.default_model.as_deref()),
            saved_model,
            MODEL_LABELS,
        ),
    )
}

/// `(model, mode)` as a collapsed catalog card names them: the picked value,
/// or what the adapter itself defaults to when the row picks nothing — the
/// same names the pickers show, without their "agent default" framing.
pub(in crate::settings) fn agent_row_summary(
    vocabulary: &AgentVocabularyCache,
    agent_id: &str,
    command: &str,
    saved_mode: &str,
    saved_model: &str,
) -> (String, String) {
    let seed = daruda_config::agent_vocabulary_seed(command);
    let (seed_modes, seed_models) = match seed.as_ref() {
        Some(seed) => (seed.modes.as_slice(), seed.models.as_slice()),
        None => (&[][..], &[][..]),
    };
    (
        summary_label(
            known_axis(vocabulary.known_models_for(agent_id, command), seed_models),
            seed.as_ref().and_then(|seed| seed.default_model.as_deref()),
            saved_model,
            model_label,
        ),
        summary_label(
            known_axis(vocabulary.known_modes_for(agent_id, command), seed_modes),
            seed.as_ref().and_then(|seed| seed.default_mode.as_deref()),
            saved_mode,
            mode_label,
        ),
    )
}

/// One axis of [`agent_row_summary`].
fn summary_label(
    entries: &[VocabEntry],
    adapter_default: Option<&str>,
    saved: &str,
    label: fn(&VocabEntry) -> String,
) -> String {
    if !saved.is_empty() {
        return entry_label(entries, saved, label);
    }
    match adapter_default {
        Some(id) => entry_label(entries, id, label),
        None => s::settings_agent_vocabulary_agent_default(),
    }
}

/// The label `id`'s advertised entry shows, or the id itself when nothing
/// names it.
fn entry_label(entries: &[VocabEntry], id: &str, label: fn(&VocabEntry) -> String) -> String {
    entries
        .iter()
        .find(|entry| entry.id == id)
        .map_or_else(|| id.to_string(), label)
}

/// What the agent last advertised on one axis, or the adapter seed until it
/// has advertised anything there.
fn known_axis<'a>(cached: Option<&'a [VocabEntry]>, seeded: &'a [VocabEntry]) -> &'a [VocabEntry] {
    cached.unwrap_or(seeded)
}

/// How one axis names its choices: `choice` for each entry in the picker,
/// `adapter_default` for the entry the "agent default" option points at.
#[derive(Clone, Copy)]
struct AxisLabels {
    choice: fn(&VocabEntry) -> String,
    adapter_default: fn(&VocabEntry) -> String,
}

const MODE_LABELS: AxisLabels = AxisLabels {
    choice: mode_label,
    adapter_default: mode_label,
};

/// The agent-default option names the model the default resolves to, not
/// the default choice itself: "Agent default — Opus 5.5", not
/// "Agent default — Default (Opus 5.5)".
const MODEL_LABELS: AxisLabels = AxisLabels {
    choice: model_label,
    adapter_default: resolved_model_label,
};

fn mode_label(entry: &VocabEntry) -> String {
    entry.name.clone()
}

/// Same label the agent-chat model chip shows for this choice.
fn model_label(entry: &VocabEntry) -> String {
    s::agent_model_choice_label(&entry.id, &entry.name, entry.description.as_deref())
}

/// The concrete model a default-model choice resolves to, when advertised.
fn resolved_model_label(entry: &VocabEntry) -> String {
    daruda_acp::resolved_default_model(&entry.id, entry.description.as_deref())
        .map_or_else(|| model_label(entry), str::to_string)
}

/// The `(agent default[ — name])` entry first, then the vocabulary, then
/// `saved` if the vocabulary does not list it — so a value set before the
/// agent was ever connected is never silently dropped.
fn vocabulary_options(
    entries: &[VocabEntry],
    adapter_default: Option<&str>,
    saved: &str,
    labels: AxisLabels,
) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new(
        "",
        agent_default_label(entries, adapter_default, labels.adapter_default),
    )];
    options.extend(
        entries
            .iter()
            .map(|entry| SelectOption::new(entry.id.clone(), (labels.choice)(entry))),
    );
    if !saved.is_empty() && !entries.iter().any(|entry| entry.id == saved) {
        options.push(SelectOption::new(saved.to_string(), saved.to_string()));
    }
    options
}

/// Label for the "no override" entry. It names the adapter's own default when
/// daruda knows it, so picking it is a stated choice rather than a blank.
fn agent_default_label(
    entries: &[VocabEntry],
    adapter_default: Option<&str>,
    label: fn(&VocabEntry) -> String,
) -> String {
    let Some(id) = adapter_default else {
        return s::settings_agent_vocabulary_agent_default();
    };
    s::settings_agent_vocabulary_agent_default_named(&entry_label(entries, id, label))
}

#[cfg(test)]
impl SettingsView {
    /// Test-only — install a known vocabulary cache and re-source every row's
    /// pickers from it, so a test never depends on the developer's real
    /// `agent_vocabulary.json`.
    pub(in crate::settings) fn set_agent_vocabulary_for_test(
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
    use super::{VocabEntry, agent_row_summary, agent_row_vocabulary_options, vocabulary_options};
    use daruda_store::agent_vocabulary::AgentVocabularyCache;

    fn entries(pairs: &[(&str, &str)]) -> Vec<VocabEntry> {
        pairs
            .iter()
            .map(|(id, name)| VocabEntry::new(*id, *name))
            .collect()
    }

    const CLAUDE: &str = "npx -y @agentclientprotocol/claude-agent-acp@latest";

    fn values(options: &[super::SelectOption]) -> Vec<String> {
        options.iter().map(|o| o.value.to_string()).collect()
    }

    /// The empty sentinel leads the list and is what `collect_agent_catalog`
    /// maps back to "no override", so it must never be a real mode id.
    #[test]
    fn the_agent_default_entry_comes_first_and_carries_an_empty_value() {
        let options = vocabulary_options(
            &entries(&[("plan", "Plan")]),
            Some("plan"),
            "",
            super::MODE_LABELS,
        );
        assert_eq!(values(&options), vec!["", "plan"]);
        assert!(
            options[0].label.contains("Plan"),
            "the entry names the adapter's own default: {}",
            options[0].label
        );
    }

    #[test]
    fn an_unknown_adapter_default_leaves_the_entry_unnamed() {
        let unlisted =
            vocabulary_options(&entries(&[("plan", "Plan")]), None, "", super::MODE_LABELS);
        let named = vocabulary_options(
            &entries(&[("plan", "Plan")]),
            Some("plan"),
            "",
            super::MODE_LABELS,
        );
        assert_ne!(unlisted[0].label, named[0].label);
        assert_eq!(values(&unlisted), vec!["", "plan"]);
    }

    /// A value pinned before the agent ever advertised anything has to stay
    /// selectable, or opening Settings would drop it on the next save.
    #[test]
    fn a_saved_value_outside_the_vocabulary_is_appended() {
        let options = vocabulary_options(
            &entries(&[("plan", "Plan")]),
            None,
            "legacy",
            super::MODE_LABELS,
        );
        assert_eq!(values(&options), vec!["", "plan", "legacy"]);
    }

    #[test]
    fn a_saved_value_already_in_the_vocabulary_is_not_duplicated() {
        let options = vocabulary_options(
            &entries(&[("plan", "Plan")]),
            None,
            "plan",
            super::MODE_LABELS,
        );
        assert_eq!(values(&options), vec!["", "plan"]);
    }

    #[test]
    fn an_empty_vocabulary_still_offers_the_agent_default() {
        assert_eq!(
            values(&vocabulary_options(&[], None, "", super::MODE_LABELS)),
            vec![""]
        );
    }

    /// The default model names what it resolves to, the same label the
    /// agent-chat chip shows.
    #[test]
    fn the_default_model_names_the_model_it_resolves_to() {
        let mut cache = AgentVocabularyCache::default();
        cache.record_models(
            "claude",
            CLAUDE,
            vec![
                VocabEntry::new("default", "Default (recommended)")
                    .with_description(Some("Opus 5.5".to_string())),
                VocabEntry::new("opus", "Opus 5.5")
                    .with_description(Some("For complex work".to_string())),
            ],
        );

        let (_modes, models) = agent_row_vocabulary_options(&cache, "claude", CLAUDE, "", "");

        let labels: Vec<String> = models.iter().map(|o| o.label.to_string()).collect();
        assert_eq!(labels[1..], ["Default (Opus 5.5)", "Opus 5.5"]);
        assert_eq!(
            labels[0],
            crate::surface::strings::settings_agent_vocabulary_agent_default_named("Opus 5.5"),
            "the agent-default entry names the model, not the default choice"
        );
    }

    /// A collapsed card names the picked value, or — picking nothing — what
    /// the adapter itself defaults to, from the same vocabulary the pickers use.
    #[test]
    fn the_summary_names_the_pick_or_the_adapter_default() {
        let command = "npx -y @agentclientprotocol/claude-agent-acp@latest";
        let mut cache = AgentVocabularyCache::default();
        cache.record_models(
            "claude",
            command,
            entries(&[("default", "Default (recommended)"), ("opus", "Opus 5.5")]),
        );
        cache.record_modes(
            "claude",
            command,
            entries(&[("default", "Manual"), ("plan", "Plan")]),
        );

        assert_eq!(
            agent_row_summary(&cache, "claude", command, "", ""),
            ("Default (recommended)".to_string(), "Manual".to_string())
        );
        // The model axis names it the way the chat's model chip does.
        cache.record_models(
            "claude",
            command,
            vec![
                VocabEntry::new("default", "Default (recommended)")
                    .with_description(Some("Opus 5.5".to_string())),
                VocabEntry::new("opus", "Opus 5.5")
                    .with_description(Some("For complex work".to_string())),
            ],
        );
        assert_eq!(
            agent_row_summary(&cache, "claude", command, "", "").0,
            "Default (Opus 5.5)"
        );
        assert_eq!(
            agent_row_summary(&cache, "claude", command, "plan", "opus"),
            ("Opus 5.5".to_string(), "Plan".to_string())
        );
        assert_eq!(
            agent_row_summary(&cache, "claude", command, "plan", "default").0,
            "Default (Opus 5.5)"
        );
        // A pick the vocabulary does not list still names itself.
        assert_eq!(
            agent_row_summary(&cache, "claude", command, "legacy", "").1,
            "legacy"
        );
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
