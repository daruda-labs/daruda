use super::*;
use std::time::Duration;

/// The run list only keeps recent runs, so the label drops the year to
/// stay narrow. Asserted zone-independently: whatever the machine's zone,
/// a 2026 instant must not print "2026".
#[test]
fn a_run_start_label_carries_no_year() {
    let at = chrono::DateTime::parse_from_rfc3339("2026-07-01T14:32:05Z")
        .expect("valid rfc3339")
        .to_utc();
    let label = flow::run_started_at(at);
    assert!(!label.contains("2026"), "{label:?} should have no year");
    assert!(!label.is_empty());
}

/// Every category the bar can count needs phrasing of its own. A missing
/// match arm falls through to "other" silently, and a key the locales never
/// got renders as the key itself — neither fails anywhere else.
#[test]
fn every_tool_category_has_its_own_group_label() {
    use crate::transcript::tool_category::ToolCategory;
    let other = agent_chat::group_category(ToolCategory::Other.token(), 3);
    for category in ToolCategory::ALL {
        for count in [1, 3] {
            let label = agent_chat::group_category(category.token(), count);
            assert!(
                !label.contains("agent_chat."),
                "{category:?} renders a raw key: {label}"
            );
            assert!(
                label.contains(&count.to_string()),
                "{category:?} drops the count: {label}"
            );
        }
        if category != ToolCategory::Other {
            assert_ne!(
                agent_chat::group_category(category.token(), 3),
                other,
                "{category:?} falls through to the catch-all"
            );
        }
    }
}

const EN: &str = include_str!("../../../locales/en.yml");
const KO: &str = include_str!("../../../locales/ko.yml");

#[test]
fn locale_en_ko_key_parity() {
    strings_gen::check_key_parity(EN, KO).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn locale_en_ko_placeholder_parity() {
    strings_gen::check_placeholder_parity(EN, KO).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn telegram_notification_labels_are_non_empty() {
    assert!(!super::notification::telegram_reply_ack().is_empty());
    assert!(!super::notification::telegram_reply_queued().is_empty());
    assert!(!super::notification::telegram_first_tool_ack().is_empty());
}

#[test]
fn byte_size_and_media_labels_cover_boundaries() {
    // `{:.0}` rounds a KB fraction to the nearest integer, so everything
    // below the 512-byte half-KB midpoint (inclusive, round-half-to-even)
    // reads "0 KB" rather than "1 KB".
    for (bytes, expected) in [
        (0, "0 KB"),
        (511, "0 KB"),
        (512, "0 KB"),
        (513, "1 KB"),
        (2 * 1024 * 1024, "2.0 MB"),
    ] {
        assert_eq!(agent_chat::format_byte_size(bytes), expected, "{bytes}");
    }

    let label = agent_chat::tool_media_label("", 1024);
    assert_eq!(label, "[1 KB]");
    assert!(!label.contains('·'));
}

#[test]
fn compact_duration_labels_cover_thresholds() {
    for (secs, expected) in [
        (0, "0s"),
        (42, "42s"),
        (59, "59s"),
        (60, "1m 00s"),
        (63, "1m 03s"),
        (3_599, "59m 59s"),
        (3_600, "1h 00m"),
        (8_115, "2h 15m"),
    ] {
        assert_eq!(
            notification::format_duration_compact(Duration::from_secs(secs)),
            expected,
            "{secs}"
        );
    }
}

#[test]
fn reset_countdown_labels_cover_thresholds() {
    for (secs, expected) in [
        (0, "Resets now"),
        (1, "Resets in <1m"),
        (30, "Resets in <1m"),
        (59, "Resets in <1m"),
        (60, "Resets in 1m"),
        (59 * 60, "Resets in 59m"),
        (3_600, "Resets in 1h 0m"),
        (2 * 3_600 + 14 * 60, "Resets in 2h 14m"),
        (23 * 3_600 + 59 * 60, "Resets in 23h 59m"),
        (24 * 3_600, "Resets in 1d 0h"),
        (3 * 24 * 3_600 + 7 * 3_600, "Resets in 3d 7h"),
    ] {
        assert_eq!(
            usage::format_reset_countdown(Duration::from_secs(secs)),
            expected,
            "{secs}"
        );
    }
}

#[test]
fn service_status_labels_cover_description_policy() {
    use daruda_agent::{ServiceStatus, StatusIndicator};

    for (indicator, description, expected) in [
        (StatusIndicator::None, "stale message", "Operational"),
        (
            StatusIndicator::Minor,
            "Increased 4xx errors on /messages",
            "Increased 4xx errors on /messages",
        ),
        (StatusIndicator::Major, "", "Partial outage"),
        (StatusIndicator::Critical, "", "Major outage"),
        (StatusIndicator::Unknown, "garbage", "Status unavailable"),
    ] {
        let status = ServiceStatus {
            indicator,
            description: description.into(),
            fetched_at: None,
        };
        assert_eq!(status::service_status_label(&status), expected);
    }
}

#[test]
fn bottom_input_placeholders_cover_terminal_and_agent_contexts() {
    for (agent, mode, modifier, expected) in [
        (false, None, false, bottom_dock::input_placeholder()),
        (false, None, true, bottom_dock::input_placeholder()),
        (false, Some("Auto"), false, bottom_dock::input_placeholder()),
        (false, Some("Auto"), true, bottom_dock::input_placeholder()),
        (true, None, false, bottom_dock::input_agent_placeholder()),
        (
            true,
            None,
            true,
            bottom_dock::input_agent_modifier_placeholder(),
        ),
        (
            true,
            Some("Auto"),
            false,
            bottom_dock::input_agent_mode_placeholder("Auto"),
        ),
        (
            true,
            Some("Auto"),
            true,
            bottom_dock::input_agent_mode_modifier_placeholder("Auto"),
        ),
    ] {
        assert_eq!(
            bottom_dock::bottom_input_placeholder_for_context(agent, mode, modifier),
            expected
        );
    }
}

/// A title goes in through a placeholder, never as a prefix/suffix pair, so
/// every locale can place it where its own word order wants it.
#[test]
fn title_prompts_interpolate_the_title() {
    for heading in [
        task::edit_save_prompt("notes.md"),
        task::watcher_heading("notes.md"),
    ] {
        assert!(heading.contains("notes.md"), "{heading:?}");
        assert!(!heading.contains("%{"), "{heading:?}");
    }
}
