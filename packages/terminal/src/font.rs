/// Build a terminal font with the given primary family. Fallback chain
/// and font features are the same as `default_terminal_font()`.
pub fn terminal_font_with_family(family: &str) -> gpui::Font {
    let fallbacks = terminal_font_fallbacks();
    let family: gpui::SharedString = family.to_string().into();
    let mut font = gpui::font(family);
    font.fallbacks = Some(fallbacks);
    font
}

pub fn default_terminal_font() -> gpui::Font {
    // macOS primary mirrors iTerm2's factory default (`Monaco 12` in
    // `DefaultBookmark.plist`) so a fresh daruda profile lands on the
    // same glyph shapes a returning iTerm2 user expects.
    let family = if cfg!(target_os = "macos") {
        "Monaco"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    };

    let fallbacks = terminal_font_fallbacks();
    let mut font = gpui::font(family);
    font.fallbacks = Some(fallbacks);
    font
}

fn terminal_font_fallbacks() -> gpui::FontFallbacks {
    gpui::FontFallbacks::from_fonts(vec![
        "SF Mono".to_string(),
        "Menlo".to_string(),
        "Monaco".to_string(),
        "Consolas".to_string(),
        "Cascadia Mono".to_string(),
        "DejaVu Sans Mono".to_string(),
        "Noto Sans Mono".to_string(),
        "JetBrains Mono".to_string(),
        "Fira Mono".to_string(),
        "Sarasa Mono SC".to_string(),
        "Sarasa Term SC".to_string(),
        "Sarasa Mono J".to_string(),
        "Noto Sans Mono CJK SC".to_string(),
        "Noto Sans Mono CJK JP".to_string(),
        "Source Han Mono SC".to_string(),
        "WenQuanYi Zen Hei Mono".to_string(),
        "Apple Color Emoji".to_string(),
        "Noto Color Emoji".to_string(),
        "Segoe UI Emoji".to_string(),
    ])
}

pub fn default_terminal_font_features() -> gpui::FontFeatures {
    use std::sync::Arc;
    gpui::FontFeatures(Arc::new(vec![
        ("calt".to_string(), 0),
        ("liga".to_string(), 0),
        ("kern".to_string(), 0),
    ]))
}
