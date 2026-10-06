use super::MdColors;
use super::block::code_block_text;
use super::inline::highlight_for;
use super::prose::InlineStyle;
use super::prose::is_openable_markdown_url;
use crate::workspace::main_area::file_view_pane::{HighlightedSpan, VisualRow, VisualRowKind};

fn row(content: &str, spans: Vec<HighlightedSpan>) -> VisualRow {
    VisualRow {
        kind: VisualRowKind::Context,
        line_no_left: String::new(),
        line_no_right: String::new(),
        content: content.to_string(),
        header_context: String::new(),
        spans,
        word_changes: Vec::new(),
    }
}

fn span(text: &str, color: Option<gpui::Hsla>) -> HighlightedSpan {
    HighlightedSpan {
        text: text.to_string(),
        color,
        style: Default::default(),
    }
}

const RED: gpui::Hsla = gpui::Hsla {
    h: 0.0,
    s: 1.0,
    l: 0.5,
    a: 1.0,
};

/// One distinct hue per slot. With every slot the same colour an assertion
/// like `color == colors.link` passes for `text` and `subtle` too, so the
/// contract this fixture exists to pin would go unchecked.
fn colors() -> MdColors {
    let hue = |h: f32| gpui::Hsla {
        h,
        s: 1.0,
        l: 0.5,
        a: 1.0,
    };
    MdColors {
        text: hue(0.0),
        muted: hue(0.1),
        subtle: hue(0.2),
        link: hue(0.3),
        fill: hue(0.4),
        raised: hue(0.5),
        line: hue(0.6),
    }
}

#[test]
fn inline_style_mapping_matches_the_design_contract() {
    let colors = colors();
    let code = highlight_for(
        InlineStyle {
            code: true,
            ..Default::default()
        },
        &colors,
    );
    assert_eq!(code.color, Some(colors.text));
    assert_eq!(code.background_color, Some(colors.fill));
    assert!(code.underline.is_none());

    let strike = highlight_for(
        InlineStyle {
            strikethrough: true,
            ..Default::default()
        },
        &colors,
    );
    assert_eq!(strike.color, Some(colors.subtle));
    assert!(strike.strikethrough.is_some());

    let link = highlight_for(
        InlineStyle {
            link: true,
            ..Default::default()
        },
        &colors,
    );
    assert_eq!(link.color, Some(colors.link));
    assert!(link.underline.is_none());

    // `[`CLAUDE.md`](…)` — code inside a link. Without the link colour it
    // is indistinguishable from ordinary inline code, and underline is off,
    // so nothing marks it clickable. Code still contributes its fill.
    let linked_code = highlight_for(
        InlineStyle {
            code: true,
            link: true,
            ..Default::default()
        },
        &colors,
    );
    assert_eq!(linked_code.color, Some(colors.link));
    assert_eq!(linked_code.background_color, Some(colors.fill));
}

#[test]
fn markdown_links_allow_only_external_safe_schemes() {
    for url in [
        "https://example.com/path",
        "http://localhost:3000",
        "mailto:dev@example.com",
    ] {
        assert!(is_openable_markdown_url(url), "{url}");
    }
    for url in [
        "javascript:alert(1)",
        "file:///tmp/secret",
        "../relative.md",
        "data:text/html,hello",
    ] {
        assert!(!is_openable_markdown_url(url), "{url}");
    }
}

#[test]
fn rows_join_with_newlines_so_one_element_covers_the_block() {
    let (text, highlights) = code_block_text(&[row("fn a() {}", vec![]), row("fn b() {}", vec![])]);
    assert_eq!(text, "fn a() {}\nfn b() {}");
    assert!(
        highlights.is_empty(),
        "an unhighlighted row takes the surface colour"
    );
}

#[test]
fn a_highlight_range_addresses_the_joined_text_not_its_own_row() {
    let (text, highlights) = code_block_text(&[
        row("let x", vec![]),
        row("", vec![span("let", Some(RED)), span(" y", None)]),
    ]);
    assert_eq!(text, "let x\nlet y");
    assert_eq!(highlights.len(), 1);
    assert_eq!(
        highlights[0].0,
        6..9,
        "offset counts the earlier row and its newline"
    );
    assert_eq!(highlights[0].1.color, Some(RED));
}

#[test]
fn an_empty_span_contributes_no_range() {
    let (text, highlights) =
        code_block_text(&[row("", vec![span("", Some(RED)), span("x", None)])]);
    assert_eq!(text, "x");
    assert!(highlights.is_empty());
}

#[test]
fn every_highlight_lands_on_a_char_boundary() {
    // `StyledText::with_highlights` debug-asserts this, and multi-byte
    // source is ordinary in a code block.
    let (text, highlights) = code_block_text(&[row(
        "",
        vec![span("사과", Some(RED)), span("=1", Some(RED))],
    )]);
    for (range, _) in &highlights {
        assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
    }
}

use crate::ui::theme::{
    apply_ui_theme, contrast_ratio, current, file_viewer_pane_bg, init_if_missing,
    set_agent_chat_bg, set_agent_chat_fg,
};

/// Every colour this preview paints comes from the surface it is painted
/// on. It used to come from the UI theme, and the file viewer's surface
/// mirrors the *terminal* palette — with `ui_preset` and `terminal_preset`
/// being independent config keys, a light UI theme over a dark terminal put
/// `#2d2d2d` prose on a near-black pane.
#[gpui::test]
fn the_preview_reads_on_the_pane_it_is_painted_on(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        init_if_missing(cx);
        // The combination the defect needed: light chrome, dark terminal.
        apply_ui_theme("daruda_light", cx);
        let mut ui_source_failures = 0;

        for preset in daruda_config::theme_presets::PRESETS {
            let Some(colors) = daruda_config::theme_presets::colors_for_preset(preset.name) else {
                continue;
            };
            let (bg, fg) = (colors.background, colors.foreground);
            set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
            set_agent_chat_fg(cx, fg.r, fg.g, fg.b);
            let pane = file_viewer_pane_bg(cx);
            let t = MdColors::for_pane(cx);

            let body = contrast_ratio(t.text, pane);
            assert!(
                body >= 4.5,
                "{}: body prose measures {body:.2}:1",
                preset.name
            );
            let quote = contrast_ratio(t.muted, pane);
            assert!(
                quote >= 3.0,
                "{}: quote prose measures {quote:.2}:1",
                preset.name
            );
            // Fills and lines have to separate from the surface at all —
            // they are decoration, so this is a visibility floor, not
            // DESIGN.md's 3:1 affordance floor.
            for (what, color) in [("fill", t.fill), ("raised", t.raised), ("line", t.line)] {
                let ratio = contrast_ratio(color, pane);
                assert!(
                    ratio > 1.0,
                    "{}: the {what} does not separate from the pane",
                    preset.name
                );
            }

            // What the UI theme would have painted on this same surface.
            if contrast_ratio(current(cx).text_body, pane) < 4.5 {
                ui_source_failures += 1;
            }
        }

        assert!(
            ui_source_failures > 0,
            "if the UI theme's body tone were legible on every terminal preset the \
             pane-derived colours would be unnecessary"
        );
        apply_ui_theme("daruda_dark", cx);
    });
}
