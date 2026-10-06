//! Markdown to Telegram-HTML conversion for outgoing bridge messages.
//!
//! GPUI-free, stateless, and fallible only at the later HTTP send step. HTML is
//! used instead of MarkdownV2 because Telegram supports the same entity set but
//! HTML has a much smaller, context-independent escaping surface (`<`, `>`,
//! `&`), reducing silent HTTP 400 failures. GFM constructs without Telegram
//! equivalents are downgraded to plain-text approximations.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Convert CommonMark/GFM source into Telegram's HTML `parse_mode` subset.
/// Unmapped constructs degrade to plain text rather than failing.
pub fn to_telegram_html(source: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_GFM);
    let events: Vec<Event<'_>> = Parser::new_ext(source, opts).collect();

    let mut blocks = Vec::new();
    let mut pos = 0;
    while pos < events.len() {
        if let Some((block, consumed)) = render_block(&events, pos) {
            if !block.is_empty() {
                blocks.push(block);
            }
            pos += consumed.max(1);
        } else {
            pos += 1;
        }
    }
    blocks.join("\n\n")
}

/// Escape HTML text for Telegram `parse_mode`; also used for plain tail
/// segments that must be escaped but not markdown-parsed.
pub(crate) fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// [`escape_text`] plus quote-escaping, for a value going inside a
/// double-quoted HTML attribute (`href`, `class`).
fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// One block-level construct starting at `events[pos]`. Returns the
/// rendered HTML and how many events it consumed, or `None` for an event
/// this function doesn't start a block at (the caller skips one event and
/// retries — mirrors `file_view_pane::markdown_viewer::parse_block`'s same
/// "unrecognized → skip" fallback).
fn render_block(events: &[Event<'_>], pos: usize) -> Option<(String, usize)> {
    match &events[pos] {
        // No Telegram heading entity exists — bold is the closest
        // equivalent that still visually sets the line apart.
        Event::Start(Tag::Heading { .. }) => {
            let (inner, consumed) = render_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Heading(_)))
            });
            Some((format!("<b>{inner}</b>"), consumed + 2))
        }

        Event::Start(Tag::Paragraph) => {
            let (inner, consumed) = render_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Paragraph))
            });
            Some((inner, consumed + 2))
        }

        // A `mermaid` fence gets no special treatment here (unlike the
        // in-app renderer, which rasterizes it to an image) — a Telegram
        // text message cannot embed an image at all, so the only faithful
        // option is showing the diagram source as a normal code block.
        Event::Start(Tag::CodeBlock(kind)) => {
            let lang = match kind {
                CodeBlockKind::Fenced(s) if !s.is_empty() => {
                    let lang = s.split_whitespace().next().unwrap_or("").to_owned();
                    if lang.is_empty() { None } else { Some(lang) }
                }
                _ => None,
            };
            let (text, consumed) = collect_raw_text_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::CodeBlock))
            });
            // pulldown-cmark's fenced-code text always carries the final
            // line's trailing newline (the one before the closing fence) —
            // not meaningful content, so it's dropped rather than shown as
            // a trailing blank line inside the `<pre>` block.
            let escaped = escape_text(text.strip_suffix('\n').unwrap_or(&text));
            let rendered = match lang {
                Some(lang) => format!(
                    "<pre><code class=\"language-{}\">{escaped}</code></pre>",
                    escape_attr(&lang)
                ),
                None => format!("<pre>{escaped}</pre>"),
            };
            Some((rendered, consumed + 2))
        }

        // Telegram's HTML `<blockquote>` wraps arbitrary multi-line content
        // in one tag — no per-line `>` needed, unlike CommonMark source.
        Event::Start(Tag::BlockQuote(_)) => {
            let (inner, consumed) = render_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::BlockQuote(_)))
            });
            Some((format!("<blockquote>{inner}</blockquote>"), consumed + 2))
        }

        Event::Start(Tag::List(start_num)) => Some(render_list(events, pos, *start_num, 0)),

        // No `<hr>` equivalent; a plain divider line reads fine in a chat.
        Event::Rule => Some(("———".to_string(), 1)),

        // Footnotes render inline as `[^label]: body`, right where the
        // definition appears in the source, rather than being collected
        // into an end-of-message references list — footnotes are rare
        // enough in agent chat responses that the simpler (if less
        // typographically faithful) inline placement isn't worth a second
        // accumulator threaded through every block-rendering call site.
        Event::Start(Tag::FootnoteDefinition(label)) => {
            let label = escape_text(label);
            let (body, consumed) = render_footnote_body(events, pos + 1);
            Some((format!("[^{label}]: {body}"), consumed + 2))
        }

        // Raw HTML from the source is shown, escaped, as literal text
        // rather than passed through: it was never meant for Telegram's
        // parser, and letting it through unescaped risks either breaking
        // message-entity parsing or rendering tags Telegram doesn't
        // recognize as visible garbage anyway.
        Event::Html(s) => Some((escape_text(s), 1)),

        Event::Start(Tag::Table(_)) => Some(render_table(events, pos)),

        _ => None,
    }
}

/// Render one `Tag::List` (ordered or unordered) starting at `events[pos]`,
/// at nesting depth `indent` (0 = top level). No native Telegram list
/// entity exists in either parse_mode, so items become plain
/// `"- text"` / `"N. text"` lines (`"☑ "` / `"☐ "` for a GFM task-list
/// item), two-space-indented per nesting level, joined by newlines.
fn render_list(
    events: &[Event<'_>],
    pos: usize,
    start_num: Option<u64>,
    indent: usize,
) -> (String, usize) {
    let ordered = start_num.is_some();
    let mut next_index = start_num.unwrap_or(1);
    let mut lines = Vec::new();
    let mut i = pos + 1;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Item) => {
                let (line, consumed) = render_item(events, i + 1, indent, ordered, next_index);
                lines.push(line);
                next_index += 1;
                i += consumed + 2;
            }
            Event::End(TagEnd::List(_)) => {
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }
    (lines.join("\n"), i - pos)
}

/// Render one `Tag::Item`'s content (starting just past `Start(Item)`) as a
/// single indented line, recursing into a nested list as extra lines
/// underneath. Returns the line(s) and events consumed, NOT including the
/// trailing `End(Item)` (mirrors `markdown_viewer::parse_item`'s contract).
fn render_item(
    events: &[Event<'_>],
    pos: usize,
    indent: usize,
    ordered: bool,
    index: u64,
) -> (String, usize) {
    let mut i = pos;
    let checked = if let Some(Event::TaskListMarker(c)) = events.get(i) {
        let c = *c;
        i += 1;
        Some(c)
    } else {
        None
    };

    let mut text = String::new();
    let mut children: Vec<String> = Vec::new();
    while i < events.len() {
        match &events[i] {
            Event::End(TagEnd::Item) => break,
            // Loose lists wrap item content in a paragraph.
            Event::Start(Tag::Paragraph) => {
                let (inner, consumed) = render_inline_until(events, i + 1, |e| {
                    matches!(e, Event::End(TagEnd::Paragraph))
                });
                text.push_str(&inner);
                i += consumed + 2;
            }
            Event::Start(Tag::List(start_num)) => {
                let (child, consumed) = render_list(events, i, *start_num, indent + 1);
                children.push(child);
                i += consumed;
            }
            _ => {
                let (s, consumed) = render_inline(events, i);
                text.push_str(&s);
                i += consumed;
            }
        }
    }

    let bullet = match checked {
        Some(true) => "☑ ".to_string(),
        Some(false) => "☐ ".to_string(),
        None if ordered => format!("{index}. "),
        None => "- ".to_string(),
    };
    let prefix = "  ".repeat(indent);
    let mut line = format!("{prefix}{bullet}{text}");
    // `child` was rendered with `indent + 1`, so each of its lines already
    // carries its own correct indentation — appending it verbatim, not with
    // an extra prefix here, is what keeps nesting at exactly two spaces per
    // level instead of doubling up.
    for child in children {
        for child_line in child.lines() {
            line.push('\n');
            line.push_str(child_line);
        }
    }
    (line, i - pos)
}

/// Render a GFM table as a monospace block. Telegram has no table entity in
/// either parse_mode, so this is the same plain-text degrade
/// `file_view_pane::markdown_viewer::md_block_plain_text` uses for its
/// clipboard-copy path — a `<pre>` block keeps it monospace-aligned-ish and
/// visually set apart. Cell content is flattened to plain text (not run
/// through [`render_inline`]) because it ends up inside `<pre>`, where a
/// `<b>`/`<i>` tag would show as literal, un-rendered angle brackets rather
/// than being interpreted.
fn render_table(events: &[Event<'_>], pos: usize) -> (String, usize) {
    let mut header: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut i = pos + 1;

    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::TableHead) => {
                i += 1;
                while i < events.len() {
                    match &events[i] {
                        Event::Start(Tag::TableCell) => {
                            let (cell, consumed) = flatten_inline_until(events, i + 1, |e| {
                                matches!(e, Event::End(TagEnd::TableCell))
                            });
                            header.push(cell);
                            i += consumed + 2;
                        }
                        Event::End(TagEnd::TableHead) => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            Event::Start(Tag::TableRow) => {
                i += 1;
                let mut row = Vec::new();
                while i < events.len() {
                    match &events[i] {
                        Event::Start(Tag::TableCell) => {
                            let (cell, consumed) = flatten_inline_until(events, i + 1, |e| {
                                matches!(e, Event::End(TagEnd::TableCell))
                            });
                            row.push(cell);
                            i += consumed + 2;
                        }
                        Event::End(TagEnd::TableRow) => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                rows.push(row);
            }
            Event::End(TagEnd::Table) => {
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }

    let mut lines = vec![header.join(" | ")];
    lines.push(header.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    for row in &rows {
        lines.push(row.join(" | "));
    }
    (
        format!("<pre>{}</pre>", escape_text(&lines.join("\n"))),
        i - pos,
    )
}

/// A footnote definition's body: nested paragraphs are un-wrapped (their
/// text is kept, the wrapper contributes only a separating space) since a
/// footnote is rendered as a single `"[^label]: body"` line.
fn render_footnote_body(events: &[Event<'_>], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = start;
    while i < events.len() {
        match &events[i] {
            Event::End(TagEnd::FootnoteDefinition) => {
                i += 1;
                break;
            }
            Event::Start(Tag::Paragraph) => {
                let (inner, consumed) = render_inline_until(events, i + 1, |e| {
                    matches!(e, Event::End(TagEnd::Paragraph))
                });
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&inner);
                i += consumed + 2;
            }
            _ => {
                let (s, consumed) = render_inline(events, i);
                text.push_str(&s);
                i += consumed;
            }
        }
    }
    (text, i - start)
}

/// Render inline events from `start` until `stop` matches, concatenating
/// each event's HTML (see [`render_inline`]).
fn render_inline_until(
    events: &[Event<'_>],
    start: usize,
    stop: impl Fn(&Event<'_>) -> bool,
) -> (String, usize) {
    let mut out = String::new();
    let mut i = start;
    while i < events.len() {
        if stop(&events[i]) {
            break;
        }
        let (s, consumed) = render_inline(events, i);
        out.push_str(&s);
        i += consumed;
    }
    (out, i - start)
}

/// One inline construct at `events[pos]`. An event this function doesn't
/// recognize (e.g. a block-tag boundary that leaked into an inline run —
/// see the blockquote/footnote callers, which intentionally hand it a
/// `Tag::Paragraph` wrapper to swallow) renders as an empty string and
/// consumes exactly one event, so the surrounding loop always makes
/// progress.
fn render_inline(events: &[Event<'_>], pos: usize) -> (String, usize) {
    match &events[pos] {
        Event::Text(s) => (escape_text(s), 1),
        Event::Code(s) => (format!("<code>{}</code>", escape_text(s)), 1),
        Event::SoftBreak | Event::HardBreak => ("\n".to_string(), 1),

        Event::Start(Tag::Strong) => {
            let (inner, consumed) =
                render_inline_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Strong)));
            (format!("<b>{inner}</b>"), consumed + 2)
        }
        Event::Start(Tag::Emphasis) => {
            let (inner, consumed) = render_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Emphasis))
            });
            (format!("<i>{inner}</i>"), consumed + 2)
        }
        Event::Start(Tag::Strikethrough) => {
            let (inner, consumed) = render_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Strikethrough))
            });
            (format!("<s>{inner}</s>"), consumed + 2)
        }

        Event::Start(Tag::Link {
            dest_url, title, ..
        }) => {
            let url = if dest_url.is_empty() { title } else { dest_url };
            let (inner, consumed) =
                render_inline_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Link)));
            (
                format!("<a href=\"{}\">{inner}</a>", escape_attr(url)),
                consumed + 2,
            )
        }
        // A Telegram text message cannot embed an image inline (that needs
        // a separate `sendPhoto` call) — downgrade to a link so the image
        // is still reachable with one tap, labeled with its alt text.
        Event::Start(Tag::Image { dest_url, .. }) => {
            let (alt, consumed) =
                collect_raw_text_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Image)));
            let label = if alt.trim().is_empty() {
                "[image]".to_string()
            } else {
                format!("[image: {}]", escape_text(alt.trim()))
            };
            (
                format!("<a href=\"{}\">{label}</a>", escape_attr(dest_url)),
                consumed + 2,
            )
        }

        Event::InlineHtml(s) | Event::Html(s) => (escape_text(s), 1),
        Event::FootnoteReference(label) => (format!("[^{}]", escape_text(label)), 1),

        _ => (String::new(), 1),
    }
}

/// Flatten inline events to plain text (no HTML tags at all — see
/// [`render_table`] for why), for a run of events from `start` until `stop`
/// matches. Bold/italic/strikethrough/link *wrapper* tags contribute
/// nothing; the plain text and inline code nested inside them still comes
/// through via their own `Event::Text`/`Event::Code`.
fn flatten_inline_until(
    events: &[Event<'_>],
    start: usize,
    stop: impl Fn(&Event<'_>) -> bool,
) -> (String, usize) {
    let mut text = String::new();
    let mut i = start;
    while i < events.len() {
        if stop(&events[i]) {
            break;
        }
        match &events[i] {
            Event::Text(s) | Event::Code(s) => text.push_str(s),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            Event::FootnoteReference(label) => {
                text.push_str("[^");
                text.push_str(label);
                text.push(']');
            }
            _ => {}
        }
        i += 1;
    }
    (text, i - start)
}

/// Accumulate raw `Event::Text` between `start` and `stop`, ignoring any
/// nested tags — used where only the literal text matters (a code block's
/// body, an image's alt text).
fn collect_raw_text_until(
    events: &[Event<'_>],
    start: usize,
    stop: impl Fn(&Event<'_>) -> bool,
) -> (String, usize) {
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

#[cfg(test)]
mod tests {
    use super::to_telegram_html;

    #[test]
    fn markdown_constructs_render_to_telegram_html() {
        let cases = [
            (
                "emphasis",
                "**bold** _italic_ ~~gone~~",
                "<b>bold</b> <i>italic</i> <s>gone</s>",
            ),
            (
                "inline code escape",
                "run `a < b && c > d`",
                "run <code>a &lt; b &amp;&amp; c &gt; d</code>",
            ),
            (
                "fenced code language",
                "```rust\nfn main() {}\n```",
                "<pre><code class=\"language-rust\">fn main() {}</code></pre>",
            ),
            (
                "fenced code no language",
                "```\nplain\n```",
                "<pre>plain</pre>",
            ),
            (
                "mermaid fence stays source",
                "```mermaid\ngraph TD\nA-->B\n```",
                "<pre><code class=\"language-mermaid\">graph TD\nA--&gt;B</code></pre>",
            ),
            (
                "link",
                "[daruda](https://example.com/a?b=1&c=2)",
                "<a href=\"https://example.com/a?b=1&amp;c=2\">daruda</a>",
            ),
            ("heading", "# Title", "<b>Title</b>"),
            (
                "blockquote",
                "> quoted text",
                "<blockquote>quoted text</blockquote>",
            ),
            ("thematic break", "---", "———"),
            ("bullet list", "- one\n- two", "- one\n- two"),
            ("ordered list", "5. five\n6. six", "5. five\n6. six"),
            ("nested list", "- outer\n  - inner", "- outer\n  - inner"),
            ("task list", "- [x] done\n- [ ] todo", "☑ done\n☐ todo"),
            (
                "table",
                "| a | b |\n|---|---|\n| 1 | 2 |",
                "<pre>a | b\n--- | ---\n1 | 2</pre>",
            ),
            (
                "table formatting flattened",
                "| a |\n|---|\n| **bold** |",
                "<pre>a\n---\nbold</pre>",
            ),
            (
                "image alt",
                "![a diagram](https://example.com/x.png)",
                "<a href=\"https://example.com/x.png\">[image: a diagram]</a>",
            ),
            (
                "image generic",
                "![](https://example.com/x.png)",
                "<a href=\"https://example.com/x.png\">[image]</a>",
            ),
            (
                "raw html escaped",
                "<div>hello</div>",
                "&lt;div&gt;hello&lt;/div&gt;",
            ),
            (
                "plain text escape",
                "a < b & c > d, no markdown here",
                "a &lt; b &amp; c &gt; d, no markdown here",
            ),
            ("paragraphs", "first\n\nsecond", "first\n\nsecond"),
            ("empty", "", ""),
        ];

        for (name, source, expected) in cases {
            assert_eq!(to_telegram_html(source), expected, "{name}");
        }
    }

    #[test]
    fn footnote_renders_inline_as_a_label_body_line() {
        let html = to_telegram_html("See it.[^1]\n\n[^1]: the footnote body");
        assert!(html.contains("See it.[^1]"));
        assert!(html.contains("[^1]: the footnote body"));
    }

    #[test]
    fn edge_cases_do_not_panic() {
        let cases = [
            "***bold italic***",
            "- \n- ",
            "| |\n|-|\n",
            "```\nno trailing newline```",
            "# [a link](https://x)",
            "**a*b**c*",
            "> \n> \n",
            "- [x]\n- [ ]",
            "[![alt](img.png)](https://x)",
            "1. \n2. ",
            "- outer\n  - inner\n    - deepest\n",
        ];
        for c in cases {
            let _ = to_telegram_html(c);
        }
    }
}
