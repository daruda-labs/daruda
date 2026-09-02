//! Fold state with explicit user overrides over pane defaults.

// Item-index keys require append-only item order. Clear FoldState before
// removing or reordering items.

use std::collections::HashMap;

use super::pane_choice::PaneChoice;
use crate::transcript::fold_mode::{BlockRule, FoldBlock, FoldMode, TurnPosition};
use crate::transcript::tool_category::ToolCategory;

/// Stable identity of a foldable block.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(in crate::workspace) enum FoldKey {
    Assistant(usize),
    Thinking(usize),
    Tool(String),
    Diff(String),
    ToolRawInput(String),
    Subagent(String),
    ToolGroup(String),
    ThinkingGroup(usize),
    Response(usize),
    Tail(usize),
    Filtered(usize),
}

enum FoldPolicy {
    DefaultExpanded,
    ExpandedWhileActive,
    DefaultCollapsed,
}

impl FoldKey {
    /// The mode-controlled block, if this key is not owned by another chip.
    fn block(&self) -> Option<FoldBlock> {
        match self {
            FoldKey::Assistant(_) => Some(FoldBlock::Assistant),
            FoldKey::Thinking(_) => Some(FoldBlock::Thinking),
            FoldKey::Tool(_) => Some(FoldBlock::Tool),
            FoldKey::Diff(_) => Some(FoldBlock::Diff),
            FoldKey::ToolRawInput(_) => Some(FoldBlock::RawInput),
            FoldKey::Subagent(_) => Some(FoldBlock::Subagent),
            FoldKey::ToolGroup(_) => Some(FoldBlock::ToolGroup),
            FoldKey::ThinkingGroup(_) => Some(FoldBlock::ThinkingGroup),
            FoldKey::Response(_) => Some(FoldBlock::Response),
            FoldKey::Tail(_) | FoldKey::Filtered(_) => None,
        }
    }

    fn policy(&self) -> FoldPolicy {
        match self {
            // Assistant prose and file-edit diffs stay visible by default.
            FoldKey::Assistant(_) | FoldKey::Diff(_) => FoldPolicy::DefaultExpanded,
            FoldKey::Thinking(_)
            | FoldKey::Tool(_)
            | FoldKey::ToolGroup(_)
            | FoldKey::ThinkingGroup(_)
            | FoldKey::Response(_) => FoldPolicy::ExpandedWhileActive,
            // Chip-owned bulk remains collapsed until explicitly revealed.
            FoldKey::ToolRawInput(_)
            | FoldKey::Subagent(_)
            | FoldKey::Tail(_)
            | FoldKey::Filtered(_) => FoldPolicy::DefaultCollapsed,
        }
    }
}

fn natural_default(policy: FoldPolicy, active: bool) -> bool {
    match policy {
        FoldPolicy::DefaultExpanded => true,
        FoldPolicy::ExpandedWhileActive => active,
        FoldPolicy::DefaultCollapsed => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct FoldContext {
    active: bool,
    position: TurnPosition,
    tool_category: Option<ToolCategory>,
}

impl FoldContext {
    #[cfg(test)]
    pub(in crate::workspace) fn past(active: bool) -> Self {
        Self {
            active,
            position: TurnPosition::Past,
            tool_category: None,
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn last(active: bool) -> Self {
        Self {
            active,
            position: TurnPosition::Last,
            tool_category: None,
        }
    }

    pub(in crate::workspace) fn new(position: TurnPosition, active: bool) -> Self {
        Self {
            active,
            position,
            tool_category: None,
        }
    }

    pub(in crate::workspace) fn with_tool_category(mut self, category: ToolCategory) -> Self {
        self.tool_category = Some(category);
        self
    }
}

/// Fold state for one conversation: the pane's mode plus explicit user choices.
#[derive(Default)]
pub(in crate::workspace) struct FoldState {
    /// Present = an explicit user choice; absent = derive the natural default.
    /// Nothing but a user gesture writes here.
    overrides: HashMap<FoldKey, bool>,
    /// The one response a prompt send froze open so the prose being read is not
    /// yanked away. Machine-written, so it stays out of `overrides` and holds
    /// only the newest run start — see [`Self::hold_response`].
    held_response: Option<usize>,
    mode: PaneChoice<FoldMode>,
}

impl FoldState {
    pub(in crate::workspace) fn with_mode(mode: FoldMode) -> Self {
        Self {
            overrides: HashMap::new(),
            held_response: None,
            mode: PaneChoice::Seeded(mode),
        }
    }

    pub(in crate::workspace) fn mode(&self) -> FoldMode {
        self.mode.value()
    }

    pub(in crate::workspace) fn chosen_mode(&self) -> Option<FoldMode> {
        self.mode.chosen()
    }

    /// The mode together with whether it is still config's. The Activity Bar
    /// reads both — the label from one, the overridden mark from the other.
    pub(in crate::workspace) fn mode_choice(&self) -> PaneChoice<FoldMode> {
        self.mode
    }

    /// Follow a reloaded config default. A mode the user picked is untouched.
    pub(in crate::workspace) fn reseed_mode(&mut self, mode: FoldMode) {
        self.mode.reseed(mode);
    }

    pub(in crate::workspace) fn set_mode(&mut self, mode: FoldMode) {
        self.decide_mode(PaneChoice::Chosen(mode));
    }

    /// Drop the pane's own mode and follow the configured default again.
    pub(in crate::workspace) fn reset_mode(&mut self, mode: FoldMode) {
        self.decide_mode(PaneChoice::Seeded(mode));
    }

    fn decide_mode(&mut self, mode: PaneChoice<FoldMode>) {
        self.mode = mode;
        // Deciding the mode — picking one or handing the axis back to config —
        // is a statement about the whole transcript, so it supersedes the
        // transient send-time hold. User overrides survive it.
        self.held_response = None;
    }

    /// Freeze the response starting at `run_start` open until the next send, or
    /// release the hold with `None`. Ranks below a user override and above the
    /// mode default, and replaces any previous hold — only the newest response
    /// is ever held.
    pub(in crate::workspace) fn hold_response(&mut self, run_start: Option<usize>) {
        self.held_response = run_start;
    }

    fn policy_for(&self, key: &FoldKey, ctx: FoldContext) -> FoldPolicy {
        let mode = self.mode.value();
        let category_rule = match (key, ctx.tool_category) {
            (FoldKey::Tool(_), Some(category)) => mode.tool_rule(ctx.position, category),
            _ => BlockRule::Builtin,
        };
        let rule = if category_rule != BlockRule::Builtin {
            category_rule
        } else {
            key.block()
                .map(|block| mode.rule(ctx.position, block))
                .unwrap_or(BlockRule::Builtin)
        };
        match rule {
            BlockRule::Expanded => FoldPolicy::DefaultExpanded,
            BlockRule::Collapsed => FoldPolicy::DefaultCollapsed,
            BlockRule::Builtin => key.policy(),
        }
    }

    pub(in crate::workspace) fn is_expanded(&self, key: &FoldKey, ctx: FoldContext) -> bool {
        if let Some(expanded) = self.overrides.get(key) {
            return *expanded;
        }
        if matches!(key, FoldKey::Response(run_start) if self.held_response == Some(*run_start)) {
            return true;
        }
        natural_default(self.policy_for(key, ctx), ctx.active)
    }

    pub(in crate::workspace) fn toggle(&mut self, key: FoldKey, ctx: FoldContext) {
        let cur = self.is_expanded(&key, ctx);
        self.overrides.insert(key, !cur);
    }

    pub(in crate::workspace) fn clear_overrides(&mut self) {
        self.overrides.clear();
        // The anchor is an item index, invalid once the conversation is dropped.
        self.held_response = None;
    }

    pub(in crate::workspace) fn clear_tail_reveals(&mut self) -> bool {
        self.clear_matching_overrides(|key| matches!(key, FoldKey::Tail(_)))
    }

    pub(in crate::workspace) fn clear_filter_reveals(&mut self) -> bool {
        self.clear_matching_overrides(|key| matches!(key, FoldKey::Filtered(_)))
    }

    fn clear_matching_overrides(&mut self, matches: impl Fn(&FoldKey) -> bool) -> bool {
        let old_len = self.overrides.len();
        self.overrides.retain(|key, _| !matches(key));
        self.overrides.len() != old_len
    }

    pub(in crate::workspace) fn set_all(
        &mut self,
        keys: impl IntoIterator<Item = FoldKey>,
        expanded: bool,
    ) {
        for key in keys {
            self.overrides.insert(key, expanded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_is_always_expanded_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Assistant(0), FoldContext::past(true)));
        assert!(state.is_expanded(&FoldKey::Assistant(0), FoldContext::past(false)));
    }

    #[test]
    fn thinking_tracks_active_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Thinking(0), FoldContext::past(true)));
        assert!(!state.is_expanded(&FoldKey::Thinking(0), FoldContext::past(false)));
    }

    #[test]
    fn tool_tracks_active_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Tool("call-1".into()), FoldContext::past(true)));
        assert!(!state.is_expanded(&FoldKey::Tool("call-1".into()), FoldContext::past(false)));
    }

    #[test]
    fn a_group_tracks_active_by_default() {
        let state = FoldState::default();
        // Both group kinds share the `ExpandedWhileActive` arm, so both are
        // asserted rather than one standing in for the other.
        for key in [FoldKey::ToolGroup("t".into()), FoldKey::ThinkingGroup(5)] {
            assert!(state.is_expanded(&key, FoldContext::past(true)), "{key:?}");
            assert!(
                !state.is_expanded(&key, FoldContext::past(false)),
                "{key:?}"
            );
        }
    }

    #[test]
    fn tail_keeps_the_covered_steps_folded_even_while_active() {
        let state = FoldState::default();
        assert!(!state.is_expanded(&FoldKey::Tail(0), FoldContext::past(true)));
        assert!(!state.is_expanded(&FoldKey::Tail(0), FoldContext::past(false)));
    }

    #[test]
    fn the_filter_keeps_what_it_hides_folded_even_while_active() {
        let state = FoldState::default();
        assert!(!state.is_expanded(&FoldKey::Filtered(0), FoldContext::past(true)));
        assert!(!state.is_expanded(&FoldKey::Filtered(0), FoldContext::past(false)));
    }

    #[test]
    fn diff_is_expanded_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Diff("call-1#0".into()), FoldContext::past(true)));
        assert!(state.is_expanded(&FoldKey::Diff("call-1#0".into()), FoldContext::past(false)));
    }

    #[test]
    fn tool_raw_input_is_collapsed_by_default() {
        let state = FoldState::default();
        assert!(!state.is_expanded(
            &FoldKey::ToolRawInput("call-1".into()),
            FoldContext::past(true)
        ));
        assert!(!state.is_expanded(
            &FoldKey::ToolRawInput("call-1".into()),
            FoldContext::past(false)
        ));
    }

    #[test]
    fn subagent_is_collapsed_by_default_even_while_active() {
        let state = FoldState::default();
        let key = FoldKey::Subagent("task-1".into());
        assert!(!state.is_expanded(&key, FoldContext::past(true)));
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn subagent_override_expands_and_sticks_across_active() {
        let mut state = FoldState::default();
        let key = FoldKey::Subagent("task-1".into());
        state.toggle(key.clone(), FoldContext::past(false));
        assert!(state.is_expanded(&key, FoldContext::past(false)));
        assert!(state.is_expanded(&key, FoldContext::past(true)));
    }

    #[test]
    fn natural_default_matrix() {
        assert!(natural_default(FoldPolicy::DefaultExpanded, true));
        assert!(natural_default(FoldPolicy::DefaultExpanded, false));
        assert!(natural_default(FoldPolicy::ExpandedWhileActive, true));
        assert!(!natural_default(FoldPolicy::ExpandedWhileActive, false));
        assert!(!natural_default(FoldPolicy::DefaultCollapsed, true));
        assert!(!natural_default(FoldPolicy::DefaultCollapsed, false));
    }

    #[test]
    fn untouched_block_auto_collapses_when_done() {
        let state = FoldState::default();
        let key = FoldKey::Tool("call-1".into());
        assert!(state.is_expanded(&key, FoldContext::past(true)));
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn override_via_set_all_beats_active() {
        let mut state = FoldState::default();
        let key = FoldKey::Thinking(3);
        state.set_all([key.clone()], false);
        assert!(!state.is_expanded(&key, FoldContext::past(true)));
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn override_expanded_beats_collapsed_default() {
        let mut state = FoldState::default();
        let key = FoldKey::ToolRawInput("call-1".into());
        state.set_all([key.clone()], true);
        assert!(state.is_expanded(&key, FoldContext::past(true)));
        assert!(state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn user_override_persists_across_active_changes() {
        let mut state = FoldState::default();
        let key = FoldKey::Tool("call-1".into());
        state.toggle(key.clone(), FoldContext::past(true));
        assert!(!state.is_expanded(&key, FoldContext::past(true)));
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn toggle_never_touched_raw_input_flips_to_expanded_then_back() {
        let mut state = FoldState::default();
        let key = FoldKey::ToolRawInput("call-1".into());
        state.toggle(key.clone(), FoldContext::past(false));
        assert!(state.is_expanded(&key, FoldContext::past(false)));
        state.toggle(key.clone(), FoldContext::past(false));
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn toggle_never_touched_assistant_flips_to_collapsed() {
        let mut state = FoldState::default();
        let key = FoldKey::Assistant(0);
        state.toggle(key.clone(), FoldContext::past(false));
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn toggle_respects_active_for_effective_default() {
        let mut state = FoldState::default();
        let key = FoldKey::Tool("call-1".into());
        state.toggle(key.clone(), FoldContext::past(true));
        assert!(!state.is_expanded(&key, FoldContext::past(true)));

        let mut state2 = FoldState::default();
        let key2 = FoldKey::Tool("call-2".into());
        state2.toggle(key2.clone(), FoldContext::past(false));
        assert!(state2.is_expanded(&key2, FoldContext::past(false)));
    }

    #[test]
    fn set_all_affects_only_given_keys() {
        let mut state = FoldState::default();
        let touched = FoldKey::ToolRawInput("call-1".into());
        let untouched = FoldKey::ToolRawInput("call-2".into());
        state.set_all([touched.clone()], true);

        assert!(state.is_expanded(&touched, FoldContext::past(false)));
        assert!(!state.is_expanded(&untouched, FoldContext::past(false)));
    }

    use crate::transcript::fold_mode::FoldPreset;

    fn every_key() -> Vec<FoldKey> {
        vec![
            FoldKey::Assistant(0),
            FoldKey::Thinking(1),
            FoldKey::Tool("t".into()),
            FoldKey::Diff("t#0".into()),
            FoldKey::ToolRawInput("t".into()),
            FoldKey::Subagent("s".into()),
            FoldKey::ToolGroup("t".into()),
            FoldKey::ThinkingGroup(5),
            FoldKey::Response(3),
            FoldKey::Tail(4),
            FoldKey::Filtered(4),
        ]
    }

    #[test]
    fn auto_is_the_default_and_only_pins_the_newest_response() {
        let state = FoldState::default();
        for key in every_key() {
            for active in [true, false] {
                let builtin = natural_default(key.policy(), active);
                assert_eq!(
                    state.is_expanded(&key, FoldContext::past(active)),
                    builtin,
                    "past {key:?} active={active}"
                );
                let expected_last = matches!(key, FoldKey::Response(_)) || builtin;
                assert_eq!(
                    state.is_expanded(&key, FoldContext::last(active)),
                    expected_last,
                    "last {key:?} active={active}"
                );
            }
        }
    }

    #[test]
    fn summary_treats_the_newest_turn_like_history() {
        let state = FoldState::with_mode(FoldPreset::Summary.mode());
        let key = FoldKey::Response(3);
        assert!(!state.is_expanded(&key, FoldContext::last(false)));
        assert!(state.is_expanded(&key, FoldContext::last(true)));
        for key in every_key() {
            for active in [true, false] {
                let builtin = natural_default(key.policy(), active);
                assert_eq!(
                    state.is_expanded(&key, FoldContext::last(active)),
                    builtin,
                    "{key:?} active={active}"
                );
            }
        }
    }

    #[test]
    fn expanded_opens_past_responses_and_the_newest_settled_groups() {
        let state = FoldState::with_mode(FoldPreset::Expanded.mode());
        assert!(state.is_expanded(&FoldKey::Response(0), FoldContext::past(false)));
        for key in [FoldKey::ToolGroup("t".into()), FoldKey::ThinkingGroup(2)] {
            assert!(state.is_expanded(&key, FoldContext::last(false)), "{key:?}");
            assert!(
                !state.is_expanded(&key, FoldContext::past(false)),
                "{key:?}"
            );
        }
    }

    #[test]
    fn the_two_turn_axes_move_independently() {
        let state = FoldState::default();
        let key = FoldKey::Response(3);
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
        assert!(state.is_expanded(&key, FoldContext::last(false)));
    }

    #[test]
    fn reseeding_moves_an_unchosen_mode_only() {
        let mut state = FoldState::with_mode(FoldPreset::Auto.mode());
        state.reseed_mode(FoldPreset::Summary.mode());
        assert_eq!(state.mode(), FoldPreset::Summary.mode());
        assert_eq!(state.chosen_mode(), None, "a reseed is not a choice");

        state.set_mode(FoldPreset::Auto.mode());
        state.reseed_mode(FoldPreset::Expanded.mode());
        assert_eq!(
            state.mode(),
            FoldPreset::Auto.mode(),
            "config must not overwrite the user's pick"
        );
    }

    #[test]
    fn a_user_override_outranks_the_mode() {
        let mut state = FoldState::with_mode(FoldPreset::Expanded.mode());
        let key = FoldKey::Response(0);
        state.toggle(key.clone(), FoldContext::past(false)); // expanded → collapsed
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
        state.set_mode(FoldPreset::Auto.mode());
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
        state.set_mode(FoldPreset::Summary.mode());
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn the_two_chip_owned_rows_ignore_every_mode() {
        for preset in FoldPreset::ALL {
            let state = FoldState::with_mode(preset.mode());
            for key in [FoldKey::Tail(0), FoldKey::Filtered(0)] {
                for ctx in [FoldContext::past(true), FoldContext::last(true)] {
                    assert!(!state.is_expanded(&key, ctx), "{preset:?} {key:?}");
                }
            }
        }
    }

    #[test]
    fn chip_changes_clear_only_their_own_reveals() {
        let mut state = FoldState::default();
        state.set_all(
            [
                FoldKey::Tail(1),
                FoldKey::Tail(9),
                FoldKey::Filtered(1),
                FoldKey::Tool("t".into()),
            ],
            true,
        );

        assert!(state.clear_tail_reveals());
        assert!(!state.is_expanded(&FoldKey::Tail(1), FoldContext::past(false)));
        assert!(!state.is_expanded(&FoldKey::Tail(9), FoldContext::past(false)));
        assert!(
            state.is_expanded(&FoldKey::Filtered(1), FoldContext::past(false)),
            "the filter chip owns a separate reveal"
        );
        assert!(state.is_expanded(&FoldKey::Tool("t".into()), FoldContext::past(false)));

        assert!(state.clear_filter_reveals());
        assert!(!state.is_expanded(&FoldKey::Filtered(1), FoldContext::past(false)));
        assert!(state.is_expanded(&FoldKey::Tool("t".into()), FoldContext::past(false)));
        assert!(!state.clear_filter_reveals(), "a second clear is a no-op");
    }

    #[test]
    fn a_custom_rule_can_hold_one_block_kind_open() {
        use crate::transcript::fold_mode::FoldMode;
        let state = FoldState::with_mode(FoldMode::from_tokens(["auto", "last.tool=expanded"]));
        assert!(state.is_expanded(&FoldKey::Tool("t".into()), FoldContext::last(false)));
        assert!(!state.is_expanded(&FoldKey::Tool("t".into()), FoldContext::past(false)));
    }

    #[test]
    fn a_tool_category_rule_outranks_the_generic_tool_rule() {
        use crate::transcript::tool_category::ToolCategory;
        let mode = FoldPreset::Summary
            .mode()
            .with_rule(TurnPosition::Last, FoldBlock::Tool, BlockRule::Collapsed)
            .with_tool_rule(TurnPosition::Last, ToolCategory::Edit, BlockRule::Expanded);
        let state = FoldState::with_mode(mode);
        let key = FoldKey::Tool("edit".into());
        assert!(state.is_expanded(
            &key,
            FoldContext::last(false).with_tool_category(ToolCategory::Edit)
        ));
        assert!(!state.is_expanded(
            &key,
            FoldContext::last(false).with_tool_category(ToolCategory::Read)
        ));
    }

    #[test]
    fn a_held_response_stays_open_once_it_is_no_longer_the_newest() {
        let mut state = FoldState::default();
        let key = FoldKey::Response(0);
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
        state.hold_response(Some(0));
        assert!(state.is_expanded(&key, FoldContext::past(false)));
        assert!(
            !state.is_expanded(&FoldKey::Response(4), FoldContext::past(false)),
            "the hold covers one anchor, not every response"
        );
    }

    #[test]
    fn a_second_hold_releases_the_first() {
        let mut state = FoldState::default();
        state.hold_response(Some(0));
        state.hold_response(Some(4));
        assert!(!state.is_expanded(&FoldKey::Response(0), FoldContext::past(false)));
        assert!(state.is_expanded(&FoldKey::Response(4), FoldContext::past(false)));
        state.hold_response(None);
        assert!(!state.is_expanded(&FoldKey::Response(4), FoldContext::past(false)));
    }

    #[test]
    fn choosing_a_mode_releases_the_hold_but_not_a_user_override() {
        let mut state = FoldState::default();
        state.hold_response(Some(0));
        state.set_all([FoldKey::Response(4)], true);
        state.set_mode(FoldPreset::Summary.mode());
        assert!(
            !state.is_expanded(&FoldKey::Response(0), FoldContext::past(false)),
            "the mode chip outranks a machine-written hold"
        );
        assert!(
            state.is_expanded(&FoldKey::Response(4), FoldContext::past(false)),
            "an explicit user choice still outranks the mode"
        );
    }

    #[test]
    fn a_user_collapse_outranks_the_hold() {
        let mut state = FoldState::default();
        state.hold_response(Some(0));
        let key = FoldKey::Response(0);
        state.toggle(key.clone(), FoldContext::past(false)); // expanded → collapsed
        assert!(!state.is_expanded(&key, FoldContext::past(false)));
    }

    #[test]
    fn set_all_can_collapse_many_at_once() {
        let mut state = FoldState::default();
        let keys = [
            FoldKey::Assistant(0),
            FoldKey::Thinking(1),
            FoldKey::Tool("call-1".into()),
        ];
        state.set_all(keys.iter().cloned(), false);
        for key in &keys {
            assert!(!state.is_expanded(key, FoldContext::past(true)));
        }
    }
}
