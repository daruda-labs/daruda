//! File extension → source language identity.
//!
//! Answers only "what language is this file written in", never "can we
//! syntax-highlight it" — that second question depends on which grammars
//! and highlight queries the UI's tree-sitter registry happens to carry,
//! which this crate cannot see and which changes independently. Callers
//! that need colour must check capability themselves; the app does this in
//! `crate::ui::highlighter`.
//!
//! Keeping the two apart is what lets the GPUI-free protocol crate and the
//! app agree on the same answer. `.swift` is Swift whether or not daruda
//! can currently colour it.

/// Extension (lowercase, no leading dot) → language name.
///
/// Names follow the tree-sitter convention the UI registry uses, so a
/// value can be handed to it directly. Sorted by extension; the app's
/// `every_highlightable_language_is_reachable_by_extension` test walks
/// that registry and fails when a language it can colour has no extension
/// here, so a grammar added upstream cannot land unreachable.
pub const EXTENSION_LANGUAGES: &[(&str, &str)] = &[
    ("bash", "bash"),
    ("c", "c"),
    ("cc", "cpp"),
    ("cjs", "javascript"),
    ("cpp", "cpp"),
    ("cs", "csharp"),
    ("css", "css"),
    ("cts", "typescript"),
    ("cxx", "cpp"),
    ("diff", "diff"),
    ("ejs", "ejs"),
    ("erb", "erb"),
    ("ex", "elixir"),
    ("exs", "elixir"),
    ("gemspec", "ruby"),
    ("go", "go"),
    ("gql", "graphql"),
    ("graphql", "graphql"),
    ("h", "c"),
    ("hh", "cpp"),
    ("hpp", "cpp"),
    ("htm", "html"),
    ("html", "html"),
    ("hxx", "cpp"),
    ("java", "java"),
    ("js", "javascript"),
    ("json", "json"),
    ("jsonc", "json"),
    // No JSX grammar of its own — the JavaScript grammar parses JSX.
    ("jsx", "javascript"),
    ("markdown", "markdown"),
    ("md", "markdown"),
    ("mdx", "markdown"),
    ("mjs", "javascript"),
    ("mk", "make"),
    ("mts", "typescript"),
    ("patch", "diff"),
    ("proto", "proto"),
    ("py", "python"),
    ("pyi", "python"),
    ("pyw", "python"),
    ("rake", "ruby"),
    ("rb", "ruby"),
    ("rs", "rust"),
    ("sc", "scala"),
    ("scala", "scala"),
    ("scss", "css"),
    ("sh", "bash"),
    ("sql", "sql"),
    ("swift", "swift"),
    ("toml", "toml"),
    ("ts", "typescript"),
    ("tsx", "tsx"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("zig", "zig"),
    ("zsh", "bash"),
];

/// Language written in files with extension `ext` (no leading dot),
/// or `None` when the extension is not recognised.
///
/// Case-insensitive: the lowercasing happens here rather than at each call
/// site, because a caller that forgets it gets a silent miss — `Main.JAVA`
/// resolving to nothing looks identical to an unsupported language.
pub fn from_extension(ext: &str) -> Option<&'static str> {
    let lower = ext.to_ascii_lowercase();
    EXTENSION_LANGUAGES
        .iter()
        .find(|(extension, _)| *extension == lower)
        .map(|(_, language)| *language)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
