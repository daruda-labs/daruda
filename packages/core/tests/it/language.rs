//! Public API tests for language.

use daruda_core::language::*;

#[test]
fn resolves_known_extensions() {
    assert_eq!(from_extension("java"), Some("java"));
    assert_eq!(from_extension("rs"), Some("rust"));
    assert_eq!(from_extension("hpp"), Some("cpp"));
    assert_eq!(from_extension("jsx"), Some("javascript"));
    assert_eq!(from_extension("zsh"), Some("bash"));
}

#[test]
fn extension_case_does_not_matter() {
    assert_eq!(from_extension("JAVA"), Some("java"));
    assert_eq!(from_extension("Rs"), Some("rust"));
}

#[test]
fn unknown_extensions_resolve_to_nothing() {
    assert_eq!(from_extension(""), None);
    assert_eq!(from_extension("unknown_ext_xyz"), None);
}

#[test]
fn table_is_well_formed() {
    let mut seen = std::collections::HashSet::new();
    let mut previous = "";
    for (extension, language) in EXTENSION_LANGUAGES {
        assert!(
            seen.insert(*extension),
            "duplicate extension {extension:?} — the first entry wins silently"
        );
        assert_eq!(
            *extension,
            extension.to_ascii_lowercase(),
            "{extension:?} must be lowercase; lookups are lowercased"
        );
        assert!(!extension.is_empty() && !language.is_empty());
        assert!(
            !extension.starts_with('.'),
            "{extension:?} must not carry a leading dot"
        );
        assert!(
            *extension > previous,
            "{extension:?} breaks the sort order after {previous:?}"
        );
        previous = extension;
    }
}
