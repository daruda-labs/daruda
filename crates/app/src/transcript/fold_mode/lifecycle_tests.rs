use super::*;

#[test]
fn all_lifecycle_pairs_can_be_edited_independently() {
    for rule in BlockRule::ALL {
        for active in [false, true] {
            for expanded in [false, true] {
                let next = rule.with_expanded(active, expanded);
                assert_eq!(next.is_expanded(active), expanded);
                assert_eq!(next.is_expanded(!active), rule.is_expanded(!active));
            }
        }
    }
    assert!(!BlockRule::AfterRunning.is_expanded(true));
    assert!(BlockRule::AfterRunning.is_expanded(false));
}

#[test]
fn lifecycle_and_history_tokens_round_trip_without_changing_legacy_defaults() {
    for preset in FoldPreset::ALL {
        assert!(!preset.mode().collapse_history());
        for rule in BlockRule::ALL {
            let mode = preset
                .mode()
                .with_rule(TurnPosition::Last, FoldBlock::Tool, rule)
                .with_collapse_history(true);
            assert_eq!(
                FoldMode::from_tokens(mode.tokens().iter().map(String::as_str)),
                mode
            );
        }
    }
    let mode = FoldMode::from_tokens(["auto", "history=collapsed", "history=preserved"]);
    assert_eq!(mode, FoldPreset::Auto.mode());
}

#[test]
fn the_history_flag_is_independent_of_the_preset() {
    for preset in FoldPreset::ALL {
        let mode = preset.mode().with_collapse_history(true);
        assert_eq!(
            mode.preset(),
            Some(preset),
            "the flag is not part of the matrix"
        );
        assert_eq!(
            mode.tokens(),
            [preset.token().to_owned(), HISTORY_COLLAPSED.to_owned()]
        );
        for tokens in [
            [preset.token(), HISTORY_COLLAPSED],
            [HISTORY_COLLAPSED, preset.token()],
        ] {
            assert_eq!(FoldMode::from_tokens(tokens), mode, "{tokens:?}");
        }
    }
}

#[test]
fn history_off_round_trips_settled_cells() {
    let mode = FoldPreset::Auto
        .mode()
        .with_rule(TurnPosition::Past, FoldBlock::Diff, BlockRule::AfterRunning)
        .with_tool_rule(
            TurnPosition::Last,
            ToolCategory::Read,
            BlockRule::AfterRunning,
        );
    assert!(!mode.collapse_history());
    assert!(!mode.tokens().iter().any(|t| t.starts_with("history=")));
    assert_eq!(
        FoldMode::from_tokens(mode.tokens().iter().map(String::as_str)),
        mode
    );
}

#[test]
fn editing_the_newest_turn_carries_matching_past_cells_across_a_preset() {
    let mode = FoldPreset::Expanded.mode();
    let next = mode.with_phase(TurnPosition::Last, FoldBlock::Response, false, false);
    for turn in TurnPosition::ALL {
        assert_eq!(
            next.rule(turn, FoldBlock::Response),
            Some(BlockRule::WhileRunning),
            "{turn:?}"
        );
    }
    let tools = mode.with_phase(TurnPosition::Last, FoldBlock::Tool, false, true);
    for turn in TurnPosition::ALL {
        assert_eq!(
            tools.phase(turn, FoldBlock::Tool, false),
            Some(true),
            "{turn:?}"
        );
    }
}

#[test]
fn editing_a_shared_phase_preserves_different_historical_rules() {
    let mode = FoldPreset::Summary.mode().with_rule(
        TurnPosition::Past,
        FoldBlock::Diff,
        BlockRule::Collapsed,
    );
    let next = mode.with_phase(TurnPosition::Last, FoldBlock::Diff, true, false);
    assert_eq!(
        next.rule(TurnPosition::Last, FoldBlock::Diff),
        Some(BlockRule::AfterRunning)
    );
    assert_eq!(
        next.rule(TurnPosition::Past, FoldBlock::Diff),
        Some(BlockRule::Collapsed)
    );
    let shared = mode.with_phase(TurnPosition::Last, FoldBlock::Thinking, false, true);
    for turn in TurnPosition::ALL {
        assert_eq!(
            shared.rule(turn, FoldBlock::Thinking),
            Some(BlockRule::Expanded)
        );
    }
}

#[test]
fn mixed_tools_are_reported_per_phase_and_other_phases_survive_bulk_edits() {
    let mode = FoldPreset::Summary.mode().with_tool_rule(
        TurnPosition::Last,
        ToolCategory::Read,
        BlockRule::WhileRunning,
    );
    assert_eq!(mode.phase(TurnPosition::Last, FoldBlock::Tool, true), None);
    assert_eq!(
        mode.phase(TurnPosition::Last, FoldBlock::Tool, false),
        Some(false)
    );
    let next = mode.with_phase(TurnPosition::Last, FoldBlock::Tool, false, true);
    assert_eq!(
        next.tool_rule(TurnPosition::Last, ToolCategory::Read),
        BlockRule::Expanded
    );
    assert_eq!(
        next.tool_rule(TurnPosition::Last, ToolCategory::Edit),
        BlockRule::AfterRunning
    );
    assert_eq!(
        next.phase(TurnPosition::Last, FoldBlock::Tool, false),
        Some(true)
    );
    assert_eq!(
        next.tool_rule(TurnPosition::Past, ToolCategory::Read),
        BlockRule::Collapsed
    );
}
