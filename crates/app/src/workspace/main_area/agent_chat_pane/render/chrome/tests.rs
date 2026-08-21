//! Unit tests for the pane-chrome derivations — the context meter, the
//! last-active timestamp, the banner's retry/reauth gates, and the
//! working-indicator labels.

use super::{
    AgentSessionStatus, banner_offers_reauth, banner_offers_retry, context_meter, format_elapsed,
    format_token_count, last_active_tooltip, running_tool_title, single_line_title,
};
use daruda_acp::{
    ChatItem, CostView, Remedy, ToolCallItem, ToolKindView, ToolStatusView, UsageView,
};

fn errored(remedy: Remedy) -> AgentSessionStatus {
    AgentSessionStatus::Error {
        message: "boom".to_owned(),
        remedy,
    }
}

/// The whole point of routing the banner through a remedy: an expired
/// login used to get a Retry button that reconnected with the same
/// credentials and failed identically.
#[test]
fn banner_withholds_retry_from_failures_it_cannot_fix() {
    for remedy in [
        Remedy::Reauthenticate,
        Remedy::ExternalAction,
        Remedy::Configure,
        Remedy::NoneAvailable,
    ] {
        assert!(
            !banner_offers_retry(&errored(remedy), true),
            "{remedy:?} is not fixed by reconnecting"
        );
    }
}

/// The cwd gate belongs to Retry alone. Signing in again fixes an expired
/// login whether or not the pane has a working directory, so inheriting
/// that gate would hide the button on exactly the failures it can fix.
#[test]
fn the_reauth_button_does_not_inherit_the_retry_cwd_gate() {
    assert!(banner_offers_reauth(&errored(Remedy::Reauthenticate)));
    assert!(!banner_offers_retry(&errored(Remedy::Reauthenticate), true));
}

/// An organization-blocked account is the case this separation exists for:
/// signing in again succeeds and changes nothing, so the button must not
/// appear next to a message that already says to ask an admin.
#[test]
fn only_a_reauthenticable_failure_offers_the_button() {
    for remedy in [
        Remedy::Retry,
        Remedy::ExternalAction,
        Remedy::Configure,
        Remedy::NoneAvailable,
    ] {
        assert!(
            !banner_offers_reauth(&errored(remedy)),
            "{remedy:?} is not fixed by signing in again"
        );
    }
}

#[test]
fn no_status_but_error_offers_a_reauth_button() {
    for status in [
        AgentSessionStatus::Idle,
        AgentSessionStatus::Connecting,
        AgentSessionStatus::Connected,
    ] {
        assert!(!banner_offers_reauth(&status), "{status:?}");
    }
}

#[test]
fn banner_offers_retry_only_for_a_transient_failure_on_a_connectable_pane() {
    assert!(banner_offers_retry(&errored(Remedy::Retry), true));
    // Mechanical gate still applies: no cwd, nothing to reconnect to.
    assert!(!banner_offers_retry(&errored(Remedy::Retry), false));
}

/// Every non-error status renders its own copy and never a retry button.
#[test]
fn banner_offers_no_retry_outside_the_error_status() {
    for status in [
        AgentSessionStatus::Idle,
        AgentSessionStatus::Connecting,
        AgentSessionStatus::Connected,
    ] {
        assert!(!banner_offers_retry(&status, true), "{status:?}");
    }
}

#[test]
fn single_line_title_collapses_newlines_tabs_and_runs() {
    assert_eq!(
        single_line_title("git commit -m \"line one\nline two\""),
        "git commit -m \"line one line two\""
    );
    assert_eq!(single_line_title("  foo\t\tbar \n baz  "), "foo bar baz");
    assert_eq!(single_line_title("already clean"), "already clean");
}

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

fn tool(id: &str, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind: ToolKindView::Edit,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        exit: None,
    })
}

#[test]
fn running_tool_title_cases() {
    let items = [
        ChatItem::AssistantText {
            text: "a".into(),
            streaming: true,
            message_id: None,
        },
        tool("c1", ToolStatusView::Completed),
    ];
    assert_eq!(running_tool_title(&items), None);

    // Settled (Completed) calls are skipped; the latest *live* one wins.
    // `Pending` counts as live (see `ToolStatusView::is_live`), so a trailing
    // Pending call outranks an earlier InProgress one.
    let items = [
        tool("c1", ToolStatusView::Completed),
        tool("c2", ToolStatusView::InProgress),
        tool("c3", ToolStatusView::Pending),
    ];
    assert_eq!(running_tool_title(&items), Some("Tool c3".to_owned()));
}

#[test]
fn format_elapsed_cases() {
    for (secs, expected) in [
        (0, "0s"),
        (5, "5s"),
        (60, "1m00s"),
        (65, "1m05s"),
        (600, "10m00s"),
    ] {
        assert_eq!(
            format_elapsed(std::time::Duration::from_secs(secs)),
            expected
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
