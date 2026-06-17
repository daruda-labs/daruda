//! Re-export of `gpui_component`'s tree-sitter language registry.
//!
//! Application code must never import `gpui_component::*` directly
//! (see `crate::ui` module docs); this is the sanctioned access point
//! for the syntax-highlighting language data.
//!
//! Only the GPUI-free *data* types are re-exported: [`LanguageRegistry`]
//! (a process-wide singleton mapping language name/extension to a
//! [`LanguageConfig`]) and [`LanguageConfig`] itself, which carries the
//! `tree_sitter::Language` and the bundled `highlights` query string.
//! The registry resolves short names too (`"rs"` → Rust, `"js"` →
//! JavaScript), so the file viewer can look a language up by file
//! extension. `LanguageRegistry::language()` takes no `App`/`Window`, so
//! it is safe to call on `background_executor`.
//!
//! The gpui_component `SyntaxHighlighter` is intentionally **not**
//! re-exported: it is `Rope`-based and emits `gpui::HighlightStyle`
//! (theme-coupled, GPUI-side). The file viewer runs its own GPUI-free
//! highlighting over `tree-sitter-highlight` using the data below.

pub use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};
