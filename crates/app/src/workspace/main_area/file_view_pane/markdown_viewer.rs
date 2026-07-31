//! Markdown parser and IR types for the file viewer preview mode.
//!
//! `parse_markdown` converts a raw Markdown string into a flat list of
//! `MdBlock`s using `pulldown-cmark`. Code blocks are syntax-highlighted
//! in-place using the existing `highlighter` infrastructure.
//!
//! The rendering of these blocks lives in `file_viewer/render/markdown.rs`.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::highlighter::highlight_raw_rows;
use super::mermaid_theme::MermaidPalette;
use super::visual::RasterImage;
use super::{VisualRow, VisualRowKind};

// ----------------------------------------------------------------
// Inline IR
// ----------------------------------------------------------------

#[derive(Clone)]
pub(in crate::workspace) enum MdSpan {
    Text(String),
    Bold(Vec<MdSpan>),
    Italic(Vec<MdSpan>),
    Code(String),
    Link {
        text: String,
        #[allow(dead_code)]
        url: String,
    },
    Strikethrough(Vec<MdSpan>),
    SoftBreak,
    HardBreak,
    /// Inline footnote reference `[^label]`.
    Footnote(String),
    /// Inline HTML shown verbatim in dim monospace.
    Html(String),
    /// Image reference. `raster` is filled by `resolve_images` after parsing
    /// (local files + data URIs only); `None` falls back to the alt text.
    Image {
        url: String,
        alt: String,
        raster: Option<RasterImage>,
    },
}

// ----------------------------------------------------------------
// List item IR
// ----------------------------------------------------------------

/// A single item in a bullet or ordered list.
#[derive(Clone)]
pub(in crate::workspace) struct ListItem {
    /// `Some(true)` = [x] checked, `Some(false)` = [ ] unchecked, `None` = plain bullet.
    pub checked: Option<bool>,
    pub spans: Vec<MdSpan>,
    /// Nested sublists (parsed recursively).
    pub children: Vec<MdBlock>,
}

// ----------------------------------------------------------------
// Block IR
// ----------------------------------------------------------------

#[derive(Clone)]
pub(in crate::workspace) enum MdBlock {
    Heading {
        level: u8,
        spans: Vec<MdSpan>,
    },
    Paragraph(Vec<MdSpan>),
    /// Pre-highlighted code block rows (one `VisualRow` per source line).
    CodeBlock {
        #[allow(dead_code)]
        lang: Option<String>,
        rows: Vec<VisualRow>,
    },
    BulletList(Vec<ListItem>),
    OrderedList {
        start: u64,
        items: Vec<ListItem>,
    },
    Blockquote(Vec<MdSpan>),
    Rule,
    /// GFM table. `header` is the first row; `rows` are the body rows.
    /// Each cell is a list of inline spans.
    Table {
        header: Vec<Vec<MdSpan>>,
        rows: Vec<Vec<Vec<MdSpan>>>,
    },
    /// Footnote definition `[^label]: ...` collected at end of document.
    FootnoteDefinition {
        label: String,
        spans: Vec<MdSpan>,
    },
    /// Raw HTML block (dim monospace passthrough).
    HtmlBlock(String),
    /// Mermaid diagram (```mermaid fence). `raster` is filled by the loader
    /// (merman → SVG → rasterize); `None` falls back to the raw source.
    Mermaid {
        source: String,
        raster: Option<RasterImage>,
    },
}

// ----------------------------------------------------------------
// Parser
// ----------------------------------------------------------------

/// Parse `text` into a `Vec<MdBlock>`. Code fences are syntax-highlighted
/// using `syntax_theme` (falls back to the bundled default on unknown names).
pub(in crate::workspace) fn parse_markdown(
    text: &str,
    syntax_theme: &str,
    is_light: bool,
) -> Vec<MdBlock> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_GFM);

    let events: Vec<Event<'_>> = Parser::new_ext(text, opts).collect();
    let mut pos = 0;
    let mut blocks = Vec::new();

    while pos < events.len() {
        if let Some((block, consumed)) = parse_block(&events, pos, syntax_theme, is_light) {
            blocks.push(block);
            pos += consumed;
        } else {
            pos += 1;
        }
    }
    blocks
}

// ----------------------------------------------------------------
// Block-level parsing helpers
// ----------------------------------------------------------------

fn parse_block(
    events: &[Event<'_>],
    pos: usize,
    syntax_theme: &str,
    is_light: bool,
) -> Option<(MdBlock, usize)> {
    match &events[pos] {
        Event::Start(Tag::Heading { level, .. }) => {
            let (spans, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Heading(_)))
            });
            Some((
                MdBlock::Heading {
                    level: heading_level_to_u8(*level),
                    spans,
                },
                consumed + 2, // +2: Start + End
            ))
        }

        Event::Start(Tag::Paragraph) => {
            let (spans, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Paragraph))
            });
            Some((MdBlock::Paragraph(spans), consumed + 2))
        }

        Event::Start(Tag::CodeBlock(kind)) => {
            let lang = match kind {
                CodeBlockKind::Fenced(s) if !s.is_empty() => {
                    let lang = s.split_whitespace().next().unwrap_or("").to_owned();
                    if lang.is_empty() { None } else { Some(lang) }
                }
                _ => None,
            };
            let (text, consumed) = collect_text_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::CodeBlock))
            });
            if lang.as_deref() == Some("mermaid") {
                return Some((
                    MdBlock::Mermaid {
                        source: text,
                        raster: None,
                    },
                    consumed + 2,
                ));
            }
            let mut rows = build_code_rows(&text);
            if let Some(ref l) = lang {
                highlight_raw_rows(&mut rows, l, syntax_theme, is_light);
            }
            Some((MdBlock::CodeBlock { lang, rows }, consumed + 2))
        }

        Event::Start(Tag::BlockQuote(_)) => {
            let (spans, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::BlockQuote(_)))
            });
            Some((MdBlock::Blockquote(spans), consumed + 2))
        }

        Event::Start(Tag::List(start_num)) => {
            let ordered = start_num.is_some();
            let start = start_num.unwrap_or(1);
            let mut items: Vec<ListItem> = Vec::new();
            let mut i = pos + 1;
            while i < events.len() {
                match &events[i] {
                    Event::Start(Tag::Item) => {
                        let (item, consumed) = parse_item(events, i + 1, syntax_theme, is_light);
                        items.push(item);
                        i += consumed + 2;
                    }
                    Event::End(TagEnd::List(_)) => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            let consumed = i - pos;
            let block = if ordered {
                MdBlock::OrderedList { start, items }
            } else {
                MdBlock::BulletList(items)
            };
            Some((block, consumed))
        }

        Event::Rule => Some((MdBlock::Rule, 1)),

        Event::Start(Tag::FootnoteDefinition(label)) => {
            let label = label.to_string();
            let mut spans: Vec<MdSpan> = Vec::new();
            let mut i = pos + 1;
            while i < events.len() {
                match &events[i] {
                    Event::End(TagEnd::FootnoteDefinition) => {
                        i += 1;
                        break;
                    }
                    // pulldown-cmark wraps footnote body in paragraphs; strip the wrapper.
                    Event::Start(Tag::Paragraph) => {
                        if !spans.is_empty() {
                            spans.push(MdSpan::SoftBreak);
                        }
                        let (ps, consumed) = collect_inline_until(events, i + 1, |e| {
                            matches!(e, Event::End(TagEnd::Paragraph))
                        });
                        spans.extend(ps);
                        i += consumed + 2;
                    }
                    _ => {
                        let (span, consumed) = parse_inline(events, i);
                        spans.push(span);
                        i += consumed;
                    }
                }
            }
            Some((MdBlock::FootnoteDefinition { label, spans }, i - pos))
        }

        Event::Html(s) => Some((MdBlock::HtmlBlock(s.to_string()), 1)),

        Event::Start(Tag::Table(_alignments)) => {
            let mut header: Vec<Vec<MdSpan>> = Vec::new();
            let mut rows: Vec<Vec<Vec<MdSpan>>> = Vec::new();
            let mut i = pos + 1;

            while i < events.len() {
                match &events[i] {
                    Event::Start(Tag::TableHead) => {
                        i += 1;
                        let mut head_cells: Vec<Vec<MdSpan>> = Vec::new();
                        while i < events.len() {
                            match &events[i] {
                                Event::Start(Tag::TableCell) => {
                                    let (spans, consumed) =
                                        collect_inline_until(events, i + 1, |e| {
                                            matches!(e, Event::End(TagEnd::TableCell))
                                        });
                                    head_cells.push(spans);
                                    i += consumed + 2;
                                }
                                Event::End(TagEnd::TableHead) => {
                                    i += 1;
                                    break;
                                }
                                _ => i += 1,
                            }
                        }
                        header = head_cells;
                    }
                    Event::Start(Tag::TableRow) => {
                        i += 1;
                        let mut row_cells: Vec<Vec<MdSpan>> = Vec::new();
                        while i < events.len() {
                            match &events[i] {
                                Event::Start(Tag::TableCell) => {
                                    let (spans, consumed) =
                                        collect_inline_until(events, i + 1, |e| {
                                            matches!(e, Event::End(TagEnd::TableCell))
                                        });
                                    row_cells.push(spans);
                                    i += consumed + 2;
                                }
                                Event::End(TagEnd::TableRow) => {
                                    i += 1;
                                    break;
                                }
                                _ => i += 1,
                            }
                        }
                        rows.push(row_cells);
                    }
                    Event::End(TagEnd::Table) => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            Some((MdBlock::Table { header, rows }, i - pos))
        }

        _ => None,
    }
}

// ----------------------------------------------------------------
// List item parser
// ----------------------------------------------------------------

/// Parse one list item starting at `pos` (just after `Start(Item)`).
/// Returns the item and the number of events consumed (NOT including `End(Item)`).
fn parse_item(
    events: &[Event<'_>],
    pos: usize,
    syntax_theme: &str,
    is_light: bool,
) -> (ListItem, usize) {
    let mut i = pos;

    // Task list marker is always the first event in task list items.
    let checked = if let Some(Event::TaskListMarker(c)) = events.get(i) {
        let c = *c;
        i += 1;
        Some(c)
    } else {
        None
    };

    let mut spans: Vec<MdSpan> = Vec::new();
    let mut children: Vec<MdBlock> = Vec::new();

    while i < events.len() {
        match &events[i] {
            Event::End(TagEnd::Item) => break,

            // Loose lists wrap item content in a paragraph.
            Event::Start(Tag::Paragraph) => {
                let (ps, consumed) = collect_inline_until(events, i + 1, |e| {
                    matches!(e, Event::End(TagEnd::Paragraph))
                });
                spans.extend(ps);
                i += consumed + 2;
            }

            // Nested list — recurse via parse_block.
            Event::Start(Tag::List(_)) => {
                if let Some((block, consumed)) = parse_block(events, i, syntax_theme, is_light) {
                    children.push(block);
                    i += consumed;
                } else {
                    i += 1;
                }
            }

            _ => {
                let (span, consumed) = parse_inline(events, i);
                spans.push(span);
                i += consumed;
            }
        }
    }

    (
        ListItem {
            checked,
            spans,
            children,
        },
        i - pos,
    )
}

// ----------------------------------------------------------------
// Inline parsing helpers
// ----------------------------------------------------------------

fn collect_inline_until<F>(events: &[Event<'_>], start: usize, stop: F) -> (Vec<MdSpan>, usize)
where
    F: Fn(&Event<'_>) -> bool,
{
    let mut spans = Vec::new();
    let mut i = start;
    while i < events.len() {
        if stop(&events[i]) {
            break;
        }
        let (span, consumed) = parse_inline(events, i);
        spans.push(span);
        i += consumed;
    }
    (spans, i - start)
}

fn parse_inline(events: &[Event<'_>], pos: usize) -> (MdSpan, usize) {
    match &events[pos] {
        Event::Text(s) => (MdSpan::Text(s.to_string()), 1),
        Event::Code(s) => (MdSpan::Code(s.to_string()), 1),
        Event::SoftBreak => (MdSpan::SoftBreak, 1),
        Event::HardBreak => (MdSpan::HardBreak, 1),

        Event::Start(Tag::Strong) => {
            let (inner, consumed) =
                collect_inline_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Strong)));
            (MdSpan::Bold(inner), consumed + 2)
        }

        Event::Start(Tag::Emphasis) => {
            let (inner, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Emphasis))
            });
            (MdSpan::Italic(inner), consumed + 2)
        }

        Event::Start(Tag::Strikethrough) => {
            let (inner, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Strikethrough))
            });
            (MdSpan::Strikethrough(inner), consumed + 2)
        }

        Event::Start(Tag::Link {
            dest_url, title, ..
        }) => {
            let url = if dest_url.is_empty() {
                title.to_string()
            } else {
                dest_url.to_string()
            };
            let (inner, consumed) =
                collect_inline_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Link)));
            let text = flatten_spans_to_text(&inner);
            (MdSpan::Link { text, url }, consumed + 2)
        }

        Event::Start(Tag::Image { dest_url, .. }) => {
            let (alt_text, consumed) =
                collect_text_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Image)));
            (
                MdSpan::Image {
                    url: dest_url.to_string(),
                    alt: alt_text,
                    raster: None,
                },
                consumed + 2,
            )
        }

        Event::InlineHtml(s) => (MdSpan::Html(s.to_string()), 1),
        Event::FootnoteReference(label) => (MdSpan::Footnote(label.to_string()), 1),
        Event::Html(_) => (MdSpan::Text(String::new()), 1),

        _ => (MdSpan::Text(String::new()), 1),
    }
}

// ----------------------------------------------------------------
// Utilities
// ----------------------------------------------------------------

fn collect_text_until<F>(events: &[Event<'_>], start: usize, stop: F) -> (String, usize)
where
    F: Fn(&Event<'_>) -> bool,
{
    let mut text = String::new();
    let mut i = start;
    while i < events.len() {
        if stop(&events[i]) {
            break;
        }
        if let Event::Text(s) = &events[i] {
            text.push_str(s);
        }
        i += 1;
    }
    (text, i - start)
}

/// Plain-text representation of a block for clipboard copy.
pub(in crate::workspace) fn md_block_plain_text(block: &MdBlock) -> String {
    match block {
        MdBlock::Heading { level, spans } => {
            let prefix = "#".repeat(*level as usize);
            format!("{} {}", prefix, flatten_spans_to_text(spans))
        }
        MdBlock::Paragraph(spans) => flatten_spans_to_text(spans),
        MdBlock::CodeBlock { lang, rows } => {
            let fence = lang.as_deref().unwrap_or("");
            let body = rows
                .iter()
                .map(|r| r.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            format!("```{fence}\n{body}\n```")
        }
        MdBlock::Mermaid { source, .. } => format!("```mermaid\n{source}\n```"),
        MdBlock::BulletList(items) => items
            .iter()
            .map(|item| {
                let prefix = match item.checked {
                    Some(true) => "- [x] ",
                    Some(false) => "- [ ] ",
                    None => "- ",
                };
                let mut text = format!("{}{}", prefix, flatten_spans_to_text(&item.spans));
                for child in &item.children {
                    for line in md_block_plain_text(child).lines() {
                        text.push('\n');
                        text.push_str("  ");
                        text.push_str(line);
                    }
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n"),
        MdBlock::OrderedList { start, items } => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mut text = format!(
                    "{}. {}",
                    start + i as u64,
                    flatten_spans_to_text(&item.spans)
                );
                for child in &item.children {
                    for line in md_block_plain_text(child).lines() {
                        text.push('\n');
                        text.push_str("  ");
                        text.push_str(line);
                    }
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n"),
        MdBlock::Blockquote(spans) => format!("> {}", flatten_spans_to_text(spans)),
        MdBlock::Rule => "---".to_owned(),
        MdBlock::FootnoteDefinition { label, spans } => {
            format!("[^{}]: {}", label, flatten_spans_to_text(spans))
        }
        MdBlock::HtmlBlock(html) => html.clone(),
        MdBlock::Table { header, rows } => {
            let header_line = header
                .iter()
                .map(|c| flatten_spans_to_text(c))
                .collect::<Vec<_>>()
                .join(" | ");
            let sep_line = header.iter().map(|_| "---").collect::<Vec<_>>().join(" | ");
            let body_lines: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|c| flatten_spans_to_text(c))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .collect();
            let mut out = format!("{header_line}\n{sep_line}");
            for line in body_lines {
                out.push('\n');
                out.push_str(&line);
            }
            out
        }
    }
}

fn flatten_spans_to_text(spans: &[MdSpan]) -> String {
    let mut out = String::new();
    for s in spans {
        match s {
            MdSpan::Text(t) | MdSpan::Code(t) | MdSpan::Link { text: t, .. } => out.push_str(t),
            MdSpan::Bold(inner) | MdSpan::Italic(inner) | MdSpan::Strikethrough(inner) => {
                out.push_str(&flatten_spans_to_text(inner));
            }
            MdSpan::SoftBreak | MdSpan::HardBreak => out.push(' '),
            MdSpan::Footnote(label) => out.push_str(&format!("[^{label}]")),
            MdSpan::Html(s) => out.push_str(s),
            MdSpan::Image { url, alt, .. } => {
                let label = if alt.is_empty() { url } else { alt };
                out.push_str(&format!("[{label}]"));
            }
        }
    }
    out
}

/// Walk every image span in `blocks` (recursing into nested spans and list
/// item children) and fill its `raster` via `resolve`. Pure traversal — the
/// caller supplies the I/O (file read + decode) as the closure.
pub(in crate::workspace) fn resolve_images(
    blocks: &mut [MdBlock],
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for block in blocks {
        match block {
            MdBlock::Heading { spans, .. }
            | MdBlock::Paragraph(spans)
            | MdBlock::Blockquote(spans)
            | MdBlock::FootnoteDefinition { spans, .. } => {
                resolve_images_in_spans(spans, resolve);
            }
            MdBlock::BulletList(items) | MdBlock::OrderedList { items, .. } => {
                for item in items {
                    resolve_images_in_spans(&mut item.spans, resolve);
                    resolve_images(&mut item.children, resolve);
                }
            }
            MdBlock::Table { header, rows } => {
                for cell in header {
                    resolve_images_in_spans(cell, resolve);
                }
                for row in rows {
                    for cell in row {
                        resolve_images_in_spans(cell, resolve);
                    }
                }
            }
            MdBlock::CodeBlock { .. }
            | MdBlock::Rule
            | MdBlock::HtmlBlock(_)
            | MdBlock::Mermaid { .. } => {}
        }
    }
}

/// If `spans` is a single image (ignoring surrounding whitespace), return its
/// `(alt, raster)`. Such a paragraph renders the image block-style (large);
/// otherwise images render inline, sized to the text line.
pub(in crate::workspace) fn lone_image(spans: &[MdSpan]) -> Option<(&str, Option<&RasterImage>)> {
    let mut found: Option<(&str, Option<&RasterImage>)> = None;
    for span in spans {
        match span {
            MdSpan::Image { alt, raster, .. } => {
                if found.is_some() {
                    return None;
                }
                found = Some((alt.as_str(), raster.as_ref()));
            }
            MdSpan::SoftBreak | MdSpan::HardBreak => {}
            MdSpan::Text(t) if t.trim().is_empty() => {}
            _ => return None,
        }
    }
    found
}

/// Build the merman host-theme profile matching daruda's active appearance
/// (`palette`), so every diagram type — not just flowchart nodes — picks up
/// daruda's actual surface/text/border/note/actor colors instead of leaving
/// diagram-specific elements (sequence notes/actors, pie background, ...) on
/// mermaid's own hardcoded light defaults. The root background is force-
/// patched to `transparent` regardless of which diagram renderer produced
/// the SVG — the rewrite is what stops the hardcoded-white leak many diagram
/// types don't route through `themeVariables`, and transparency (rather than
/// an opaque `canvas` fill) lets the host surface show through, matching the
/// translucent-tint design language of agent-chat cards. Node/label fills
/// stay opaque (`MermaidPalette` flattens them against `canvas`) so text
/// keeps a solid backing.
pub(in crate::workspace) fn mermaid_host_theme_profile(
    palette: &MermaidPalette,
) -> merman::render::HostThemeProfile {
    merman::render::HostThemeProfile::builder()
        .appearance(if palette.dark {
            merman::render::HostThemeAppearance::Dark
        } else {
            merman::render::HostThemeAppearance::Light
        })
        .roles(merman::render::HostThemeRoles {
            canvas: Some(palette.background.clone()),
            surface: Some(palette.primary_color.clone()),
            surface_alt: Some(palette.secondary_color.clone()),
            surface_muted: Some(palette.surface_muted.clone()),
            text: Some(palette.primary_text_color.clone()),
            subtle_text: Some(palette.line_color.clone()),
            border: Some(palette.primary_border_color.clone()),
            line: Some(palette.line_color.clone()),
            edge_label_background: Some(palette.background.clone()),
            cluster_background: Some(palette.cluster_background.clone()),
            cluster_border: Some(palette.primary_border_color.clone()),
            note_background: Some(palette.note_background.clone()),
            note_border: Some(palette.warning.clone()),
            note_text: Some(palette.note_text.clone()),
            actor_background: Some(palette.primary_color.clone()),
            actor_border: Some(palette.primary_border_color.clone()),
            actor_text: Some(palette.primary_text_color.clone()),
            activation_background: Some(palette.activation_background.clone()),
            activation_border: Some(palette.primary_border_color.clone()),
            error: Some(palette.error.clone()),
            warning: Some(palette.warning.clone()),
            success: Some(palette.success.clone()),
        })
        // Mindmap/timeline sections, pie slices, and git-graph branches don't
        // read from `roles` at all — they cycle a categorical `series_palette`
        // (`cScaleN`/`git{N}`/`pie{N}`). Left empty, merman's "base" theme
        // auto-derives those from `surface`, compounding into more
        // near-black boxes on top of the ones `roles` already covers. daruda
        // has no categorical palette of its own, so borrow merman's — tuned
        // by its own authors for the same "editor preview on a dark/light
        // host" case this is.
        .series_palette(if palette.dark {
            MERMAID_SERIES_PALETTE_DARK
        } else {
            MERMAID_SERIES_PALETTE_LIGHT
        })
        // `resvg_safe_editor()` defaults the root background to the opaque
        // `canvas` role; `Color(transparent)` keeps its rewrite of per-
        // diagram hardcoded backgrounds while clearing them instead of
        // repainting (usvg parses the non-standard root `background-color`
        // and `transparent` yields an alpha-0 fill). `None` would skip the
        // postprocessor entirely and let hardcoded whites through.
        .output(merman::render::HostThemeOutput {
            root_background: merman::render::HostThemeRootBackground::Color(
                MERMAID_ROOT_BACKGROUND.to_owned(),
            ),
            ..merman::render::HostThemeOutput::resvg_safe_editor()
        })
        .build()
}

/// CSS color for the patched SVG root background: transparent, so the
/// diagram composites over whatever surface hosts it (agent-chat card
/// tint, file-viewer background) instead of stamping an opaque rectangle.
const MERMAID_ROOT_BACKGROUND: &str = "transparent";

const MERMAID_SERIES_PALETTE_DARK: [&str; 8] = [
    "#60a5fa", "#34d399", "#f59e0b", "#c084fc", "#22d3ee", "#fb7185", "#facc15", "#a3e635",
];
const MERMAID_SERIES_PALETTE_LIGHT: [&str; 8] = [
    "#2563eb", "#059669", "#d97706", "#7c3aed", "#0891b2", "#be123c", "#a16207", "#65a30d",
];

/// Whether `source` already carries its own `%%{init: ...}%%` directive —
/// if so, daruda's host theme is skipped entirely so the diagram author's
/// customization (theme name, individual `themeVariables`, `themeCSS`, ...)
/// isn't silently overridden. merman applies a renderer-level host theme via
/// site config, which wins over a document-level directive wholesale rather
/// than merging per-field, so partial-respect isn't possible here — an
/// author who wrote any `%%{init}%%` block opts out of daruda's chrome for
/// that diagram.
pub(in crate::workspace) fn source_has_own_theme_directive(source: &str) -> bool {
    source.contains("%%{init")
}

/// Walk every mermaid block in `blocks` (recursing into list item children) and
/// fill its `raster` via `resolve`. Pure traversal — the caller supplies the
/// rendering (merman → SVG → rasterize) as the closure.
pub(in crate::workspace) fn resolve_mermaid(
    blocks: &mut [MdBlock],
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for block in blocks {
        match block {
            MdBlock::Mermaid { source, raster } => {
                if raster.is_none() {
                    *raster = resolve(source);
                }
            }
            MdBlock::BulletList(items) | MdBlock::OrderedList { items, .. } => {
                for item in items {
                    resolve_mermaid(&mut item.children, resolve);
                }
            }
            MdBlock::Heading { .. }
            | MdBlock::Paragraph(_)
            | MdBlock::CodeBlock { .. }
            | MdBlock::Blockquote(_)
            | MdBlock::Rule
            | MdBlock::Table { .. }
            | MdBlock::FootnoteDefinition { .. }
            | MdBlock::HtmlBlock(_) => {}
        }
    }
}

fn resolve_images_in_spans(
    spans: &mut [MdSpan],
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for span in spans {
        match span {
            MdSpan::Image { url, raster, .. } => {
                if raster.is_none() {
                    *raster = resolve(url);
                }
            }
            MdSpan::Bold(inner) | MdSpan::Italic(inner) | MdSpan::Strikethrough(inner) => {
                resolve_images_in_spans(inner, resolve);
            }
            MdSpan::Text(_)
            | MdSpan::Code(_)
            | MdSpan::Link { .. }
            | MdSpan::SoftBreak
            | MdSpan::HardBreak
            | MdSpan::Footnote(_)
            | MdSpan::Html(_) => {}
        }
    }
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn build_code_rows(text: &str) -> Vec<VisualRow> {
    text.lines()
        .enumerate()
        .map(|(i, line)| VisualRow {
            content: line.to_owned(),
            kind: VisualRowKind::Plain,
            line_no_left: (i + 1).to_string(),
            line_no_right: String::new(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        })
        .collect()
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_heading() {
        let blocks = parse_markdown("# Hello\n", "base16-ocean.dark", false);
        assert!(matches!(blocks[0], MdBlock::Heading { level: 1, .. }));
    }

    #[test]
    fn parse_paragraph_with_inline() {
        let blocks = parse_markdown("normal **bold** text\n", "base16-ocean.dark", false);
        assert!(matches!(blocks[0], MdBlock::Paragraph(_)));
        if let MdBlock::Paragraph(spans) = &blocks[0] {
            assert!(spans.iter().any(|s| matches!(s, MdSpan::Bold(_))));
        }
    }

    #[test]
    fn parse_fenced_code_block() {
        let md = "```rust\nfn main() {}\n```\n";
        let blocks = parse_markdown(md, "base16-ocean.dark", false);
        assert!(matches!(
            blocks[0],
            MdBlock::CodeBlock { lang: Some(_), .. }
        ));
    }

    #[test]
    fn parse_bullet_list() {
        let blocks = parse_markdown("- item one\n- item two\n", "base16-ocean.dark", false);
        assert!(matches!(blocks[0], MdBlock::BulletList(_)));
        if let MdBlock::BulletList(items) = &blocks[0] {
            assert_eq!(items.len(), 2);
        }
    }

    #[test]
    fn parse_horizontal_rule() {
        let blocks = parse_markdown("---\n", "base16-ocean.dark", false);
        assert!(matches!(blocks[0], MdBlock::Rule));
    }

    #[test]
    fn resolve_images_fills_raster_for_each_image() {
        let mut blocks = vec![MdBlock::Paragraph(vec![MdSpan::Image {
            url: "pic.png".to_owned(),
            alt: "a".to_owned(),
            raster: None,
        }])];
        let dummy = RasterImage {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
            scale: 1.0,
        };
        resolve_images(&mut blocks, &mut |url| {
            assert_eq!(url, "pic.png");
            Some(dummy.clone())
        });
        let MdBlock::Paragraph(spans) = &blocks[0] else {
            panic!("expected paragraph");
        };
        let MdSpan::Image { raster, .. } = &spans[0] else {
            panic!("expected image span");
        };
        assert!(raster.is_some());
    }

    #[test]
    fn lone_image_detects_standalone_image_paragraphs() {
        let img = || MdSpan::Image {
            url: "x".to_owned(),
            alt: "a".to_owned(),
            raster: None,
        };
        // single image → standalone
        assert!(lone_image(&[img()]).is_some());
        // image + whitespace-only text → still standalone
        assert!(lone_image(&[img(), MdSpan::Text("   ".to_owned())]).is_some());
        // image among real text → inline (None)
        assert!(lone_image(&[MdSpan::Text("see ".to_owned()), img()]).is_none());
        // two images → not a lone image
        assert!(lone_image(&[img(), img()]).is_none());
    }

    fn test_palette() -> MermaidPalette {
        MermaidPalette {
            dark: true,
            background: "#111111".to_owned(),
            primary_color: "#222222".to_owned(),
            primary_text_color: "#eeeeee".to_owned(),
            primary_border_color: "#333333".to_owned(),
            line_color: "#cccccc".to_owned(),
            secondary_color: "#444444".to_owned(),
            surface_muted: "#1a1a1a".to_owned(),
            cluster_background: "#1c1c1c".to_owned(),
            note_background: "#3a2a10".to_owned(),
            note_text: "#f0d9a0".to_owned(),
            activation_background: "#2a2a2a".to_owned(),
            error: "#ff6666".to_owned(),
            warning: "#ffcc66".to_owned(),
            success: "#66ff99".to_owned(),
        }
    }

    #[test]
    fn mermaid_host_theme_profile_matches_appearance_and_palette() {
        let palette = test_palette();
        let dark = mermaid_host_theme_profile(&palette);
        assert_eq!(dark.appearance, merman::render::HostThemeAppearance::Dark);
        assert_eq!(
            dark.roles.canvas.as_deref(),
            Some(palette.background.as_str())
        );
        assert_eq!(
            dark.roles.text.as_deref(),
            Some(palette.primary_text_color.as_str())
        );

        let mut light = palette.clone();
        light.dark = false;
        let light_profile = mermaid_host_theme_profile(&light);
        assert_eq!(
            light_profile.appearance,
            merman::render::HostThemeAppearance::Light
        );
    }

    /// Regression guard: mindmap/timeline/pie/gitgraph sections don't read
    /// `roles` at all — they cycle a categorical `series_palette`
    /// (`cScaleN`/`git{N}`/`pie{N}`). An empty palette isn't "use mermaid's
    /// default colors", it's "auto-derive from `surface`", which compounds
    /// into near-black boxes on top of a dark `surface`. See the mindmap/
    /// timeline "too black" report this guards against.
    #[test]
    fn mermaid_host_theme_profile_always_sets_a_series_palette() {
        assert!(
            !mermaid_host_theme_profile(&test_palette())
                .series_palette
                .is_empty()
        );
        let mut light = test_palette();
        light.dark = false;
        assert!(!mermaid_host_theme_profile(&light).series_palette.is_empty());
    }

    /// Regression guard: the root background must be patched to
    /// `transparent` — `Canvas` would stamp an opaque rectangle that breaks
    /// the translucent agent-chat card design, while `None` would skip the
    /// rewrite and let per-diagram hardcoded white backgrounds through.
    #[test]
    fn mermaid_host_theme_profile_patches_root_background_transparent() {
        assert_eq!(
            mermaid_host_theme_profile(&test_palette())
                .output
                .root_background,
            merman::render::HostThemeRootBackground::Color("transparent".to_owned())
        );
    }

    #[test]
    fn source_has_own_theme_directive_detects_any_init_block() {
        assert!(!source_has_own_theme_directive("graph TD\nA-->B"));
        assert!(source_has_own_theme_directive(
            "%%{init: {\"theme\":\"forest\"}}%%\ngraph TD\nA-->B"
        ));
        // Even a themeVariables-only (no theme name) directive opts out —
        // daruda's host theme can't merge on top of it per-field.
        assert!(source_has_own_theme_directive(
            "%%{init: {\"themeVariables\": {\"primaryColor\": \"#ff0000\"}}}%%\ngraph TD\nA-->B"
        ));
    }

    #[test]
    fn resolve_mermaid_fills_raster() {
        let mut blocks = vec![MdBlock::Mermaid {
            source: "graph TD\nA-->B".to_owned(),
            raster: None,
        }];
        let dummy = RasterImage {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
            scale: 1.0,
        };
        resolve_mermaid(&mut blocks, &mut |src| {
            assert!(src.contains("graph"));
            Some(dummy.clone())
        });
        let MdBlock::Mermaid { raster, .. } = &blocks[0] else {
            panic!("expected mermaid block");
        };
        assert!(raster.is_some());
    }

    #[test]
    fn resolve_images_recurses_into_nested_spans() {
        let mut blocks = vec![MdBlock::Paragraph(vec![MdSpan::Bold(vec![
            MdSpan::Image {
                url: "n.png".to_owned(),
                alt: String::new(),
                raster: None,
            },
        ])])];
        let mut count = 0;
        resolve_images(&mut blocks, &mut |_| {
            count += 1;
            None
        });
        assert_eq!(count, 1);
    }
}
