use daruda_transcript::tool_category::ToolCategory;

use super::*;

/// Every category the bar can count needs phrasing of its own. A missing
/// match arm falls through to "other" silently, and a key the locales never
/// got renders as the key itself — neither fails anywhere else.
#[test]
fn every_tool_category_has_its_own_group_label() {
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

/// A segment of one is the common case in a mixed group, so every category
/// needs a singular form — "1 files read" is the shape this guards against.
#[test]
fn every_category_has_both_a_singular_and_a_plural_form() {
    for locale in ["en", "ko"] {
        for category in ToolCategory::ALL {
            let one_key = format!("agent_chat.group_{}_one", category.token());
            let many_key = format!("agent_chat.group_{}", category.token());
            let one = rust_i18n::t!(&one_key, locale = locale);
            let many = rust_i18n::t!(&many_key, count = 3, locale = locale);
            assert!(
                !one.contains("group_") && !many.contains("group_"),
                "{locale}/{} is missing a form (t! echoes the key when unresolved)",
                category.token()
            );
            assert_ne!(one, many, "{locale}/{}", category.token());
        }
    }
    // Pinned, because the defect this guards is a *wording* one: the plural
    // form printed for a count of one ("1 files read").
    for (category, expected) in [
        ("read", "1 file read"),
        ("edit", "1 file edited"),
        ("delete", "1 file deleted"),
        ("search", "1 search"),
        ("run", "1 command"),
        ("fetch", "1 web call"),
        ("mcp", "1 MCP call"),
        ("agent", "1 agent"),
        ("other", "1 other call"),
    ] {
        let key = format!("agent_chat.group_{category}_one");
        assert_eq!(rust_i18n::t!(&key, locale = "en"), expected);
    }
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

const EN: &str = include_str!("../../locales/en.yml");
const KO: &str = include_str!("../../locales/ko.yml");

#[test]
fn locale_en_ko_key_parity() {
    strings_gen::check_key_parity(EN, KO).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn locale_en_ko_placeholder_parity() {
    strings_gen::check_placeholder_parity(EN, KO).unwrap_or_else(|e| panic!("{e}"));
}
