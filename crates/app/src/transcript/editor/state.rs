//! What the fold editor remembers between frames, as distinct from the matrix
//! it edits.

use crate::transcript::fold_mode::{FoldMode, FoldPreset, TurnPosition};

/// Which turn column the rule rows edit, and the hand-edited matrix the
/// `Custom` segment re-selects.
///
/// Neither is part of the value: a host that persists the mode does not persist
/// these, and two hosts editing the same agent each keep their own. That is why
/// the editor reads this rather than deriving it — the matrix `Custom` points
/// at is a history of *this* editor, not a property of the mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FoldEditorState {
    turn: TurnPosition,
    custom: Option<FoldMode>,
}

impl Default for FoldEditorState {
    /// Opens on the newest turn: that is the one the user is reading.
    fn default() -> Self {
        Self {
            turn: TurnPosition::Last,
            custom: None,
        }
    }
}

impl FoldEditorState {
    pub(crate) fn turn(self) -> TurnPosition {
        self.turn
    }

    /// Returns whether the column actually moved, so a host can skip a repaint.
    pub(crate) fn set_turn(&mut self, turn: TurnPosition) -> bool {
        let changed = self.turn != turn;
        self.turn = turn;
        changed
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

    /// The matrix a preset-strip segment applies. `None` is the `Custom`
    /// segment, which has a target only once something has been hand-edited —
    /// the strip disables it until then.
    pub(crate) fn segment_target(self, preset: Option<FoldPreset>) -> Option<FoldMode> {
        match preset {
            Some(preset) => Some(preset.mode()),
            None => self.custom,
        }
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
    fn the_editor_opens_on_the_newest_turn() {
        assert_eq!(FoldEditorState::default().turn(), TurnPosition::Last);
        assert_eq!(FoldEditorState::default().custom(), None);
    }

    #[test]
    fn moving_the_column_reports_only_a_real_move() {
        let mut state = FoldEditorState::default();
        assert!(!state.set_turn(TurnPosition::Last));
        assert!(state.set_turn(TurnPosition::Past));
        assert_eq!(state.turn(), TurnPosition::Past);
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
    fn a_preset_segment_targets_its_own_matrix_and_custom_targets_the_edit() {
        let mut state = FoldEditorState::default();
        for preset in FoldPreset::ALL {
            assert_eq!(state.segment_target(Some(preset)), Some(preset.mode()));
        }
        assert_eq!(state.segment_target(None), None, "nothing edited yet");
        state.remember(FoldPreset::Auto.mode(), matrix());
        assert_eq!(state.segment_target(None), Some(matrix()));
    }
}
