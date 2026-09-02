//! Unit tests for the Activity Bar's derivations — the context meter, the
//! last-active timestamp, and the compact bar's state signal.

use super::{
    DisplayFilter, FoldMode, PaneChoice, TailWindow, context_meter, every_axis_follows_config,
    format_token_count, last_active_tooltip, options_tooltip,
};
use crate::transcript::display_filter::FilterFacet;
use crate::transcript::fold_mode::FoldPreset;
use daruda_acp::{CostView, UsageView};
use daruda_config::TAIL_WINDOW_CHOICES;

/// Zone conversion and the per-locale shapes belong to `surface::timestamp`;
/// what this pane owns is the fallback — a timestamp it cannot read reaches the
/// tooltip verbatim instead of blanking it.
#[test]
fn the_last_active_tooltip_echoes_a_timestamp_it_cannot_parse() {
    for input in ["not-a-timestamp", "2026-07-01"] {
        assert!(
            last_active_tooltip(input).contains(input),
            "{input:?} did not survive into the tooltip"
        );
    }
}

/// Percent is integer-truncated, and the cost joins the tooltip only when the
/// agent reports one.
#[test]
fn context_meter_derives_label_and_tooltip() {
    let priced = UsageView {
        used: 53_000,
        size: 200_000,
        cost: Some(CostView {
            amount: 1.25,
            currency: "USD".to_owned(),
        }),
    };
    let meter = context_meter(&priced);
    assert_eq!(meter.label, "53k / 200k");
    assert_eq!(
        meter.tooltip,
        "Context: 53k / 200k tokens (26%) \u{b7} 1.25 USD"
    );

    let free = UsageView {
        cost: None,
        ..priced
    };
    assert_eq!(
        context_meter(&free).tooltip,
        "Context: 53k / 200k tokens (26%)"
    );
}

/// A window the agent reports as size 0 must not divide by it.
#[test]
fn context_meter_survives_a_zero_size_window() {
    let meter = context_meter(&UsageView {
        used: 0,
        size: 0,
        cost: None,
    });
    assert_eq!(meter.label, "0 / 0");
    assert!(meter.tooltip.contains("(0%)"));
}

/// Each locale places the cost itself: the separator and where the amount sits
/// relative to the fill are translator decisions, so the amount has to come out
/// of the locale's own pattern.
#[test]
fn every_locale_places_the_context_cost_itself() {
    for locale in ["en", "ko"] {
        let tip = rust_i18n::t!(
            "agent_chat.context_tooltip_with_cost",
            locale = locale,
            used = "53k",
            size = "200k",
            percent = "26",
            amount = "1.25",
            currency = "USD"
        );
        assert!(
            tip.contains("53k") && tip.contains("1.25") && tip.contains("USD"),
            "locale {locale} rendered {tip:?}"
        );
    }
}

#[test]
fn format_token_count_cases() {
    for (tokens, expected) in [
        (0, "0"),
        (512, "512"),
        (999, "999"),
        (1000, "1k"),
        (1500, "2k"),
        (53_000, "53k"),
        (200_000, "200k"),
    ] {
        assert_eq!(format_token_count(tokens), expected);
    }
}

/// A pane nobody has touched: every axis still following config.
fn following() -> (
    PaneChoice<FoldMode>,
    PaneChoice<DisplayFilter>,
    PaneChoice<TailWindow>,
) {
    (
        PaneChoice::Seeded(FoldMode::default()),
        PaneChoice::Seeded(DisplayFilter::default()),
        PaneChoice::Seeded(TailWindow::All),
    )
}

#[test]
fn a_fresh_pane_leaves_the_compact_gear_unmarked() {
    let (fold, filter, tail) = following();
    assert!(every_axis_follows_config(fold, filter, tail));
}

/// Each axis has to be able to mark the gear on its own — this is the signal
/// that replaces three chip labels, so one axis going unnoticed is the whole
/// bug it exists to prevent. The mark tracks the *override*, not the value:
/// a pane pinned to the default's own value has still been taken off config.
#[test]
fn any_single_overridden_axis_marks_the_compact_gear() {
    let (fold, filter, tail) = following();
    assert!(!every_axis_follows_config(
        PaneChoice::Chosen(FoldPreset::Summary.mode()),
        filter,
        tail
    ));
    assert!(!every_axis_follows_config(
        PaneChoice::Chosen(FoldMode::default()),
        filter,
        tail
    ));
    assert!(!every_axis_follows_config(
        fold,
        PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::ToolEdit)),
        tail
    ));
    assert!(!every_axis_follows_config(
        fold,
        filter,
        PaneChoice::Chosen(TailWindow::last(TAIL_WINDOW_CHOICES[0]))
    ));
}

/// The gear replaces three labelled chips, so its tooltip has to carry all
/// three values — that is the only place a compact bar states them — and the
/// overridden marks with them, so a narrowed bar still says which axis.
#[test]
fn the_compact_tooltip_names_every_axis() {
    let tip = options_tooltip(
        PaneChoice::Chosen(FoldMode::from_tokens(["auto", "last.tool=expanded"])),
        PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::ToolEdit)),
        PaneChoice::Chosen(TailWindow::last(TAIL_WINDOW_CHOICES[0])),
    );
    for value in [
        crate::surface::strings::agent_chat_fold_mode_custom(),
        crate::surface::strings::agent_chat_filter_tool_edit(),
    ] {
        assert!(tip.contains(&value), "{value:?} missing from {tip:?}");
    }
    // And the default reading names the defaults rather than going silent.
    let (fold, filter, tail) = following();
    let quiet = options_tooltip(fold, filter, tail);
    assert!(quiet.contains(&crate::surface::strings::agent_chat_fold_mode_auto()));
    assert!(quiet.contains(&crate::surface::strings::agent_chat_filter_none()));
    assert_ne!(
        quiet,
        options_tooltip(PaneChoice::Chosen(FoldMode::default()), filter, tail),
        "the tooltip carries each chip's overridden mark"
    );
}
