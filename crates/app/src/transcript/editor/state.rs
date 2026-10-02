//! What the fold editor remembers between frames, as distinct from the matrix
//! it edits.

use crate::transcript::fold_mode::{FoldMode, FoldPreset, TurnPosition};

/// Which disclosures are open and the hand-edited matrix `Custom` re-selects.
///
/// Neither is part of the value: a host that persists the mode does not persist
/// these, and two hosts editing the same agent each keep their own. That is why
/// the editor reads this rather than deriving it — the matrix `Custom` points
/// at is a history of *this* editor, not a property of the mode.
///
/// Every disclosure starts closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct FoldEditorState {
    history_rules_open: bool,
    recent_tools_open: bool,
    past_tools_open: bool,
    custom: Option<FoldMode>,
}

impl FoldEditorState {
    /// Whether the previous-response overrides are shown.
    pub(crate) fn history_rules_open(self) -> bool {
        self.history_rules_open
    }

    pub(crate) fn toggle_history_rules(&mut self) {
        self.history_rules_open = !self.history_rules_open;
    }

    /// Whether the per-category tool rows under `turn`'s section are shown.
    pub(crate) fn tools_open(self, turn: TurnPosition) -> bool {
        match turn {
            TurnPosition::Last => self.recent_tools_open,
            TurnPosition::Past => self.past_tools_open,
        }
    }

    pub(crate) fn toggle_tools(&mut self, turn: TurnPosition) {
        let open = match turn {
            TurnPosition::Last => &mut self.recent_tools_open,
            TurnPosition::Past => &mut self.past_tools_open,
        };
        *open = !*open;
    }

    pub(crate) fn custom(self) -> Option<FoldMode> {
        self.custom
    }

    /// Record a move from `current` to `next`, keeping `Custom` pointed at the
    /// latest hand-edited matrix. Pressing the already-selected `Custom`
    /// segment then re-applies that matrix instead of resurrecting an older
    /// edit, and picking a preset does not throw the edit away.
    pub(crate) fn remember(&mut self, current: FoldMode, next: FoldMode) {
        match (current.preset(), next.preset()) {
            (_, None) => self.custom = Some(next),
            (None, Some(_)) => self.custom = Some(current),
            (Some(_), Some(_)) => {}
        }
    }

    /// Leaving the edited matrix for `target`, as the reset footer does.
    ///
    /// Separate from [`Self::remember`] because the arrival is a hand-back, not
    /// a pick: `target` can itself be a matrix, and that one is where the axis
    /// is *going*, so it must not become the recall.
    pub(crate) fn remember_before_reset(&mut self, current: FoldMode, target: FoldMode) {
        if current.preset().is_none() && current != target {
            self.custom = Some(current);
        }
    }

    /// The matrix a preset-strip segment applies to `current`. `None` is the
    /// `Custom` segment, which has a target only once something has been
    /// hand-edited — the strip disables it until then. The history flag is not
    /// part of the matrix, so the segment keeps the one `current` has.
    pub(crate) fn segment_target(
        self,
        preset: Option<FoldPreset>,
        current: FoldMode,
    ) -> Option<FoldMode> {
        let matrix = match preset {
            Some(preset) => preset.mode(),
            None => self.custom?,
        };
        Some(matrix.with_collapse_history(current.collapse_history()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::fold_mode::{BlockRule, FoldBlock};

    fn matrix() -> FoldMode {
        FoldPreset::Summary.mode().with_rule(
            TurnPosition::Past,
            FoldBlock::Thinking,
            BlockRule::Collapsed,
        )
    }

    #[test]
    fn the_editor_opens_with_every_disclosure_closed() {
        let state = FoldEditorState::default();
        assert!(!state.history_rules_open());
        for turn in TurnPosition::ALL {
            assert!(!state.tools_open(turn), "{turn:?}");
        }
        assert_eq!(state.custom(), None);
    }

    #[test]
    fn the_history_rules_disclosure_toggles() {
        let mut state = FoldEditorState::default();
        state.toggle_history_rules();
        assert!(state.history_rules_open());
        state.toggle_history_rules();
        assert!(!state.history_rules_open());
    }

    /// Editing into a matrix, then picking a preset, leaves `Custom` pointing
    /// at the edit — that is what makes the segment a way back.
    #[test]
    fn a_hand_edit_survives_picking_a_preset() {
        let mut state = FoldEditorState::default();
        state.remember(FoldPreset::Auto.mode(), matrix());
        assert_eq!(state.custom(), Some(matrix()));
        state.remember(matrix(), FoldPreset::Expanded.mode());
        assert_eq!(state.custom(), Some(matrix()), "still the way back");
    }

    /// Two presets in a row have no edit between them to remember.
    #[test]
    fn preset_to_preset_remembers_nothing() {
        let mut state = FoldEditorState::default();
        state.remember(FoldPreset::Auto.mode(), FoldPreset::Summary.mode());
        assert_eq!(state.custom(), None);
    }

    /// The recall tracks the *latest* edit, so pressing the already-selected
    /// `Custom` segment re-applies what is on screen rather than an older one.
    #[test]
    fn a_second_edit_replaces_the_recall() {
        let mut state = FoldEditorState::default();
        state.remember(FoldPreset::Auto.mode(), matrix());
        let later = matrix().with_rule(TurnPosition::Last, FoldBlock::Diff, BlockRule::Expanded);
        state.remember(matrix(), later);
        assert_eq!(state.custom(), Some(later));
    }

    /// A reset whose target is itself a matrix must not make the target the
    /// recall — that is where the axis just went.
    #[test]
    fn a_reset_never_remembers_its_own_target() {
        let mut state = FoldEditorState::default();
        state.remember_before_reset(matrix(), matrix());
        assert_eq!(state.custom(), None, "landing where it already was");

        let other = FoldPreset::Summary.mode().with_rule(
            TurnPosition::Last,
            FoldBlock::Diff,
            BlockRule::Expanded,
        );
        state.remember_before_reset(matrix(), other);
        assert_eq!(state.custom(), Some(matrix()), "the edit, not the target");
    }

    /// Resetting away from a preset has no edit to keep.
    #[test]
    fn a_reset_from_a_preset_remembers_nothing() {
        let mut state = FoldEditorState::default();
        state.remember_before_reset(FoldPreset::Expanded.mode(), FoldPreset::Auto.mode());
        assert_eq!(state.custom(), None);
    }

    #[test]
    fn a_segment_keeps_the_history_flag_it_finds() {
        let mut state = FoldEditorState::default();
        state.remember(FoldPreset::Auto.mode(), matrix());
        let current = FoldPreset::Auto.mode().with_collapse_history(true);
        for preset in FoldPreset::ALL {
            assert_eq!(
                state.segment_target(Some(preset), current),
                Some(preset.mode().with_collapse_history(true))
            );
        }
        assert_eq!(
            state.segment_target(None, current),
            Some(matrix().with_collapse_history(true))
        );
    }

    #[test]
    fn each_section_opens_its_own_tool_categories() {
        let mut state = FoldEditorState::default();
        state.toggle_tools(TurnPosition::Past);
        assert!(state.tools_open(TurnPosition::Past));
        assert!(
            !state.tools_open(TurnPosition::Last),
            "the other section stays shut"
        );
        state.toggle_tools(TurnPosition::Past);
        assert!(!state.tools_open(TurnPosition::Past));
    }

    #[test]
    fn a_preset_segment_targets_its_own_matrix_and_custom_targets_the_edit() {
        let mut state = FoldEditorState::default();
        let current = FoldMode::default();
        for preset in FoldPreset::ALL {
            assert_eq!(
                state.segment_target(Some(preset), current),
                Some(preset.mode())
            );
        }
        assert_eq!(
            state.segment_target(None, current),
            None,
            "nothing edited yet"
        );
        state.remember(FoldPreset::Auto.mode(), matrix());
        assert_eq!(state.segment_target(None, current), Some(matrix()));
    }
}
