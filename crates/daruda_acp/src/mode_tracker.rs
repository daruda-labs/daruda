//! Session-mode ownership for a live ACP connection.
//!
//! The protocol reports the agent's session mode on two channels: a
//! `CurrentModeUpdate` notification carrying only the new mode **id**, and the
//! `Mode`-category entry of the full config-option set (`ConfigOptionUpdate`
//! notifications and `set_config_option` replies), which carries the whole
//! advertised list. Neither is complete on its own — the id-only channel can't
//! report a list the agent rebuilt per model, and the option channel is not the
//! one the agent uses to announce a plain mode switch.
//!
//! [`ModeTracker`] folds both into the single [`ModeStateView`] it owns, so the
//! host receives one reconciled mode fact ([`crate::session::AcpEvent::ModeChanged`])
//! and never has to reconcile two mirrors itself. The `Mode` config option is
//! stripped on the way out for the same reason: mode reaches the host as mode,
//! not as a config option the host would have to recognize.

use std::sync::{Arc, Mutex};

use crate::model::{ConfigOptionCategoryView, ConfigOptionView, ModeStateView};

/// The result of folding a fresh config-option set into the tracker.
pub(crate) struct ConfigOptionsFold {
    /// `Some` only when the mode state actually changed — the caller emits a
    /// `ModeChanged` for it and stays silent otherwise, so an option change
    /// that leaves mode alone (a model or effort pick) causes no mode traffic.
    pub(crate) mode: Option<ModeStateView>,
    /// The option set with the `Mode` entry removed.
    pub(crate) options: Vec<ConfigOptionView>,
}

/// Owns the current [`ModeStateView`] for one connection.
///
/// Cheap to clone (shared handle) because the notification handler and the
/// connection task are separate closures built before the session exists — the
/// same reason [`crate::session`]'s permission parks are shared this way.
///
/// A tracker is **inert** until [`ModeTracker::seed`] gives it a mode state, and
/// permanently inert when seeded with `None`. That keeps "does this agent have
/// modes at all" a connect-time decision: an agent that advertised no
/// `SessionModeState` never grows mode affordances mid-session, and a
/// notification replayed during `session/load` (which arrives before the load
/// response seeds the authoritative state) can't race ahead of it.
#[derive(Clone, Default)]
pub(crate) struct ModeTracker {
    state: Arc<Mutex<Option<ModeStateView>>>,
}

impl ModeTracker {
    /// Install the connect-time mode state — the `session/new` / `session/load`
    /// response's `SessionModeState`, after any configured initial mode has been
    /// applied to it. `None` marks the agent as having no modes.
    pub(crate) fn seed(&self, modes: Option<ModeStateView>) {
        *self.lock() = modes;
    }

    /// Fold a `CurrentModeUpdate`'s mode id in. Returns the new state only when
    /// it changed.
    ///
    /// The id is taken at face value even when it is absent from the advertised
    /// list: the agent is authoritative about the mode it is in, and refusing an
    /// unrecognized id is what let a rebuilt-list session drift out of sync. The
    /// matching list follows on the option channel.
    pub(crate) fn apply_current_mode(&self, mode_id: String) -> Option<ModeStateView> {
        let mut guard = self.lock();
        let state = guard.as_mut()?;
        if state.current == mode_id {
            return None;
        }
        state.current = mode_id;
        Some(state.clone())
    }

    /// Fold a full config-option set in: update the mode state from its `Mode`
    /// entry and strip that entry from the set handed to the host.
    pub(crate) fn fold_config_options(&self, options: Vec<ConfigOptionView>) -> ConfigOptionsFold {
        let mut guard = self.lock();
        let mode = match (guard.as_mut(), ModeStateView::from_config_options(&options)) {
            (Some(state), Some(fresh)) if *state != fresh => {
                *state = fresh.clone();
                Some(fresh)
            }
            _ => None,
        };
        drop(guard);
        ConfigOptionsFold {
            mode,
            options: strip_mode_options(options),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<ModeStateView>> {
        self.state.lock().expect("mode tracker mutex poisoned")
    }
}

/// Drop every `Mode`-category option. Applied to every option set that reaches
/// the host, including the connect-time one, so the host has exactly one
/// representation of mode.
pub(crate) fn strip_mode_options(options: Vec<ConfigOptionView>) -> Vec<ConfigOptionView> {
    options
        .into_iter()
        .filter(|o| o.category != ConfigOptionCategoryView::Mode)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConfigChoiceView, SessionModeView};

    fn choice(id: &str) -> ConfigChoiceView {
        ConfigChoiceView {
            value: id.to_string(),
            name: id.to_uppercase(),
            description: None,
        }
    }

    fn mode_option(current: &str, ids: &[&str]) -> ConfigOptionView {
        ConfigOptionView {
            id: "mode".to_string(),
            name: "Mode".to_string(),
            description: None,
            category: ConfigOptionCategoryView::Mode,
            current_value: current.to_string(),
            options: ids.iter().map(|id| choice(id)).collect(),
        }
    }

    fn model_option() -> ConfigOptionView {
        ConfigOptionView {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: None,
            category: ConfigOptionCategoryView::Model,
            current_value: "sonnet".to_string(),
            options: vec![choice("sonnet")],
        }
    }

    fn seeded(current: &str, ids: &[&str]) -> ModeTracker {
        let tracker = ModeTracker::default();
        tracker.seed(Some(ModeStateView {
            available: ids
                .iter()
                .map(|id| SessionModeView {
                    id: id.to_string(),
                    name: id.to_uppercase(),
                    description: None,
                })
                .collect(),
            current: current.to_string(),
        }));
        tracker
    }

    #[test]
    fn unseeded_tracker_is_inert() {
        let tracker = ModeTracker::default();
        assert_eq!(tracker.apply_current_mode("plan".to_string()), None);
        let fold = tracker.fold_config_options(vec![mode_option("plan", &["plan", "default"])]);
        assert_eq!(fold.mode, None, "a replayed update can't seed the tracker");
        assert!(fold.options.is_empty(), "the Mode entry is still stripped");
    }

    #[test]
    fn a_modeless_agent_stays_modeless() {
        let tracker = ModeTracker::default();
        tracker.seed(None);
        assert_eq!(tracker.apply_current_mode("plan".to_string()), None);
        assert_eq!(
            tracker
                .fold_config_options(vec![mode_option("plan", &["plan"])])
                .mode,
            None,
            "mode support stays a connect-time decision"
        );
    }

    #[test]
    fn current_mode_update_reports_only_real_changes() {
        let tracker = seeded("default", &["default", "plan"]);

        assert_eq!(
            tracker.apply_current_mode("default".to_string()),
            None,
            "re-reporting the mode already held is not a change"
        );
        let changed = tracker
            .apply_current_mode("plan".to_string())
            .expect("switching mode is a change");
        assert_eq!(changed.current, "plan");
        assert_eq!(
            changed.available.len(),
            2,
            "the id-only channel leaves the advertised list intact"
        );
    }

    #[test]
    fn an_unadvertised_mode_id_is_still_believed() {
        // The agent is authoritative about the mode it is in; the list catches
        // up on the option channel. Dropping the id here is the drift this
        // tracker exists to prevent.
        let tracker = seeded("default", &["default", "plan"]);
        let changed = tracker
            .apply_current_mode("auto".to_string())
            .expect("an unknown id is applied, not dropped");
        assert_eq!(changed.current, "auto");
    }

    #[test]
    fn config_options_refresh_the_list_and_strip_the_mode_entry() {
        let tracker = seeded("default", &["default", "plan"]);

        let fold = tracker.fold_config_options(vec![
            mode_option("auto", &["auto", "default", "plan"]),
            model_option(),
        ]);

        let mode = fold.mode.expect("the rebuilt list is a change");
        assert_eq!(mode.current, "auto");
        assert_eq!(
            mode.available
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["auto", "default", "plan"]
        );
        assert_eq!(
            fold.options
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            ["model"],
            "the host never sees mode as a config option"
        );
    }

    #[test]
    fn an_unchanged_mode_option_emits_nothing() {
        let tracker = seeded("default", &["default", "plan"]);
        // A model pick re-sends the whole set; mode rides along unchanged.
        let fold = tracker.fold_config_options(vec![
            mode_option("default", &["default", "plan"]),
            model_option(),
        ]);
        assert_eq!(fold.mode, None, "no mode traffic for a model-only change");
        assert_eq!(fold.options.len(), 1);
    }

    #[test]
    fn an_option_set_without_a_mode_entry_leaves_the_state_alone() {
        let tracker = seeded("plan", &["default", "plan"]);
        let fold = tracker.fold_config_options(vec![model_option()]);
        assert_eq!(fold.mode, None);
        assert_eq!(
            tracker.apply_current_mode("default".to_string()),
            Some(ModeStateView {
                available: vec![
                    SessionModeView {
                        id: "default".to_string(),
                        name: "DEFAULT".to_string(),
                        description: None,
                    },
                    SessionModeView {
                        id: "plan".to_string(),
                        name: "PLAN".to_string(),
                        description: None,
                    },
                ],
                current: "default".to_string(),
            }),
            "the seeded list survived an unrelated option change"
        );
    }

    #[test]
    fn an_empty_mode_option_keeps_the_last_known_good_list() {
        let tracker = seeded("plan", &["default", "plan"]);
        let fold = tracker.fold_config_options(vec![mode_option("plan", &[])]);
        assert_eq!(fold.mode, None);
        assert_eq!(
            tracker
                .apply_current_mode("default".to_string())
                .map(|s| s.available.len()),
            Some(2),
            "an empty selector is treated as an adapter transient"
        );
    }

    #[test]
    fn strip_mode_options_leaves_other_categories() {
        let stripped = strip_mode_options(vec![mode_option("plan", &["plan"]), model_option()]);
        assert_eq!(stripped.len(), 1);
        assert_eq!(stripped[0].category, ConfigOptionCategoryView::Model);
    }
}
