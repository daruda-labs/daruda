//! Pure compilation of Markdown inline spans into shaped-text parts.
//!
//! Also the home of [`is_openable_markdown_url`], the host's link policy: the
//! compiler is where a link becomes a clickable range *and* a coloured run, so
//! deciding here is what keeps those two from disagreeing.

use std::ops::Range;

use crate::workspace::main_area::file_view_pane::markdown_viewer::MdSpan;
use crate::workspace::main_area::file_view_pane::visual::RasterImage;

/// Whether this host will actually open `url` when the link is clicked.
///
/// The one place that decision is made. It feeds both the clickable range
/// *and* the link colouring, because the two disagreeing is what produces a
/// false affordance: `underline` is off by design (DESIGN.md), so colour is
/// the only cue a span is a link, and colouring one that cannot be opened
/// promises a click that does nothing.
///
/// Relative links (`[`CLAUDE.md`](./CLAUDE.md)`, the common form in this
/// repo's own docs) are therefore rendered as ordinary prose. Resolving them
/// against the viewed file and opening them in the file viewer would be the
/// better answer, but that is a feature, not a rendering rule.
pub(super) fn is_openable_markdown_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: bool,
    pub strikethrough: bool,
    pub footnote: bool,
    pub html: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleRun {
    pub range: Range<usize>,
    pub style: InlineStyle,
}

#[derive(Debug, Default)]
pub(super) struct CompiledText {
    pub text: String,
    pub style_runs: Vec<StyleRun>,
    pub link_ranges: Vec<Range<usize>>,
    pub link_urls: Vec<String>,
}

pub(super) struct InlineImage<'a> {
    pub alt: &'a str,
    pub raster: Option<&'a RasterImage>,
    pub link_url: Option<&'a str>,
}

pub(super) enum ProsePart<'a> {
    Text(CompiledText),
    Image(InlineImage<'a>),
}

#[derive(Clone, Copy, Default)]
struct ActiveStyle<'a> {
    inline: InlineStyle,
    link: Option<(usize, &'a str)>,
}

#[derive(Default)]
struct Compiler<'a> {
    parts: Vec<ProsePart<'a>>,
    pending: CompiledText,
    next_link_id: usize,
    last_link_id: Option<usize>,
}

impl<'a> Compiler<'a> {
    fn compile(mut self, spans: &'a [MdSpan]) -> Vec<ProsePart<'a>> {
        self.walk(spans, ActiveStyle::default());
        self.flush_text();
        self.parts
    }

    fn walk(&mut self, spans: &'a [MdSpan], active: ActiveStyle<'a>) {
        for span in spans {
            match span {
                MdSpan::Text(text) => self.append(text, active),
                MdSpan::Bold(children) => {
                    let mut nested = active;
                    nested.inline.bold = true;
                    self.walk(children, nested);
                }
                MdSpan::Italic(children) => {
                    let mut nested = active;
                    nested.inline.italic = true;
                    self.walk(children, nested);
                }
                MdSpan::Code(code) => {
                    let mut nested = active;
                    nested.inline.code = true;
                    self.append(code, nested);
                }
                MdSpan::Link { children, url } => {
                    let mut nested = active;
                    // A link this host cannot open carries neither the colour
                    // nor the click — see [`is_openable_markdown_url`]. Its
                    // children still render, as the plain text they are.
                    if is_openable_markdown_url(url) {
                        nested.inline.link = true;
                        nested.link = Some((self.next_link_id, url));
                        self.next_link_id += 1;
                    }
                    self.walk(children, nested);
                }
                MdSpan::Strikethrough(children) => {
                    let mut nested = active;
                    nested.inline.strikethrough = true;
                    self.walk(children, nested);
                }
                MdSpan::SoftBreak => self.append(" ", active),
                MdSpan::HardBreak => self.append("\n", active),
                MdSpan::ParagraphBreak => {
                    // `render_md_prose` splits the run here, so reaching this is
                    // a bug. The assert is debug-only, so release still needs a
                    // separator — without one the two paragraphs' words jam
                    // together.
                    debug_assert!(false, "paragraph breaks must be split before compile_prose");
                    self.append("\n", active);
                }
                MdSpan::Footnote(label) => {
                    let mut nested = active;
                    nested.inline.footnote = true;
                    self.append(&format!("[^{label}]"), nested);
                }
                MdSpan::Html(html) => {
                    let mut nested = active;
                    nested.inline.html = true;
                    self.append(html, nested);
                }
                MdSpan::Image {
                    url, alt, raster, ..
                } => {
                    self.flush_text();
                    self.parts.push(ProsePart::Image(InlineImage {
                        alt: if alt.is_empty() { url } else { alt },
                        raster: raster.as_ref(),
                        link_url: active.link.map(|(_, url)| url),
                    }));
                }
            }
        }
    }

    fn append(&mut self, text: &str, active: ActiveStyle<'a>) {
        if text.is_empty() {
            return;
        }

        let start = self.pending.text.len();
        self.pending.text.push_str(text);
        let end = self.pending.text.len();

        if let Some(last) = self.pending.style_runs.last_mut()
            && last.range.end == start
            && last.style == active.inline
        {
            last.range.end = end;
        } else {
            self.pending.style_runs.push(StyleRun {
                range: start..end,
                style: active.inline,
            });
        }

        if let Some((link_id, url)) = active.link {
            let extends_last_link = self
                .pending
                .link_ranges
                .last()
                .zip(self.pending.link_urls.last())
                .is_some_and(|(range, last_url)| {
                    range.end == start && last_url == url && self.last_link_id == Some(link_id)
                });
            if extends_last_link {
                self.pending.link_ranges.last_mut().unwrap().end = end;
            } else {
                self.pending.link_ranges.push(start..end);
                self.pending.link_urls.push(url.to_owned());
            }
            self.last_link_id = Some(link_id);
        } else {
            self.last_link_id = None;
        }
    }

    fn flush_text(&mut self) {
        if self.pending.text.is_empty() {
            return;
        }
        self.parts
            .push(ProsePart::Text(std::mem::take(&mut self.pending)));
        self.last_link_id = None;
    }
}

pub(super) fn compile_prose(spans: &[MdSpan]) -> Vec<ProsePart<'_>> {
    Compiler::default().compile(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_part<'parts, 'spans>(parts: &'parts [ProsePart<'spans>]) -> &'parts CompiledText {
        let [ProsePart::Text(text)] = parts else {
            panic!("expected one text part");
        };
        text
    }

    fn style_for<'a>(text: &'a CompiledText, needle: &str) -> &'a InlineStyle {
        let start = text.text.find(needle).expect("text contains needle");
        &text
            .style_runs
            .iter()
            .find(|run| run.range.start <= start && start < run.range.end)
            .expect("style covers needle")
            .style
    }

    #[test]
    fn nested_inline_styles_compile_into_one_text_part() {
        let spans = vec![
            MdSpan::Text("plain ".into()),
            MdSpan::Bold(vec![
                MdSpan::Text("bold ".into()),
                MdSpan::Link {
                    children: vec![
                        MdSpan::Text("linked ".into()),
                        MdSpan::Italic(vec![MdSpan::Text("nested".into())]),
                    ],
                    url: "https://example.com".into(),
                },
            ]),
            MdSpan::Text(" ".into()),
            MdSpan::Code("code".into()),
            MdSpan::Text(" ".into()),
            MdSpan::Strikethrough(vec![MdSpan::Text("strike".into())]),
        ];

        let parts = compile_prose(&spans);
        let compiled = text_part(&parts);
        assert_eq!(compiled.text, "plain bold linked nested code strike");
        assert!(style_for(compiled, "bold").bold);
        let nested = style_for(compiled, "nested");
        assert!(nested.bold && nested.italic && nested.link);
        assert!(style_for(compiled, "code").code);
        assert!(style_for(compiled, "strike").strikethrough);
        assert_eq!(compiled.link_ranges, vec![11..24]);
        assert_eq!(compiled.link_urls, vec!["https://example.com"]);
    }

    #[test]
    fn rendered_ranges_stay_on_utf8_boundaries() {
        let spans = vec![MdSpan::Link {
            children: vec![MdSpan::Text("한글🙂".into())],
            url: "https://example.com/ko".into(),
        }];
        let parts = compile_prose(&spans);
        let compiled = text_part(&parts);

        assert_eq!(compiled.link_ranges, vec![0..compiled.text.len()]);
        for range in compiled
            .style_runs
            .iter()
            .map(|run| &run.range)
            .chain(compiled.link_ranges.iter())
        {
            assert!(compiled.text.is_char_boundary(range.start));
            assert!(compiled.text.is_char_boundary(range.end));
        }
    }

    #[test]
    fn adjacent_links_keep_their_url_indices() {
        let spans = vec![
            MdSpan::Link {
                children: vec![MdSpan::Text("first".into())],
                url: "https://example.com/first".into(),
            },
            MdSpan::Link {
                children: vec![MdSpan::Text("second".into())],
                url: "https://example.com/second".into(),
            },
        ];
        let parts = compile_prose(&spans);
        let compiled = text_part(&parts);

        assert_eq!(compiled.text, "firstsecond");
        assert_eq!(compiled.link_ranges, vec![0..5, 5..11]);
        assert_eq!(
            compiled.link_urls,
            vec!["https://example.com/first", "https://example.com/second"]
        );

        // Two links to the *same* URL are still two ranges. Merging them by URL
        // would pass the assertions above, so this is the case that pins the
        // per-link identity the compiler tracks.
        let same = [
            MdSpan::Link {
                children: vec![MdSpan::Text("one".into())],
                url: "https://example.com/x".into(),
            },
            MdSpan::Link {
                children: vec![MdSpan::Text("two".into())],
                url: "https://example.com/x".into(),
            },
        ];
        let parts = compile_prose(&same);
        let compiled = text_part(&parts);
        assert_eq!(compiled.link_ranges, vec![0..3, 3..6]);
        assert_eq!(
            compiled.link_urls,
            vec!["https://example.com/x", "https://example.com/x"]
        );
    }

    #[test]
    fn breaks_are_absorbed_into_text() {
        let spans = vec![
            MdSpan::Text("soft".into()),
            MdSpan::SoftBreak,
            MdSpan::Text("break".into()),
            MdSpan::HardBreak,
            MdSpan::Text("next".into()),
        ];
        let parts = compile_prose(&spans);
        let compiled = text_part(&parts);
        assert_eq!(compiled.text, "soft break\nnext");
    }

    #[test]
    fn only_images_flush_text() {
        let spans = vec![
            MdSpan::Text("before".into()),
            MdSpan::Image {
                url: "missing.png".into(),
                alt: "fallback".into(),
                raster: None,
            },
            MdSpan::Bold(vec![MdSpan::Text("after".into())]),
        ];
        let parts = compile_prose(&spans);

        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], ProsePart::Text(text) if text.text == "before"));
        assert!(
            matches!(&parts[1], ProsePart::Image(image) if image.alt == "fallback" && image.raster.is_none() && image.link_url.is_none())
        );
        assert!(matches!(&parts[2], ProsePart::Text(text) if text.text == "after"));
    }

    /// The colour and the click come from the same decision, so a link this
    /// host cannot open loses both. Only the colour tells the reader a span is
    /// a link — underline is off by design — so keeping it on an unopenable
    /// link would promise a click that goes nowhere.
    #[test]
    fn an_unopenable_link_is_neither_coloured_nor_clickable() {
        let spans = vec![
            MdSpan::Link {
                children: vec![MdSpan::Code("CLAUDE.md".into())],
                url: "./CLAUDE.md".into(),
            },
            MdSpan::Text(" and ".into()),
            MdSpan::Link {
                children: vec![MdSpan::Text("docs".into())],
                url: "https://example.com".into(),
            },
        ];
        let parts = compile_prose(&spans);
        let compiled = text_part(&parts);

        let relative = style_for(compiled, "CLAUDE.md");
        assert!(!relative.link, "a relative link is not a link to this host");
        assert!(relative.code, "it still renders as the code span it is");
        assert!(style_for(compiled, "docs").link);
        assert_eq!(compiled.link_urls, vec!["https://example.com"]);
        assert_eq!(compiled.link_ranges, vec![14..18]);
    }

    /// The same decision reaches the image path, which builds its own click
    /// target from `link_url` rather than from a text range.
    #[test]
    fn an_unopenable_link_around_an_image_is_dropped() {
        let spans = vec![MdSpan::Link {
            children: vec![MdSpan::Image {
                url: "thumb.png".into(),
                alt: "thumbnail".into(),
                raster: None,
            }],
            url: "./doc.md".into(),
        }];

        let parts = compile_prose(&spans);
        let [ProsePart::Image(image)] = parts.as_slice() else {
            panic!("expected one image part");
        };
        assert_eq!(image.link_url, None);
    }

    #[test]
    fn images_preserve_the_surrounding_link() {
        let spans = vec![MdSpan::Link {
            children: vec![MdSpan::Image {
                url: "thumb.png".into(),
                alt: "thumbnail".into(),
                raster: None,
            }],
            url: "https://example.com/full".into(),
        }];

        let parts = compile_prose(&spans);
        let [ProsePart::Image(image)] = parts.as_slice() else {
            panic!("expected one image part");
        };
        assert_eq!(image.link_url, Some("https://example.com/full"));
    }

    #[test]
    fn style_runs_are_sorted_non_overlapping_and_in_bounds() {
        let spans = vec![
            MdSpan::Text("a".into()),
            MdSpan::Bold(vec![MdSpan::Text("한".into())]),
            MdSpan::Italic(vec![MdSpan::Text("🙂".into())]),
        ];
        let parts = compile_prose(&spans);
        let compiled = text_part(&parts);
        let mut end = 0;
        for run in &compiled.style_runs {
            assert_eq!(run.range.start, end);
            assert!(run.range.end <= compiled.text.len());
            end = run.range.end;
        }
        assert_eq!(end, compiled.text.len());
    }

    /// Debug builds catch the caller; the assertion is all there is to check,
    /// since it fires before the separator the release build falls back to.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "paragraph breaks must be split before compile_prose")]
    fn paragraph_breaks_are_rejected() {
        let _ = compile_prose(&[MdSpan::ParagraphBreak]);
    }

    /// The other half, reachable only under `cargo test --release`: with the
    /// assertion compiled out the separator is what stops the two paragraphs'
    /// words from running together, so it needs a test of its own rather than
    /// sitting behind a `should_panic` that never reaches it.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_paragraph_break_still_separates_words_in_release() {
        let spans = [
            MdSpan::Text("before".into()),
            MdSpan::ParagraphBreak,
            MdSpan::Text("after".into()),
        ];
        let parts = compile_prose(&spans);
        assert_eq!(text_part(&parts).text, "before\nafter");
    }
}
