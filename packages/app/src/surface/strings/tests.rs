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

/// Recursively collect dotted key paths for every scalar leaf in a
/// YAML mapping tree (e.g. `common.btn_cancel`).
fn collect_locale_keys(
    value: &yaml_serde::Value,
    prefix: &str,
    out: &mut std::collections::BTreeSet<String>,
) {
    if let yaml_serde::Value::Mapping(map) = value {
        for (k, v) in map {
            let key = k.as_str().unwrap_or("<non-string-key>");
            let path = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            collect_locale_keys(v, &path, out);
        }
    } else {
        out.insert(prefix.to_string());
    }
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

/// Every i18n key in `en.yml` must have a counterpart in `ko.yml`
/// and vice versa. A missing translation silently renders the raw
/// key string at runtime, so key drift must fail the build.
#[test]
fn locale_en_ko_key_parity() {
    let en: yaml_serde::Value =
        yaml_serde::from_str(include_str!("../../../locales/en.yml")).unwrap();
    let ko: yaml_serde::Value =
        yaml_serde::from_str(include_str!("../../../locales/ko.yml")).unwrap();

    let mut en_keys = std::collections::BTreeSet::new();
    let mut ko_keys = std::collections::BTreeSet::new();
    collect_locale_keys(&en, "", &mut en_keys);
    collect_locale_keys(&ko, "", &mut ko_keys);

    let missing_in_ko: Vec<_> = en_keys.difference(&ko_keys).collect();
    let missing_in_en: Vec<_> = ko_keys.difference(&en_keys).collect();
    assert!(
        missing_in_ko.is_empty() && missing_in_en.is_empty(),
        "i18n key drift between en.yml and ko.yml:\n  missing in ko.yml: {missing_in_ko:?}\n  missing in en.yml: {missing_in_en:?}"
    );
}

/// Recursively collect `dotted.key -> the set of %{placeholder} names it
/// interpolates`, for every scalar leaf.
fn collect_locale_placeholders(
    value: &yaml_serde::Value,
    prefix: &str,
    out: &mut std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) {
    match value {
        yaml_serde::Value::Mapping(map) => {
            for (k, v) in map {
                let key = k.as_str().unwrap_or("<non-string-key>");
                let path = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_locale_placeholders(v, &path, out);
            }
        }
        yaml_serde::Value::String(text) => {
            let mut names = std::collections::BTreeSet::new();
            let mut rest = text.as_str();
            while let Some(open) = rest.find("%{") {
                rest = &rest[open + 2..];
                let Some(close) = rest.find('}') else { break };
                names.insert(rest[..close].trim().to_string());
                rest = &rest[close + 1..];
            }
            out.insert(prefix.to_string(), names);
        }
        _ => {}
    }
}

/// A key present in both files but interpolating different placeholders is
/// invisible to [`locale_en_ko_key_parity`], compiles, and renders a
/// literal `%{name}` to whoever is running that locale. `rust_i18n`
/// substitutes by name at the call site, so a translated string that
/// renamed or dropped one is a runtime-only defect no other check catches.
#[test]
fn locale_en_ko_placeholder_parity() {
    let en: yaml_serde::Value =
        yaml_serde::from_str(include_str!("../../../locales/en.yml")).unwrap();
    let ko: yaml_serde::Value =
        yaml_serde::from_str(include_str!("../../../locales/ko.yml")).unwrap();

    let mut en_ph = std::collections::BTreeMap::new();
    let mut ko_ph = std::collections::BTreeMap::new();
    collect_locale_placeholders(&en, "", &mut en_ph);
    collect_locale_placeholders(&ko, "", &mut ko_ph);

    let mismatched: Vec<String> = en_ph
        .iter()
        .filter_map(|(key, en_names)| {
            let ko_names = ko_ph.get(key)?;
            (en_names != ko_names).then(|| format!("{key}: en {en_names:?} vs ko {ko_names:?}"))
        })
        .collect();
    assert!(
        mismatched.is_empty(),
        "i18n placeholder drift between en.yml and ko.yml:\n  {}",
        mismatched.join("\n  ")
    );
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

/// The generator `build.rs` runs, compiled here so its rules are pinned.
#[path = "../../../build/strings_gen.rs"]
#[allow(dead_code)]
mod strings_gen;

/// The `#` lines directly above a key are its doc; a blank line ends them,
/// so a group header standing on its own documents nothing.
#[test]
fn only_comments_touching_a_key_document_it() {
    let en =
        "menu:\n  # File menu\n\n  # Opens a window.\n  new_window: \"New\"\n  open: \"Open\"\n";
    let sections = strings_gen::parse(en).expect("parses");
    let keys = &sections[0].1;
    assert_eq!(keys[0].name, "new_window");
    assert_eq!(keys[0].doc, ["Opens a window."]);
    assert!(keys[1].doc.is_empty());
}

/// Placeholders become parameters once each, in the order the English value
/// first names them — a `|-` block value included.
#[test]
fn placeholders_become_parameters_in_first_use_order() {
    let en = "flow:\n  a: \"%{node} then %{n}, %{node} again\"\n  b: |-\n    first %{x}\n    then %{y}\n";
    let sections = strings_gen::parse(en).expect("parses");
    assert_eq!(sections[0].1[0].params, ["node", "n"]);
    assert_eq!(sections[0].1[1].params, ["x", "y"]);
}

/// A key that cannot be spelled as a Rust function fails the build by name
/// rather than generating code that does not compile.
#[test]
fn a_key_that_is_not_an_identifier_is_refused() {
    for en in [
        "git:\n  type: \"x\"\n",
        "git:\n  7day: \"x\"\n",
        "git:\n  a: \"%{Name}\"\n",
    ] {
        let err = strings_gen::parse(en).err().expect("refused");
        assert!(err.contains("Rust identifier"), "{err}");
    }
}

/// A custom function named like a key replaces the generated one, and a
/// custom file whose section does not exist is refused.
#[test]
fn a_custom_function_replaces_its_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("flow.rs"),
        "pub(crate) fn pin(n: usize) -> String {\n    n.to_string()\n}\n",
    )
    .expect("write");
    let out = strings_gen::generate("flow:\n  pin: \"%{n}\"\n  stop: \"Stop\"\n", dir.path())
        .expect("generates");
    assert!(
        out.contains("pub(crate) use super::custom::flow::*;"),
        "{out}"
    );
    assert!(!out.contains("fn pin("), "{out}");
    assert!(out.contains("fn stop()"), "{out}");

    std::fs::write(dir.path().join("nowhere.rs"), "").expect("write");
    let err = strings_gen::generate("flow:\n  stop: \"Stop\"\n", dir.path()).expect_err("refused");
    assert!(err.contains("names no section"), "{err}");
}
