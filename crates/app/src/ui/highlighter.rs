//! Re-export of `gpui_component`'s tree-sitter language registry.
//!
//! Application code must never import `gpui_component::*` directly
//! (see `crate::ui` module docs); this is the sanctioned access point
//! for the syntax-highlighting language data.
//!
//! Only the GPUI-free *data* types are re-exported: [`LanguageRegistry`]
//! (a process-wide singleton keyed by *language name*) and
//! [`LanguageConfig`] itself, which carries the `tree_sitter::Language`
//! and the bundled `highlights` query string. `LanguageRegistry::language()`
//! takes no `App`/`Window`, so it is safe to call on `background_executor`.
//!
//! The gpui_component `SyntaxHighlighter` is intentionally **not**
//! re-exported: it is `Rope`-based and emits `gpui::HighlightStyle`
//! (theme-coupled, GPUI-side). The file viewer runs its own GPUI-free
//! highlighting over `tree-sitter-highlight` using the data below.
//!
//! [`language_for_extension`] is the single file-extension → language
//! resolver for every highlighting surface (raw file viewer, diff viewer,
//! agent-chat diff cards). It layers this registry's *capability* over the
//! extension → language *identity* in `daruda_core::language`, which the
//! GPUI-free protocol crate shares.

use gpui::SharedString;

pub use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};

/// Registry name of the no-op language. Its highlight query is empty, so
/// the highlighter parses nothing and the text renders in the editor's
/// base foreground.
pub const PLAIN_LANGUAGE: &str = "text";

/// Resolve a file extension to the canonical registry language name that
/// can actually highlight it, falling back to [`PLAIN_LANGUAGE`].
///
/// The extension → language *identity* lives in
/// [`daruda_core::language`], shared with the GPUI-free protocol crate.
/// This function adds the half that only the app can answer: whether the
/// registry can colour that language.
///
/// The capability check cannot be skipped. `LanguageRegistry::language`
/// never returns `None` — an unknown name falls through to the plain
/// config — and several languages are registered with a grammar but an
/// empty highlight query (their upstream crate exports none). So the
/// answer is non-plain only when a non-empty query backs it.
///
/// Returns the config's own `name`, not the input, so callers always hold
/// a canonical identifier (`"rs"` in → `"rust"` out).
pub fn language_for_extension(ext: &str) -> SharedString {
    daruda_core::language::from_extension(ext)
        .and_then(highlightable_config)
        .map(|config| config.name)
        .unwrap_or_else(|| SharedString::from(PLAIN_LANGUAGE))
}

/// The registry entry for `name`, when it carries a non-empty highlight
/// query. `LanguageRegistry::language` answers *every* name — an unknown one
/// falls through to the plain config — so the query check is what makes
/// `None` mean "this cannot be coloured".
pub fn highlightable_config(name: &str) -> Option<LanguageConfig> {
    LanguageRegistry::singleton()
        .language(name)
        .filter(|config| !config.highlights.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn can_highlight(name: &str) -> bool {
        highlightable_config(name).is_some()
    }

    #[test]
    fn resolves_extensions_the_registry_knows_by_name() {
        // Regression: `.java` fell through a hand-maintained table to the
        // empty string and the raw file viewer opened it as plain text,
        // while `.rs` — the one extension the table listed — highlighted.
        assert_eq!(language_for_extension("java"), "java");
        assert_eq!(language_for_extension("rs"), "rust");
        assert_eq!(language_for_extension("rb"), "ruby");
        assert_eq!(language_for_extension("scala"), "scala");
        assert_eq!(language_for_extension("sql"), "sql");
        assert_eq!(language_for_extension("zig"), "zig");
        assert_eq!(language_for_extension("ex"), "elixir");
        assert_eq!(language_for_extension("scss"), "css");
    }

    #[test]
    fn resolves_aliases_whose_extension_differs_from_the_language() {
        // `.jsx` used to resolve to the literal `"jsx"`, which is not a
        // registered language — the highlighter fell back to plain text.
        assert_eq!(language_for_extension("jsx"), "javascript");
        assert_eq!(language_for_extension("mjs"), "javascript");
        assert_eq!(language_for_extension("cts"), "typescript");
        assert_eq!(language_for_extension("hpp"), "cpp");
        assert_eq!(language_for_extension("h"), "c");
        assert_eq!(language_for_extension("zsh"), "bash");
        assert_eq!(language_for_extension("exs"), "elixir");
    }

    #[test]
    fn extension_case_does_not_matter() {
        assert_eq!(language_for_extension("JAVA"), "java");
        assert_eq!(language_for_extension("Rs"), "rust");
    }

    #[test]
    fn unknown_and_query_less_languages_fall_back_to_plain() {
        assert_eq!(language_for_extension(""), PLAIN_LANGUAGE);
        assert_eq!(language_for_extension("unknown_ext_xyz"), PLAIN_LANGUAGE);
        // Registered grammar, no highlight query shipped upstream — must
        // report plain rather than a name that silently highlights nothing.
        assert_eq!(language_for_extension("swift"), PLAIN_LANGUAGE);
        assert_eq!(language_for_extension("cs"), PLAIN_LANGUAGE);
    }

    /// Languages the shared table names that this registry cannot colour:
    /// the grammar is bundled but its upstream crate ships no highlight
    /// query. Listing them explicitly is what keeps the typo guard below
    /// sharp — a misspelt language name would otherwise look exactly like
    /// one of these and pass unnoticed. When a query lands upstream, delete
    /// the entry; the test then proves the language works.
    const KNOWN_UNSUPPORTED: &[&str] = &["csharp", "graphql", "proto", "swift"];

    #[test]
    fn shared_table_names_are_spelled_correctly() {
        for (extension, name) in daruda_core::language::EXTENSION_LANGUAGES {
            if KNOWN_UNSUPPORTED.contains(name) {
                assert_eq!(
                    language_for_extension(extension),
                    PLAIN_LANGUAGE,
                    "{name:?} is listed as unsupported but now highlights — \
                     drop it from KNOWN_UNSUPPORTED"
                );
                continue;
            }
            assert!(
                can_highlight(name),
                "{extension:?} points at {name:?}, which this registry cannot \
                 highlight — a typo, or a new entry for KNOWN_UNSUPPORTED"
            );
            assert_eq!(
                language_for_extension(extension),
                *name,
                "{extension:?} must resolve to the canonical name it declares"
            );
        }
    }

    #[test]
    fn every_highlightable_language_is_reachable_by_extension() {
        // The completeness guard the registry drives: a grammar added
        // upstream must not land unreachable from the file viewer. If this
        // fails, add the extension(s) for the named language to
        // `daruda_core::language::EXTENSION_LANGUAGES` — or list it below
        // with a reason.
        //
        // Injection-only grammars are never opened as a file of their own:
        // markdown embeds `markdown_inline`, JS/TS embed `jsdoc`.
        const NOT_A_FILE_TYPE: &[&str] = &["markdown_inline", "jsdoc"];

        let reachable: std::collections::HashSet<&str> = daruda_core::language::EXTENSION_LANGUAGES
            .iter()
            .map(|(_, name)| *name)
            .collect();

        let mut checked = 0;
        for name in LanguageRegistry::singleton().languages() {
            if !can_highlight(&name) || NOT_A_FILE_TYPE.contains(&name.as_ref()) {
                continue;
            }
            assert!(
                reachable.contains(name.as_ref()),
                "language {name:?} can highlight but no file extension maps to it"
            );
            checked += 1;
        }
        // Without this the loop would pass vacuously if the registry ever
        // stopped reporting its languages.
        assert!(
            checked > 15,
            "only {checked} languages checked — the registry looks empty"
        );
    }

    #[test]
    fn every_resolved_name_is_highlightable() {
        // The contract the call sites rely on: whatever comes back either
        // highlights or is explicitly plain. Nothing in between.
        for ext in [
            "rs", "java", "py", "go", "ts", "tsx", "jsx", "toml", "json", "yml", "md", "html",
            "css", "sh", "c", "hpp", "rb", "sql", "zig", "scala", "swift", "cs", "nope",
        ] {
            let name = language_for_extension(ext);
            assert!(
                can_highlight(&name) || name == PLAIN_LANGUAGE,
                "{ext} resolved to {name:?}, which neither highlights nor is plain"
            );
        }
    }
}
