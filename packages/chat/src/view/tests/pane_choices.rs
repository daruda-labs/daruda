//! The transcript axes a pane follows from config until it chooses its own.

use super::*;

// ---------------------------------------------------------------------------
// Returning an axis to the configured default
// ---------------------------------------------------------------------------

use crate::pane_choice::PaneChoice;
use crate::rows::tail::{StepWindow, TailLevel, TailWindow};
use crate::transcript_defaults::TranscriptDefaults;
use daruda_transcript::display_filter::{DisplayFilter, FilterFacet};

/// A config state distinct from the built-in one on every axis, so a reset
/// that lands on the *built-in* default instead of the one in force is caught.
fn other_defaults() -> TranscriptDefaults {
    TranscriptDefaults {
        content_width: crate::view::ChatContentWidth::Full,
        tool_summary: crate::view::ToolSummaryStyle::Compact,
        // Distinct per level, so a reset that hands back the wrong level's
        // default is caught rather than landing on a value that happens to
        // match.
        tail: StepWindow {
            steps: TailWindow::Last(5),
            calls: TailWindow::Last(3),
        },
        fold_mode: FoldPreset::Summary.mode(),
        filter: DisplayFilter::default().toggled(FilterFacet::Thinking),
    }
}

#[gpui::test]
fn range_reset_restores_both_levels_and_resumes_config_inheritance(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, _, cx| {
            view.reseed_transcript_defaults(&other_defaults(), cx);
            view.set_tail_window(TailLevel::Steps, TailWindow::Last(1), cx);
            view.set_tail_window(TailLevel::Calls, TailWindow::All, cx);
            view.fold
                .set_all([FoldKey::Tail(0), FoldKey::Tool("manual".into())], true);
            view.reset_tail_window(cx);
            assert_eq!(view.tail_steps, PaneChoice::Seeded(TailWindow::Last(5)));
            assert_eq!(view.tail_calls, PaneChoice::Seeded(TailWindow::Last(3)));
            assert!(
                !view
                    .fold
                    .is_expanded(&FoldKey::Tail(0), FoldContext::past(false))
            );
            assert!(
                view.fold
                    .is_expanded(&FoldKey::Tool("manual".into()), FoldContext::last(false))
            );
            let mut defaults = other_defaults();
            defaults.tail = StepWindow::default();
            view.reseed_transcript_defaults(&defaults, cx);
            assert_eq!(view.tail_steps, PaneChoice::Seeded(TailWindow::All));
            assert_eq!(view.tail_calls, PaneChoice::Seeded(TailWindow::All));
        })
        .expect("view update");
}

/// The axis's two levels are independent choices: a pick lands on the level it
/// names and nowhere else (they share one setter), and the sibling keeps
/// following config through every later edit. Pinning the value the level
/// already shows is still a choice — agreement with the default is not
/// following it. A fresh pane per level, because nothing un-picks this axis.
#[gpui::test]
fn a_pick_pins_its_level_and_leaves_the_other_following(cx: &mut gpui::TestAppContext) {
    for level in TailLevel::ALL {
        let sibling = TailLevel::ALL
            .into_iter()
            .find(|l| *l != level)
            .expect("two levels");
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                let shown = view.tail_choice_for_test(level).value();
                view.set_tail_window(level, shown, cx);
                assert_eq!(view.tail_choice_for_test(level), PaneChoice::Chosen(shown));

                view.reseed_transcript_defaults(&other_defaults(), cx);
                assert_eq!(
                    view.tail_choice_for_test(level),
                    PaneChoice::Chosen(shown),
                    "{level:?}: a choice that equals the old default is still a choice"
                );
                assert_eq!(
                    view.tail_choice_for_test(sibling),
                    PaneChoice::Seeded(other_defaults().tail.get(sibling)),
                    "{level:?}'s pick must not reach {sibling:?}"
                );

                let next = TranscriptDefaults {
                    tail: StepWindow {
                        steps: TailWindow::Last(2),
                        calls: TailWindow::Last(1),
                    },
                    ..other_defaults()
                };
                view.reseed_transcript_defaults(&next, cx);
                assert_eq!(view.tail_choice_for_test(level), PaneChoice::Chosen(shown));
                assert_eq!(
                    view.tail_choice_for_test(sibling),
                    PaneChoice::Seeded(next.tail.get(sibling)),
                    "{sibling:?} tracks every later config edit, not just the first"
                );
            })
            .expect("view update");
    }
}

/// A reset hands an axis back to config. What the panel footers pass to
/// `.disabled(..)` follows the choice, not the value: a pane pinned to the
/// default's own value still has an override to undo, a following pane none.
#[gpui::test]
fn a_reset_hands_each_axis_back_to_config(cx: &mut gpui::TestAppContext) {
    let window = make_test_view(cx);
    window
        .update(cx, |view, window, cx| {
            view.reseed_transcript_defaults(&other_defaults(), cx);

            assert!(view.fold.mode_choice().is_following(), "a fresh pane");
            view.set_fold_mode(other_defaults().fold_mode, window, cx);
            assert!(
                !view.fold.mode_choice().is_following(),
                "pinning the default's own value still offers the reset"
            );
            view.reset_fold_mode(window, cx);
            assert_eq!(
                view.fold.mode_choice(),
                PaneChoice::Seeded(other_defaults().fold_mode)
            );

            assert!(view.display_filter.is_following(), "a fresh pane");
            view.toggle_display_facet(FilterFacet::ToolRun, cx);
            assert!(!view.display_filter.is_following());
            view.reset_display_filter(cx);
            assert_eq!(
                view.display_filter,
                PaneChoice::Seeded(other_defaults().filter)
            );
        })
        .expect("view update");
}

/// The width follows `agent.use_reading_width` until the pane's own toggle
/// answers, and then stops — the same seeded/chosen split the transcript axes
/// get, kept as a flag because the toggle has only two states to move between.
/// Toggling back to the configured value is still an answer.
#[gpui::test]
fn the_width_follows_config_until_the_pane_toggles_it(cx: &mut gpui::TestAppContext) {
    use crate::view::ChatContentWidth;

    let defaults = |width| TranscriptDefaults {
        content_width: width,
        ..other_defaults()
    };

    let window = make_test_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.reseed_transcript_defaults(&defaults(ChatContentWidth::Reading), cx);
            assert_eq!(view.content_width, ChatContentWidth::Reading);
            view.reseed_transcript_defaults(&defaults(ChatContentWidth::Full), cx);
            assert_eq!(
                view.content_width,
                ChatContentWidth::Full,
                "an untouched pane takes the new config value"
            );
        })
        .expect("view update");

    // Configured Reading; one toggle lands on Full, two come back to Reading.
    for (toggles, toggled) in [(1, ChatContentWidth::Full), (2, ChatContentWidth::Reading)] {
        let other = match toggled {
            ChatContentWidth::Full => ChatContentWidth::Reading,
            ChatContentWidth::Reading => ChatContentWidth::Full,
        };
        let window = make_test_view(cx);
        window
            .update(cx, |view, _window, cx| {
                view.reseed_transcript_defaults(&defaults(ChatContentWidth::Reading), cx);
                for _ in 0..toggles {
                    view.toggle_content_width(cx);
                }
                assert_eq!(view.content_width, toggled, "{toggles} toggle(s)");
                view.reseed_transcript_defaults(&defaults(other), cx);
                assert_eq!(
                    view.content_width, toggled,
                    "{toggles} toggle(s): config must not overwrite the pane's own answer"
                );
            })
            .expect("view update");
    }
}

/// The whole point of the `Custom` segment: leaving a hand-edited matrix —
/// for a preset, or by resetting to the default — must not throw it away, and
/// pressing Custom brings back the latest edit, not the first. A configured
/// default is a token list that can be a matrix of its own, so the capture
/// keys on leaving the edit, not on the default happening to be a preset.
#[gpui::test]
fn the_custom_segment_brings_back_the_latest_hand_edit(cx: &mut gpui::TestAppContext) {
    use daruda_transcript::fold_mode::{BlockRule, FoldBlock, TurnPosition};

    #[derive(Debug, Clone, Copy)]
    enum Leave {
        Preset,
        ResetToBuiltIn,
        ResetToCustomDefault,
    }
    let custom_default = FoldPreset::Summary.mode().with_rule(
        TurnPosition::Last,
        FoldBlock::Tool,
        BlockRule::Expanded,
    );
    assert!(
        custom_default.preset().is_none(),
        "the fixture only means something if the default is itself custom"
    );
    let first = FoldPreset::Auto.mode().with_rule(
        TurnPosition::Past,
        FoldBlock::Thinking,
        BlockRule::Collapsed,
    );
    let second = first.with_rule(TurnPosition::Last, FoldBlock::Diff, BlockRule::Expanded);

    for leave in [
        Leave::Preset,
        Leave::ResetToBuiltIn,
        Leave::ResetToCustomDefault,
    ] {
        let window = make_test_view(cx);
        window
            .update(cx, |view, window, cx| {
                if let Leave::ResetToCustomDefault = leave {
                    view.reseed_transcript_defaults(
                        &TranscriptDefaults {
                            fold_mode: custom_default,
                            ..other_defaults()
                        },
                        cx,
                    );
                }
                view.set_fold_mode(first, window, cx);
                view.set_fold_mode(second, window, cx);
                assert_eq!(
                    view.fold_editor.custom(),
                    Some(second),
                    "custom→custom keeps the segment target on the latest edit"
                );
                view.select_fold_preset(None, window, cx);
                assert_eq!(
                    view.fold.mode(),
                    second,
                    "clicking the selected Custom segment is a no-op"
                );

                let left = match leave {
                    Leave::Preset => {
                        view.select_fold_preset(Some(FoldPreset::Summary), window, cx);
                        FoldPreset::Summary.mode()
                    }
                    Leave::ResetToBuiltIn | Leave::ResetToCustomDefault => {
                        view.reset_fold_mode(window, cx);
                        view.defaults.fold_mode
                    }
                };
                assert_eq!(view.fold.mode(), left, "{leave:?}");

                view.select_fold_preset(None, window, cx);
                assert_eq!(
                    view.fold.mode(),
                    second,
                    "{leave:?}: the latest edit, cell for cell"
                );
            })
            .expect("view update");
    }
}
