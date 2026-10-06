//! Stateful list updates must converge with a cold render without losing history.

mod fixture;
mod observation;

use fixture::{Fixture, Step};
use gpui::{TestAppContext, px};
use observation::assert_pixels;

#[gpui::test]
fn middle_card_growth_and_folding_match_a_fresh_list(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new(cx);
    fixture.step(
        Step::Scroll {
            tool: 3,
            offset: 12.,
        },
        cx,
    );
    cx.run_until_parked();
    let before = fixture.snapshot(cx);
    let short = fixture.tool_height(3, cx);
    let sibling = fixture.editor_id(4, cx);

    fixture.step(Step::Output { tool: 3, lines: 9 }, cx);
    cx.run_until_parked();
    let grown = fixture.tool_height(3, cx);
    assert!(
        grown > short,
        "the update must actually grow the target card: {short:?} -> {grown:?}"
    );
    let after = fixture.snapshot(cx);
    assert!(
        after.paints > before.paints,
        "the handler must schedule a paint"
    );
    assert_eq!(after.scroll.item_ix, before.scroll.item_ix);
    assert_pixels(
        after.scroll.offset_in_item,
        before.scroll.offset_in_item,
        "history offset",
    );
    assert!(!after.following);
    assert_eq!(
        fixture.editor_id(4, cx),
        sibling,
        "untouched editor identity"
    );
    fixture.assert_matches_fresh(cx);

    fixture.step(Step::Toggle(3), cx);
    cx.run_until_parked();
    assert!(
        fixture.tool_height(3, cx) < grown,
        "fold must shrink the row"
    );
    fixture.assert_matches_fresh(cx);
    fixture.step(Step::Toggle(3), cx);
    cx.run_until_parked();
    assert_pixels(fixture.tool_height(3, cx), grown, "reopened card");
    fixture.assert_matches_fresh(cx);
}

#[gpui::test]
fn tail_updates_preserve_the_history_anchor_with_both_paint_schedules(cx: &mut TestAppContext) {
    let mut outcomes = Vec::new();
    for paint_each in [true, false] {
        let mut fixture = Fixture::new(cx);
        fixture.step(
            Step::Scroll {
                tool: 3,
                offset: 12.,
            },
            cx,
        );
        cx.run_until_parked();
        let before = fixture.snapshot(cx);
        for lines in [5, 7, 9] {
            fixture.step(Step::Output { tool: 11, lines }, cx);
            if paint_each {
                cx.run_until_parked();
                fixture.assert_matches_fresh(cx);
            }
        }
        cx.run_until_parked();
        let after = fixture.snapshot(cx);
        assert_eq!(after.scroll.item_ix, before.scroll.item_ix);
        assert_pixels(
            after.scroll.offset_in_item,
            before.scroll.offset_in_item,
            "history offset",
        );
        assert!(
            !after.following,
            "background output must not reclaim tail-follow"
        );
        fixture.assert_matches_fresh(cx);
        outcomes.push(after);
        fixture.close(cx);
    }
    outcomes[0].assert_matches(&outcomes[1], "per-event paints vs batched history update");
}

#[gpui::test]
fn tail_follow_converges_after_each_update_or_a_batch(cx: &mut TestAppContext) {
    let mut outcomes = Vec::new();
    for paint_each in [true, false] {
        let mut fixture = Fixture::new(cx);
        let before = fixture.snapshot(cx);
        assert!(before.following);
        let short = fixture.tool_height(11, cx);
        for lines in [5, 7, 9] {
            fixture.step(Step::Output { tool: 11, lines }, cx);
            if paint_each {
                cx.run_until_parked();
                fixture.assert_matches_fresh(cx);
            }
        }
        cx.run_until_parked();
        assert!(fixture.snapshot(cx).following);
        assert!(fixture.tool_height(11, cx) > short, "tail output must grow");
        fixture.assert_matches_fresh(cx);
        outcomes.push(fixture.snapshot(cx));
        fixture.close(cx);
    }
    outcomes[0].assert_matches(&outcomes[1], "per-event paints vs batched tail update");
}

#[gpui::test]
fn revisiting_a_previously_measured_card_uses_its_new_height(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new(cx);
    fixture.step(
        Step::Scroll {
            tool: 1,
            offset: 0.,
        },
        cx,
    );
    cx.run_until_parked();
    let old_height = fixture.tool_height(1, cx);
    fixture.step(
        Step::Scroll {
            tool: 9,
            offset: 0.,
        },
        cx,
    );
    cx.run_until_parked();
    fixture.assert_tool_above_viewport(1, cx);
    fixture.step(Step::Output { tool: 1, lines: 9 }, cx);
    cx.run_until_parked();
    fixture.step(
        Step::Scroll {
            tool: 1,
            offset: 0.,
        },
        cx,
    );
    cx.run_until_parked();
    assert!(fixture.tool_height(1, cx) > old_height + px(1.));
    fixture.assert_matches_fresh(cx);
}
