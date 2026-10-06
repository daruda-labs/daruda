//! Which built-in presets the catalog offers beside the entries it already
//! has, and the rule that keeps one agent switched on. GPUI-free.

use daruda_config::{AgentPreset, PresetLaunchability};

/// The presets no entry references, split by whether daruda can run them.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PresetGroups {
    /// Runnable as-is: switching one on adds an entry for it.
    pub(super) available: Vec<AgentPreset>,
    /// Ships binaries daruda cannot launch: offered as an install page.
    pub(super) needs_install: Vec<AgentPreset>,
}

impl PresetGroups {
    fn is_empty(&self) -> bool {
        self.available.is_empty() && self.needs_install.is_empty()
    }
}

/// `presets` minus the ones `used` names, in their own order, narrowed to
/// those whose name, id or description contains `query` (case-insensitive;
/// empty keeps all). A preset an entry already references lives on that
/// entry's card, so offering it again would add a second copy of an agent.
pub(super) fn preset_groups(
    presets: impl IntoIterator<Item = AgentPreset>,
    used: &[&str],
    query: &str,
) -> PresetGroups {
    let query = query.trim().to_lowercase();
    let matches = |preset: &AgentPreset| {
        query.is_empty()
            || preset.name.to_lowercase().contains(&query)
            || preset.id.to_lowercase().contains(&query)
            || preset.description.to_lowercase().contains(&query)
    };
    let mut groups = PresetGroups::default();
    for preset in presets {
        if used.contains(&preset.id) || !matches(&preset) {
            continue;
        }
        match preset.launchability {
            PresetLaunchability::Runnable { .. } => groups.available.push(preset),
            PresetLaunchability::NeedsManualInstall { .. } => groups.needs_install.push(preset),
        }
    }
    groups
}

/// Whether `query` is what emptied both preset lists — not a catalog that
/// already uses every preset, where there is nothing left to match.
pub(super) fn query_matched_nothing(used: &[&str], query: &str) -> bool {
    !query.trim().is_empty()
        && preset_groups(daruda_config::agent_presets(), used, query).is_empty()
        && !preset_groups(daruda_config::agent_presets(), used, "").is_empty()
}

/// Whether switching `index` off would leave no agent on. Settings keeps one
/// on, so the catalog a new chat opens with is never empty.
pub(super) fn is_last_enabled(enabled: &[bool], index: usize) -> bool {
    enabled.get(index).copied().unwrap_or(false) && enabled.iter().filter(|on| **on).count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(presets: &[AgentPreset]) -> Vec<&'static str> {
        presets.iter().map(|preset| preset.id).collect()
    }

    #[test]
    fn every_preset_lands_in_exactly_one_place() {
        let used = ["codex-acp"];
        let groups = preset_groups(daruda_config::agent_presets(), &used, "");
        for preset in daruda_config::agent_presets() {
            let places = usize::from(used.contains(&preset.id))
                + usize::from(ids(&groups.available).contains(&preset.id))
                + usize::from(ids(&groups.needs_install).contains(&preset.id));
            assert_eq!(places, 1, "{} appears in {places} places", preset.id);
        }
        assert!(!groups.available.is_empty() && !groups.needs_install.is_empty());
    }

    #[test]
    fn a_manual_install_preset_is_never_offered_as_available() {
        let groups = preset_groups(daruda_config::agent_presets(), &[], "cursor");
        assert_eq!(ids(&groups.needs_install), ["cursor"]);
        assert!(groups.available.is_empty());
    }

    #[test]
    fn the_query_matches_name_id_or_description_ignoring_case() {
        let by_name = preset_groups(daruda_config::agent_presets(), &[], "GEMINI");
        assert_eq!(ids(&by_name.available), ["gemini"]);
        let by_id = preset_groups(daruda_config::agent_presets(), &[], "codex-a");
        assert_eq!(ids(&by_id.available), ["codex-acp"]);
        // "Tencent" is only in Codebuddy's description.
        let by_description = preset_groups(daruda_config::agent_presets(), &[], "tencent");
        assert_eq!(ids(&by_description.available), ["codebuddy-code"]);
        let none = preset_groups(daruda_config::agent_presets(), &[], "no-such-agent");
        assert_eq!(none, PresetGroups::default());
    }

    #[test]
    fn only_a_query_that_rejects_every_unused_preset_matched_nothing() {
        assert!(query_matched_nothing(&[], "no-such-agent"));
        assert!(!query_matched_nothing(&[], "gemini"), "it matches one");
        assert!(
            !query_matched_nothing(&[], "  "),
            "a blank query filters nothing"
        );
        let every: Vec<&str> = daruda_config::agent_presets().map(|p| p.id).collect();
        assert!(
            !query_matched_nothing(&every, "no-such-agent"),
            "with every preset in use there was nothing left to match"
        );
    }

    #[test]
    fn only_the_sole_enabled_entry_is_the_last_one() {
        assert!(is_last_enabled(&[false, true, false], 1));
        assert!(!is_last_enabled(&[true, true], 0));
        assert!(!is_last_enabled(&[false, true], 0), "already off");
        assert!(!is_last_enabled(&[true], 3), "out of range");
    }
}
