//! Syntax highlighting palettes shared by the raw editor and the diff view.

mod dark;
mod light;

use gpui::Hsla;
use gpui_component::highlighter::{SyntaxColors, ThemeStyle};

// The theme-independent half — what a token *is* — lives with the content
// pipeline so highlighted rows can be stored without a colour in them.
pub use daruda_content::syntax::{SyntaxBucket, TokenStyle, bucket_for_capture};

/// 24-bit hex → `Hsla`. Hex literals live here (the designated colour
/// home, G4-exempt), never at the call site.
fn base16(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Semantic syntax palette — one resolved [`SyntaxPalette`]'s colours,
/// shared by the raw editor (via [`editor_syntax_colors_of`], installed
/// into `gpui_component`'s `highlight_theme`) and the diff view (via
/// [`syntax_color_of`]). Fields are semantic token buckets.
#[derive(Clone, Copy)]
pub struct SyntaxTheme {
    /// purple — keywords.
    pub keyword: Hsla,
    /// blue — functions / titles.
    pub function: Hsla,
    /// yellow — types, enums, constructors, labels, preproc, embedded.
    pub type_: Hsla,
    /// orange — constants, booleans, numbers, attributes, variants, link URIs.
    pub constant: Hsla,
    /// green — strings, literal text.
    pub string: Hsla,
    /// cyan — string escapes / regex / special symbols.
    pub string_special: Hsla,
    /// red — tags, special variables, link text.
    pub tag: Hsla,
    /// brown — doctype tags.
    pub tag_doctype: Hsla,
    /// gray — comments, hints, predictive text.
    pub comment: Hsla,
    /// default foreground (variables, operators, punctuation, …).
    pub default: Hsla,
    /// Non-color channel (R1) for keywords — bold to carry structure
    /// without relying on chroma / CVD-robust.
    pub keyword_style: TokenStyle,
    /// Non-color channel for string escapes / regex / special symbols —
    /// distinguishes them from plain strings even at low chroma.
    pub string_special_style: TokenStyle,
    /// Non-color channel for comments — italic to signal "noise".
    pub comment_style: TokenStyle,
}

/// Selectable syntax palette — chosen independently of the brand theme
/// (background / accent). Resolved from `config.file_viewer.syntax_theme`
/// via [`SyntaxPalette::from_config_name`]; the single source of truth for
/// which colours the raw editor and the diff view both render with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyntaxPalette {
    /// Daruda's own palette — the readability-tuned default (recommended).
    #[default]
    Daruda,
    OneDark,
    TokyoNight,
    CatppuccinMocha,
    Dracula,
    GitHubDark,
    MaterialPalenight,
    Monokai,
    Nord,
    GruvboxDark,
    SolarizedDark,
    AyuMirage,
    NightOwl,
    Darcula,
}

impl SyntaxPalette {
    /// Resolve a config string to a palette. Unknown / legacy names
    /// (including the old `base16-*` slots) fall back to the recommended
    /// [`SyntaxPalette::Daruda`] — a normal fallback, not an error.
    pub fn from_config_name(name: &str) -> Self {
        match name {
            "one-dark" => Self::OneDark,
            "tokyo-night" => Self::TokyoNight,
            "catppuccin-mocha" => Self::CatppuccinMocha,
            "dracula" => Self::Dracula,
            "github-dark" => Self::GitHubDark,
            "material-palenight" => Self::MaterialPalenight,
            "monokai" => Self::Monokai,
            "nord" => Self::Nord,
            "gruvbox-dark" => Self::GruvboxDark,
            "solarized-dark" => Self::SolarizedDark,
            "ayu-mirage" => Self::AyuMirage,
            "night-owl" => Self::NightOwl,
            "darcula" => Self::Darcula,
            _ => Self::Daruda,
        }
    }

    /// Canonical config slug — inverse of [`SyntaxPalette::from_config_name`].
    /// Lets the settings dropdown resolve a stored (possibly legacy) value to
    /// the slug that is actually selected, so the effective palette always
    /// shows as the active option.
    pub fn config_name(self) -> &'static str {
        match self {
            Self::Daruda => "daruda",
            Self::OneDark => "one-dark",
            Self::TokyoNight => "tokyo-night",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::Dracula => "dracula",
            Self::GitHubDark => "github-dark",
            Self::MaterialPalenight => "material-palenight",
            Self::Monokai => "monokai",
            Self::Nord => "nord",
            Self::GruvboxDark => "gruvbox-dark",
            Self::SolarizedDark => "solarized-dark",
            Self::AyuMirage => "ayu-mirage",
            Self::NightOwl => "night-owl",
            Self::Darcula => "darcula",
        }
    }
}

impl SyntaxTheme {
    /// Foreground colour for a bucket.
    pub fn color(&self, bucket: SyntaxBucket) -> Hsla {
        use SyntaxBucket::*;
        match bucket {
            Keyword => self.keyword,
            Function => self.function,
            Type => self.type_,
            Constant => self.constant,
            String => self.string,
            StringSpecial => self.string_special,
            Tag => self.tag,
            TagDoctype => self.tag_doctype,
            Comment => self.comment,
            Default => self.default,
        }
    }

    /// Non-color channel (bold/italic) for a bucket. Only keyword /
    /// string_special / comment carry one; the rest are plain.
    pub fn style(&self, bucket: SyntaxBucket) -> TokenStyle {
        use SyntaxBucket::*;
        match bucket {
            Keyword => self.keyword_style,
            StringSpecial => self.string_special_style,
            Comment => self.comment_style,
            _ => TokenStyle::default(),
        }
    }

    /// Foreground colour for a capture name — [`bucket_for_capture`] +
    /// [`SyntaxTheme::color`]. Used by the diff view's own highlighter.
    pub fn color_for(&self, capture: &str) -> Hsla {
        self.color(bucket_for_capture(capture))
    }

    /// Non-color channel for a capture name.
    pub fn style_for(&self, capture: &str) -> TokenStyle {
        self.style(bucket_for_capture(capture))
    }
}

/// The active syntax palette (`base16-ocean.dark`). One source feeds both
/// highlighting paths — change a colour here and the editor and diff
/// views move together.
pub fn syntax_theme() -> SyntaxTheme {
    syntax_theme_of(SyntaxPalette::Daruda, false)
}

/// The semantic syntax palette for `palette` at the editor's lightness.
/// Hex literals live here (the designated colour home, G4-exempt), never at
/// the call site. One source feeds both highlighting paths — the raw editor
/// (via [`editor_syntax_colors_of`]) and the diff view (via
/// [`syntax_color_of`]). When `is_light` the palette's light variant is used
/// (families without one fall back to Daruda Light) so syntax stays legible
/// on a light editor background.
pub fn syntax_theme_of(palette: SyntaxPalette, is_light: bool) -> SyntaxTheme {
    if is_light {
        light::light_syntax_theme(palette)
    } else {
        dark::dark_syntax_theme(palette)
    }
}

/// Foreground colour for a tree-sitter highlight capture name. Unrecognised
/// captures (and the empty string) fall back to the default editor
/// foreground, so every token gets an explicit foreground. Used by the diff
/// view's own highlighter.
pub fn syntax_color(capture: &str) -> Hsla {
    syntax_color_of(SyntaxPalette::Daruda, false, capture)
}

/// Foreground colour for a capture in `palette` at the editor's lightness.
/// Thin wrapper over [`SyntaxTheme::color_for`] for call sites that hold only
/// a palette.
pub fn syntax_color_of(palette: SyntaxPalette, is_light: bool, capture: &str) -> Hsla {
    syntax_theme_of(palette, is_light).color_for(capture)
}

/// Build `gpui_component`'s per-capture [`SyntaxColors`] from
/// [`syntax_theme`] so the raw editor highlights with the exact colours
/// the diff view uses. The grouping mirrors [`syntax_color`] one-to-one;
/// every field is set explicitly so a future upstream field addition
/// fails to compile here rather than silently diverging.
pub fn editor_syntax_colors() -> SyntaxColors {
    editor_syntax_colors_of(SyntaxPalette::Daruda, false)
}

/// Build `gpui_component`'s per-capture [`SyntaxColors`] for `palette` at the
/// editor's lightness. The styled buckets (keyword / string_special /
/// comment) carry the palette's bold/italic channel; every other bucket is
/// colour-only.
pub fn editor_syntax_colors_of(palette: SyntaxPalette, is_light: bool) -> SyntaxColors {
    use SyntaxBucket::*;
    let t = syntax_theme_of(palette, is_light);
    // Colour + non-color channel for a bucket, as one `ThemeStyle`.
    let b = |bucket: SyntaxBucket| {
        let st = t.style(bucket);
        let mut ts = ThemeStyle::new(t.color(bucket));
        if st.bold {
            ts = ts.bold();
        }
        if st.italic {
            ts = ts.italic();
        }
        Some(ts)
    };
    SyntaxColors {
        keyword: b(Keyword),
        function: b(Function),
        title: b(Function),
        type_: b(Type),
        enum_: b(Type),
        constructor: b(Type),
        label: b(Type),
        preproc: b(Type),
        embedded: b(Type),
        constant: b(Constant),
        boolean: b(Constant),
        number: b(Constant),
        attribute: b(Constant),
        variant: b(Constant),
        link_uri: b(Constant),
        string: b(String),
        text_literal: b(String),
        string_escape: b(StringSpecial),
        string_regex: b(StringSpecial),
        string_special: b(StringSpecial),
        string_special_symbol: b(StringSpecial),
        tag: b(Tag),
        variable_special: b(Tag),
        link_text: b(Tag),
        tag_doctype: b(TagDoctype),
        comment: b(Comment),
        comment_doc: b(Comment),
        hint: b(Comment),
        predictive: b(Comment),
        variable: b(Default),
        property: b(Default),
        operator: b(Default),
        punctuation: b(Default),
        punctuation_bracket: b(Default),
        punctuation_delimiter: b(Default),
        punctuation_list_marker: b(Default),
        punctuation_special: b(Default),
        emphasis: b(Default),
        emphasis_strong: b(Default),
        primary: b(Default),
    }
}

#[cfg(test)]
mod tests;
