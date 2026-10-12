use super::*;
use crate::text::node::{InlineNode, TextMark};
use gpui::{AppContext as _, TestAppContext, VisualTestContext};

fn with_window(cx: &mut TestAppContext, test: impl FnOnce(&mut Window, &mut App)) {
    cx.update(crate::init);
    let window = cx.add_window(|_, _| gpui::Empty);
    VisualTestContext::from_window(window.into(), cx).update(test);
}

fn style() -> TextStyle {
    TextStyle {
        font_family: ".ZedMono".into(),
        font_size: px(16.).into(),
        ..Default::default()
    }
}

#[gpui::test]
fn hashes_are_unbreakable_but_prose_and_cjk_can_shrink(cx: &mut TestAppContext) {
    with_window(cx, |window, _| {
        let hash = text_widths("a1b2c3d4", &[], &style(), window);
        let prose = text_widths(
            "short words with plenty of room to wrap",
            &[],
            &style(),
            window,
        );
        let cjk = text_widths(
            "\u{d55c}\u{ae00}\u{c124}\u{ba85}\u{6587}\u{5b57}",
            &[],
            &style(),
            window,
        );
        assert!(hash.min > px(0.));
        assert_eq!(hash.min, hash.max);
        assert!(prose.min < prose.max / 3.);
        assert!(cjk.min < cjk.max / 2.);
    });
}

#[gpui::test]
fn identifier_punctuation_stays_unbreakable_when_paint_cannot_wrap_it(cx: &mut TestAppContext) {
    with_window(cx, |window, _| {
        for identifier in ["release-candidate", "Self::new", "user@example.com"] {
            let widths = text_widths(identifier, &[], &style(), window);
            assert_eq!(widths.min, widths.max, "identifier={identifier}");
        }
    });
}

#[gpui::test]
fn forced_breaks_and_nonbreaking_spaces_have_distinct_widths(cx: &mut TestAppContext) {
    with_window(cx, |window, _| {
        let single = text_widths("abcd", &[], &style(), window);
        let multiline = text_widths("ab\nabcd", &[], &style(), window);
        assert_eq!(single.max, multiline.max);
        assert_eq!(single.min, multiline.min);
        let styled = text_widths(
            "ab\ncd",
            &[(0..2, HighlightStyle::default())],
            &style(),
            window,
        );
        assert_eq!(styled.max, text_widths("ab", &[], &style(), window).max);
        let joined = text_widths("ab\u{a0}cd", &[], &style(), window);
        assert_eq!(joined.min, joined.max);
        let separated = text_widths("ab cd", &[], &style(), window);
        assert!(separated.min < joined.min);
        let empty = text_widths("", &[], &style(), window);
        assert_eq!(empty.min, px(0.));
        assert_eq!(empty.max, px(0.));
    });
}

#[gpui::test]
fn formatting_boundaries_do_not_split_an_identifier(cx: &mut TestAppContext) {
    with_window(cx, |window, cx| {
        let mut paragraph = Paragraph::new("abcd".into());
        paragraph.push(InlineNode::new("efgh").marks(vec![(0..4, TextMark::default().bold())]));
        let widths = super::paragraph(
            &paragraph,
            NodeRenderOptions::default(),
            &style(),
            window,
            cx,
        );
        assert_eq!(widths.min, widths.max);
        assert!(widths.min > text_widths("abcd", &[], &style(), window).max);
    });
}

#[gpui::test]
fn measurement_uses_the_same_styled_runs_as_paint(cx: &mut TestAppContext) {
    with_window(cx, |window, cx| {
        let mark = TextMark::default().bold().italic().code();
        let mut paragraph = Paragraph::default();
        paragraph.push(InlineNode::new("MMMM").marks(vec![(0..4, mark.clone())]));
        let options = NodeRenderOptions::default();
        let measured = super::paragraph(&paragraph, options, &style(), window, cx);
        let highlights = vec![(0..4, mark.highlight(options, cx))];
        let runs = Inline::text_runs("MMMM", &highlights, &style());
        assert_eq!(runs[0].font.weight, FontWeight::BOLD);
        assert_eq!(runs[0].font.style, gpui::FontStyle::Italic);
        assert_eq!(
            measured.max,
            window
                .text_system()
                .layout_line("MMMM", px(16.), &runs, None)
                .width
        );
        let large = TextStyle {
            font_size: px(32.).into(),
            ..style()
        };
        assert_eq!(
            super::paragraph(&paragraph, options, &large, window, cx).max,
            measured.max * 2.
        );
    });
}
