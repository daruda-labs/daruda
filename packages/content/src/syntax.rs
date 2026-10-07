//! What a highlighted token *is*, apart from how it is painted: the semantic
//! bucket a tree-sitter capture maps to, and the non-colour channel a
//! palette may give it. A theme turns a bucket into a colour at paint time,
//! so a stored row never carries one.

/// Semantic colour bucket — the unit a tree-sitter capture maps to.
/// [`bucket_for_capture`] is the single place the capture grouping lives;
/// every consumer (colour, non-color channel, editor `SyntaxColors`) reads
/// through it so the grouping can't drift between paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxBucket {
    Keyword,
    Function,
    Type,
    Constant,
    String,
    StringSpecial,
    Tag,
    TagDoctype,
    Comment,
    Default,
}

/// Map a tree-sitter highlight capture name onto its [`SyntaxBucket`].
///
/// A dotted capture that doesn't match exactly falls back to its first
/// segment (`function.method` → `function`, `keyword.control` →
/// `keyword`), mirroring the gpui_component editor's
/// `SyntaxColors::style` resolution (`registry.rs`). Without this the
/// raw editor coloured `function.method` / `type.builtin` via the base
/// field while the diff view dropped them to `Default` — the two views
/// disagreed on methods, qualified types, and keyword sub-kinds.
/// Unrecognised captures (and the empty string) resolve to
/// [`SyntaxBucket::Default`], so every token gets an explicit bucket.
pub fn bucket_for_capture(capture: &str) -> SyntaxBucket {
    use SyntaxBucket::*;
    match capture {
        "keyword" => Keyword,
        "function" | "title" => Function,
        "type" | "enum" | "constructor" | "label" | "preproc" | "embedded" => Type,
        "constant" | "boolean" | "number" | "attribute" | "variant" | "link_uri" => Constant,
        "string" | "text.literal" => String,
        "string.escape" | "string.regex" | "string.special" | "string.special.symbol" => {
            StringSpecial
        }
        "tag" | "variable.special" | "link_text" => Tag,
        "tag.doctype" => TagDoctype,
        "comment" | "comment.doc" | "hint" | "predictive" => Comment,
        _ => match capture.split_once('.') {
            Some((prefix, _)) => bucket_for_capture(prefix),
            None => Default,
        },
    }
}

/// A token's non-color rendering channel. `Default` = plain (no
/// weight / style override). Lets a palette carry bold/italic alongside
/// its colours so figure/ground survives low chroma and colour-vision
/// deficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenStyle {
    pub bold: bool,
    pub italic: bool,
}

impl TokenStyle {
    pub const PLAIN: Self = Self {
        bold: false,
        italic: false,
    };
    pub const BOLD: Self = Self {
        bold: true,
        italic: false,
    };
    pub const ITALIC: Self = Self {
        bold: false,
        italic: true,
    };
}
