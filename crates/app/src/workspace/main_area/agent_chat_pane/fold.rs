//! GPUI-free fold-state core for the agent chat pane.
//!
//! Stores only user overrides; untouched keys derive their open/closed state
//! from block kind plus whether the block is active. That keeps "expand while
//! active, collapse when settled" out of render code.

// INVARIANT: `FoldKey::Assistant`/`Thinking` are keyed by item index; this is
// valid only because `items` is append-only (only the tail mutates in place; no
// item is removed or reordered). Any future feature that removes or reorders
// items MUST clear `FoldState` (its index-keyed overrides would otherwise
// mis-target).

use std::collections::HashMap;

/// Identity of a foldable block. GPUI-free: uses `std::String`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(in crate::workspace) enum FoldKey {
    /// Assistant response, keyed by item index.
    Assistant(usize),
    /// Thinking block, keyed by item index.
    Thinking(usize),
    /// Tool call, keyed by tool-call id (`ToolCallItem.id`).
    Tool(String),
    /// Diff inside a tool call, keyed by `"{tool_id}#{diff_index}"`.
    Diff(String),
    /// A tool call's raw-input (JSON args) disclosure, keyed by tool-call id.
    ToolRawInput(String),
    /// Subagent-launch tool call; distinct so it can default collapsed.
    Subagent(String),
    /// A consecutive tool-call group, keyed by the group's first tool-call id.
    ToolGroup(String),
    /// An agent response (the run of agent items under a user message), keyed by
    /// the anchoring `UserText` index.
    Response(usize),
}

/// Default fold behavior for a block kind.
enum FoldPolicy {
    /// Always expanded by default (e.g. assistant prose).
    DefaultExpanded,
    /// Expanded only while the block is active; collapses once settled.
    ExpandedWhileActive,
    /// Always collapsed by default (e.g. diffs).
    DefaultCollapsed,
}

impl FoldKey {
    fn policy(&self) -> FoldPolicy {
        match self {
            // Assistant prose and file-edit diffs stay visible by default.
            FoldKey::Assistant(_) | FoldKey::Diff(_) => FoldPolicy::DefaultExpanded,
            FoldKey::Thinking(_)
            | FoldKey::Tool(_)
            | FoldKey::ToolGroup(_)
            | FoldKey::Response(_) => FoldPolicy::ExpandedWhileActive,
            // Raw JSON and nested subagent activity are bulky; keep them
            // collapsed even while active unless the user expands them.
            FoldKey::ToolRawInput(_) | FoldKey::Subagent(_) => FoldPolicy::DefaultCollapsed,
        }
    }
}

/// Override-free expanded state for a policy.
fn natural_default(policy: FoldPolicy, active: bool) -> bool {
    match policy {
        FoldPolicy::DefaultExpanded => true,
        FoldPolicy::ExpandedWhileActive => active,
        FoldPolicy::DefaultCollapsed => false,
    }
}

/// Fold state for one conversation; stores only explicit user choices.
#[derive(Default)]
pub(in crate::workspace) struct FoldState {
    /// Present = explicit user choice; absent = derive the natural default.
    overrides: HashMap<FoldKey, bool>,
}

impl FoldState {
    /// `active` = "this block is currently streaming / in progress".
    ///
    /// Returns the user override if set, else the natural default for the kind.
    pub(in crate::workspace) fn is_expanded(&self, key: &FoldKey, active: bool) -> bool {
        self.overrides
            .get(key)
            .copied()
            .unwrap_or_else(|| natural_default(key.policy(), active))
    }

    /// Flip the current effective state and persist it as a user override.
    pub(in crate::workspace) fn toggle(&mut self, key: FoldKey, active: bool) {
        let cur = self.is_expanded(&key, active);
        self.overrides.insert(key, !cur);
    }

    /// Force every given key to `expanded`, overwriting any previous explicit
    /// override. Used for expand-all / collapse-all.
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

    // 1. natural_default / is_expanded matrix: each kind × active ∈ {true,false}
    //    with NO override.

    #[test]
    fn assistant_is_always_expanded_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Assistant(0), true));
        assert!(state.is_expanded(&FoldKey::Assistant(0), false));
    }

    #[test]
    fn thinking_tracks_active_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Thinking(0), true));
        assert!(!state.is_expanded(&FoldKey::Thinking(0), false));
    }

    #[test]
    fn tool_tracks_active_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Tool("call-1".into()), true));
        assert!(!state.is_expanded(&FoldKey::Tool("call-1".into()), false));
    }

    #[test]
    fn diff_is_expanded_by_default() {
        let state = FoldState::default();
        assert!(state.is_expanded(&FoldKey::Diff("call-1#0".into()), true));
        assert!(state.is_expanded(&FoldKey::Diff("call-1#0".into()), false));
    }

    #[test]
    fn tool_raw_input_is_collapsed_by_default() {
        let state = FoldState::default();
        assert!(!state.is_expanded(&FoldKey::ToolRawInput("call-1".into()), true));
        assert!(!state.is_expanded(&FoldKey::ToolRawInput("call-1".into()), false));
    }

    #[test]
    fn subagent_is_collapsed_by_default_even_while_active() {
        let state = FoldState::default();
        let key = FoldKey::Subagent("task-1".into());
        // Unlike a plain Tool (expanded while active), a subagent box is folded
        // from the start and stays folded while it runs.
        assert!(!state.is_expanded(&key, true));
        assert!(!state.is_expanded(&key, false));
    }

    #[test]
    fn subagent_override_expands_and_sticks_across_active() {
        let mut state = FoldState::default();
        let key = FoldKey::Subagent("task-1".into());
        // Default collapsed → toggle expands, and the choice persists whether
        // the subagent is still running or has settled.
        state.toggle(key.clone(), false);
        assert!(state.is_expanded(&key, false));
        assert!(state.is_expanded(&key, true));
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

    // The headline consequence: a never-touched Thinking/Tool block auto-expands
    // while active and auto-collapses once settled, with no separate logic.
    #[test]
    fn untouched_block_auto_collapses_when_done() {
        let state = FoldState::default();
        let key = FoldKey::Tool("call-1".into());
        // Streaming → expanded.
        assert!(state.is_expanded(&key, true));
        // Settled → collapsed, purely from derivation.
        assert!(!state.is_expanded(&key, false));
    }

    // 2. Override precedence: an explicit choice wins regardless of `active`.

    #[test]
    fn override_via_set_all_beats_active() {
        let mut state = FoldState::default();
        let key = FoldKey::Thinking(3);
        state.set_all([key.clone()], false);
        // Even though active=true would naturally expand, the override holds.
        assert!(!state.is_expanded(&key, true));
        assert!(!state.is_expanded(&key, false));
    }

    #[test]
    fn override_expanded_beats_collapsed_default() {
        let mut state = FoldState::default();
        let key = FoldKey::ToolRawInput("call-1".into());
        state.set_all([key.clone()], true);
        // Raw input defaults to collapsed, but the override expands it regardless.
        assert!(state.is_expanded(&key, true));
        assert!(state.is_expanded(&key, false));
    }

    #[test]
    fn user_override_persists_across_active_changes() {
        let mut state = FoldState::default();
        let key = FoldKey::Tool("call-1".into());
        // User collapses it while it is active.
        state.toggle(key.clone(), true); // active default true → false
        assert!(!state.is_expanded(&key, true));
        // Block settles; override still sticks (does not re-derive).
        assert!(!state.is_expanded(&key, false));
    }

    // 3. toggle flips the effective state.

    #[test]
    fn toggle_never_touched_raw_input_flips_to_expanded_then_back() {
        let mut state = FoldState::default();
        let key = FoldKey::ToolRawInput("call-1".into());
        // Default false → toggle → true.
        state.toggle(key.clone(), false);
        assert!(state.is_expanded(&key, false));
        // Toggle again → false.
        state.toggle(key.clone(), false);
        assert!(!state.is_expanded(&key, false));
    }

    #[test]
    fn toggle_never_touched_assistant_flips_to_collapsed() {
        let mut state = FoldState::default();
        let key = FoldKey::Assistant(0);
        // Default true → toggle → false.
        state.toggle(key.clone(), false);
        assert!(!state.is_expanded(&key, false));
    }

    #[test]
    fn toggle_respects_active_for_effective_default() {
        let mut state = FoldState::default();
        let key = FoldKey::Tool("call-1".into());
        // Active=true → effective default expanded → toggle collapses.
        state.toggle(key.clone(), true);
        assert!(!state.is_expanded(&key, true));

        let mut state2 = FoldState::default();
        let key2 = FoldKey::Tool("call-2".into());
        // Active=false → effective default collapsed → toggle expands.
        state2.toggle(key2.clone(), false);
        assert!(state2.is_expanded(&key2, false));
    }

    // 4. set_all affects only the given keys.

    #[test]
    fn set_all_affects_only_given_keys() {
        let mut state = FoldState::default();
        let touched = FoldKey::ToolRawInput("call-1".into());
        let untouched = FoldKey::ToolRawInput("call-2".into());
        state.set_all([touched.clone()], true);

        assert!(state.is_expanded(&touched, false));
        // The untouched key still derives its natural default (collapsed).
        assert!(!state.is_expanded(&untouched, false));
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
            // Override forces collapsed regardless of active.
            assert!(!state.is_expanded(key, true));
        }
    }
}
