use super::*;
use std::collections::BTreeSet;

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
fn links_preserve_nested_inline_styles_and_plain_text() {
    let blocks = parse_markdown(
        "[**bold** and *italic*](https://example.com)\n",
        "base16-ocean.dark",
        false,
    );
    let MdBlock::Paragraph(spans) = &blocks[0] else {
        panic!("expected paragraph");
    };
    let [MdSpan::Link { children, url }] = spans.as_slice() else {
        panic!("expected one link");
    };

    assert_eq!(url, "https://example.com");
    assert!(children.iter().any(|span| matches!(span, MdSpan::Bold(_))));
    assert!(
        children
            .iter()
            .any(|span| matches!(span, MdSpan::Italic(_)))
    );
    assert_eq!(md_block_plain_text(&blocks[0]), "bold and italic");
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

/// A fence's info string is a **language name**, not a file extension.
/// Resolving it through the extension table left the most common fences
/// (`rust`, `python`, `javascript`, …) un-highlighted, while the handful
/// whose name happens to equal their extension (`bash`, `java`, `go`)
/// worked — which is why the breakage looked arbitrary.
#[test]
fn fenced_code_blocks_are_highlighted_by_language_name() {
    let cases = [
        ("rust", "fn main() { let x = 1; }"),
        ("python", "def f():\n    return 1"),
        ("javascript", "function f() { return 1; }"),
        ("typescript", "function f(): number { return 1; }"),
        ("ruby", "def f\n  1\nend"),
        // Name == extension: these already worked, and must keep working.
        ("bash", "echo hi"),
        ("go", "func main() { x := 1 }"),
    ];
    for (lang, code) in cases {
        let md = format!("```{lang}\n{code}\n```\n");
        let blocks = parse_markdown(&md, "base16-ocean.dark", false);
        let MdBlock::CodeBlock { rows, .. } = &blocks[0] else {
            panic!("`{lang}` fence did not parse as a code block");
        };
        assert!(
            rows.iter().any(|r| !r.spans.is_empty()),
            "```{lang} produced no highlighted spans"
        );
    }
}

/// pulldown-cmark wraps each paragraph of a footnote definition, and the
/// parser strips those wrappers to keep one flat span run. Without a separator
/// at each wrapper the two paragraphs' text would run together into one
/// sentence, so the break is what a multi-paragraph footnote reads as.
#[test]
fn a_multi_paragraph_footnote_keeps_its_paragraphs_apart() {
    let blocks = parse_markdown(
        "[^n]: first paragraph\n\n    second paragraph\n",
        "base16-ocean.dark",
        false,
    );
    let footnotes: Vec<_> = blocks
        .iter()
        .filter_map(|b| match b {
            MdBlock::FootnoteDefinition { label, spans } => Some((label, spans)),
            _ => None,
        })
        .collect();

    assert_eq!(
        footnotes.len(),
        1,
        "both paragraphs belong to the one definition, not a stray top-level block"
    );
    let (label, spans) = footnotes[0];
    assert_eq!(label, "n");
    assert!(
        matches!(
            spans.as_slice(),
            [
                MdSpan::Text(first),
                MdSpan::SoftBreak,
                MdSpan::Text(second)
            ] if first == "first paragraph" && second == "second paragraph"
        ),
        "the wrapper is stripped but the break survives, got {} spans",
        spans.len()
    );
}

#[test]
fn parse_bullet_list() {
    let blocks = parse_markdown("- item one\n- item two\n", "base16-ocean.dark", false);
    assert!(matches!(blocks[0], MdBlock::BulletList { .. }));
    if let MdBlock::BulletList { items, .. } = &blocks[0] {
        assert_eq!(items.len(), 2);
    }
}

/// A shared `loose_of` for the looseness tests: the flag the renderer
/// reads, off the document's first block.
fn loose_of(md: &str) -> bool {
    match &parse_markdown(md, "base16-ocean.dark", false)[0] {
        MdBlock::BulletList { loose, .. } | MdBlock::OrderedList { loose, .. } => *loose,
        other => panic!("expected a list, got {}", md_block_plain_text(other)),
    }
}

/// Looseness decides whether items are spaced like paragraphs, and the IR
/// is the only place it can travel: `Tag::List` reports the marker and
/// start number, nothing else.
#[test]
fn a_blank_line_between_items_makes_the_list_loose() {
    assert!(!loose_of("- one\n- two\n"), "flush items are tight");
    assert!(loose_of("- one\n\n- two\n"), "blank-line items are loose");
    assert!(
        !loose_of("1. one\n2. two\n"),
        "flush ordered items are tight"
    );
    assert!(
        loose_of("1. one\n\n2. two\n"),
        "blank-line ordered items are loose"
    );
}

/// CommonMark calls a list loose when any item holds more than one block,
/// blank line between the items or not — and a nested sublist alone does
/// not make one.
#[test]
fn a_multi_block_item_is_loose_but_a_nested_sublist_is_not() {
    assert!(loose_of("- one\n\n  continued\n- two\n"));
    assert!(!loose_of("- one\n  - nested\n- two\n"));
}

/// Only the list the blank line is in goes loose. Each nesting level is
/// parsed by its own `Tag::List` arm, so neither can reach the other's
/// tally.
#[test]
fn looseness_does_not_leak_between_nesting_levels() {
    let outer_inner = |md: &str| match &parse_markdown(md, "base16-ocean.dark", false)[0] {
        MdBlock::BulletList { items, loose } => {
            let inner = items.iter().find_map(|i| match i.children.first() {
                Some(MdBlock::BulletList { loose, .. }) => Some(*loose),
                _ => None,
            });
            (*loose, inner.expect("a nested list"))
        }
        _ => panic!("expected a bullet list"),
    };

    assert_eq!(outer_inner("- one\n  - a\n\n  - b\n- two\n"), (false, true));
    assert_eq!(outer_inner("- one\n  - a\n  - b\n\n- two\n"), (true, false));
}

/// One paragraph-wrapped item anywhere is enough. A list whose first item
/// opens with a fence has no wrapper to read there, and a check that only
/// looked at the first item would call the whole list tight.
#[test]
fn a_later_item_can_be_what_marks_the_list_loose() {
    assert!(loose_of("- ```\n  a\n  ```\n\n- two\n"));
    assert!(loose_of("-\n\n- two\n"), "an empty first item");
}

/// A copy has to paste back as the same list. Flattening a loose list to
/// single newlines turned it tight on the round trip.
#[test]
fn copied_text_keeps_a_list_as_loose_or_tight_as_it_was() {
    let copy = |md: &str| md_block_plain_text(&parse_markdown(md, "base16-ocean.dark", false)[0]);

    assert_eq!(copy("- one\n- two\n"), "- one\n- two");
    assert_eq!(copy("- one\n\n- two\n"), "- one\n\n- two");
    assert_eq!(copy("1. one\n2. two\n"), "1. one\n2. two");
    assert_eq!(copy("1. one\n\n2. two\n"), "1. one\n\n2. two");
}

/// The checkbox has to survive a loose list. pulldown-cmark moves the
/// marker inside the item's leading paragraph there, so a lookup fixed on
/// the item's own first event returned `None` and the box fell back to a
/// plain bullet.
#[test]
fn a_task_item_keeps_its_checkbox_tight_or_loose() {
    let checks = |md: &str| match &parse_markdown(md, "base16-ocean.dark", false)[0] {
        MdBlock::BulletList { items, .. } => items.iter().map(|i| i.checked).collect::<Vec<_>>(),
        _ => panic!("expected a bullet list"),
    };

    let both = vec![Some(false), Some(true)];
    assert_eq!(checks("- [ ] one\n- [x] two\n"), both, "tight");
    assert_eq!(checks("- [ ] one\n\n- [x] two\n"), both, "loose");
    assert_eq!(checks("- one\n- two\n"), vec![None, None], "plain bullets");
}

/// The marker is the checkbox, not prose — it must not leave a stray span
/// in the item's text on either shape.
#[test]
fn a_task_items_text_excludes_its_marker() {
    let copy = |md: &str| md_block_plain_text(&parse_markdown(md, "base16-ocean.dark", false)[0]);

    assert_eq!(copy("- [ ] one\n- [x] two\n"), "- [ ] one\n- [x] two");
    assert_eq!(copy("- [ ] one\n\n- [x] two\n"), "- [ ] one\n\n- [x] two");
}

fn observe_spans(spans: &[MdSpan], observed: &mut BTreeSet<&'static str>) {
    for span in spans {
        match span {
            MdSpan::Text(_) => {
                observed.insert("span.text");
            }
            MdSpan::Bold(inner) => {
                observed.insert("span.bold");
                observe_spans(inner, observed);
            }
            MdSpan::Italic(inner) => {
                observed.insert("span.italic");
                observe_spans(inner, observed);
            }
            MdSpan::Code(_) => {
                observed.insert("span.code");
            }
            MdSpan::Link { children, .. } => {
                observed.insert("span.link");
                observe_spans(children, observed);
            }
            MdSpan::Strikethrough(inner) => {
                observed.insert("span.strikethrough");
                observe_spans(inner, observed);
            }
            MdSpan::SoftBreak => {
                observed.insert("span.soft_break");
            }
            MdSpan::HardBreak => {
                observed.insert("span.hard_break");
            }
            MdSpan::ParagraphBreak => {
                observed.insert("span.paragraph_break");
            }
            MdSpan::Footnote(_) => {
                observed.insert("span.footnote");
            }
            MdSpan::Html(_) => {
                observed.insert("span.html");
            }
            MdSpan::Image { .. } => {
                observed.insert("span.image");
            }
        }
    }
}

fn observe_items(items: &[ListItem], observed: &mut BTreeSet<&'static str>) {
    for item in items {
        observed.insert(match item.checked {
            Some(true) => "list.task.checked",
            Some(false) => "list.task.unchecked",
            None => "list.plain",
        });
        observe_spans(&item.spans, observed);
        observe_blocks(&item.children, observed);
    }
}

fn observe_blocks(blocks: &[MdBlock], observed: &mut BTreeSet<&'static str>) {
    for block in blocks {
        match block {
            MdBlock::Heading { spans, .. } => {
                observed.insert("block.heading");
                observe_spans(spans, observed);
            }
            MdBlock::Paragraph(spans) => {
                observed.insert("block.paragraph");
                observe_spans(spans, observed);
            }
            MdBlock::CodeBlock { lang, .. } => {
                observed.insert("block.code");
                if lang.is_some() {
                    observed.insert("code.language");
                }
            }
            MdBlock::BulletList { items, loose } => {
                observed.insert("block.bullet_list");
                observed.insert(if *loose { "list.loose" } else { "list.tight" });
                observe_items(items, observed);
            }
            MdBlock::OrderedList {
                start,
                items,
                loose,
            } => {
                observed.insert("block.ordered_list");
                observed.insert(if *loose { "list.loose" } else { "list.tight" });
                if *start != 1 {
                    observed.insert("ordered.custom_start");
                }
                observe_items(items, observed);
            }
            MdBlock::Blockquote(spans) => {
                observed.insert("block.blockquote");
                observe_spans(spans, observed);
            }
            MdBlock::Rule => {
                observed.insert("block.rule");
            }
            MdBlock::Table { header, rows } => {
                observed.insert("block.table");
                for cell in header.iter().chain(rows.iter().flatten()) {
                    observe_spans(cell, observed);
                }
            }
            MdBlock::FootnoteDefinition { spans, .. } => {
                observed.insert("block.footnote_definition");
                observe_spans(spans, observed);
            }
            MdBlock::HtmlBlock(_) => {
                observed.insert("block.html");
            }
            MdBlock::Mermaid { .. } => {
                observed.insert("block.mermaid");
            }
        }
    }
}

/// Every parser feature that the renderer handles must be produced by at
/// least one fixture. The exhaustive visitors make a new IR variant update
/// this inventory at compile time; the semantic entries cover states such
/// as task markers and looseness that are fields rather than variants.
#[test]
fn fixtures_cover_every_markdown_ir_variant_and_semantic_state() {
    const FIXTURES: &[&str] = &[
        r#"# heading

paragraph **bold** *italic* `code` [link](https://example.invalid) ~~strike~~ [^note] <span>html</span> ![alt](missing.png)
soft
break\
hard

> quote one
>
> quote two

---

| head |
| --- |
| cell |

```rust
let value = 1;
```

```mermaid
graph TD
A-->B
```

<div>block html</div>

[^note]: footnote one

    footnote two
"#,
        "- plain\n- [ ] open\n- [x] done\n",
        "- loose one\n\n- loose two\n",
        "3. ordered\n4. next\n",
    ];
    const EXPECTED: &[&str] = &[
        "block.blockquote",
        "block.bullet_list",
        "block.code",
        "block.footnote_definition",
        "block.heading",
        "block.html",
        "block.mermaid",
        "block.ordered_list",
        "block.paragraph",
        "block.rule",
        "block.table",
        "code.language",
        "list.loose",
        "list.plain",
        "list.task.checked",
        "list.task.unchecked",
        "list.tight",
        "ordered.custom_start",
        "span.bold",
        "span.code",
        "span.footnote",
        "span.hard_break",
        "span.html",
        "span.image",
        "span.italic",
        "span.link",
        "span.paragraph_break",
        "span.soft_break",
        "span.strikethrough",
        "span.text",
    ];

    let mut observed = BTreeSet::new();
    for fixture in FIXTURES {
        observe_blocks(
            &parse_markdown(fixture, "base16-ocean.dark", false),
            &mut observed,
        );
    }

    assert_eq!(observed, EXPECTED.iter().copied().collect());
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
    let mut blocks = vec![MdBlock::Paragraph(vec![MdSpan::Link {
        children: vec![MdSpan::Bold(vec![MdSpan::Image {
            url: "n.png".to_owned(),
            alt: String::new(),
            raster: None,
        }])],
        url: "https://example.com".to_owned(),
    }])];
    let mut count = 0;
    resolve_images(&mut blocks, &mut |_| {
        count += 1;
        None
    });
    assert_eq!(count, 1);
}
