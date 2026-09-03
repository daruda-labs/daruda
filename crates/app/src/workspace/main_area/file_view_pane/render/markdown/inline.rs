//! One run of inline spans → shaped text.
//!
//! Everything a paragraph's worth of spans becomes: the compiled `StyledText`,
//! its clickable link ranges, the images that interrupt it, and the style each
//! run paints with.

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, InteractiveText, IntoElement, MouseButton,
    StrikethroughStyle, StyledText, div, prelude::*, px,
};

use crate::ui::theme;
use crate::workspace::main_area::file_view_pane::markdown_viewer::MdSpan;

use super::image::{ImageLayout, render_md_image};
use super::prose::{CompiledText, InlineImage, InlineStyle, ProsePart, compile_prose};
use super::selection::{
    cancel_pending_block_selection, record_markdown_mouse_button, take_markdown_mouse_button,
};
use super::{MdColors, MdRenderAssets, OpenUrl};

/// A run of spans as a block column of wrapping rows, one per paragraph.
///
/// The split is what makes a multi-paragraph item work. `flex_1().w_0()` is the
/// shape zed gives prose beside a bullet cell (`push_markdown_list_item`).
///
// WORKAROUND: gpui's text-measure cache (`elements/text.rs`, the
// `wrap_width.is_none() || ..` arm) returns a size measured at a *definite*
// wrap width for an unconstrained measure, against its own documented rule, so
// a zero-width probe poisons every later max-content query. A flex column here
// makes that probe decide the layout — text beside an inline image collapsed to
// one character per line. Block layout stacks the runs the same way without
// making the probe decisive; fixing the cache belongs in a `patches/` gpui
// patch, whose blast radius is every text element in the app.
pub(super) fn render_md_prose(
    spans: &[MdSpan],
    assets: MdRenderAssets<'_>,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> gpui::Div {
    let mut prose = div().flex_1().w_0();
    for (i, run) in spans
        .split(|span| matches!(span, MdSpan::ParagraphBreak))
        .enumerate()
    {
        // Block layout has no `gap`; a leading margin spaces every run but the
        // first without needing to know which one is last.
        prose = prose.child(
            render_prose_run(run, assets, block_idx, next_part_idx, on_open_url)
                .when(i > 0, |d| d.mt(px(theme::MD_BLOCK_GAP))),
        );
    }
    prose
}

pub(super) fn render_prose_run(
    spans: &[MdSpan],
    assets: MdRenderAssets<'_>,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> gpui::Div {
    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .w_full()
        .min_w_0()
        .items_center()
        .whitespace_normal();
    let parts = compile_prose(spans);
    let text_fills_row = matches!(parts.as_slice(), [ProsePart::Text(_)]);
    for part in parts {
        let part_idx = *next_part_idx;
        *next_part_idx += 1;
        row = match part {
            ProsePart::Text(text) => row.child(render_compiled_text(
                text,
                assets.t,
                block_idx,
                part_idx,
                text_fills_row,
                on_open_url,
            )),
            ProsePart::Image(image) => row.child(render_inline_md_image(
                image,
                assets,
                block_idx,
                part_idx,
                on_open_url,
            )),
        };
    }
    row
}

fn render_compiled_text(
    compiled: CompiledText,
    t: &MdColors,
    block_idx: usize,
    part_idx: usize,
    fill_width: bool,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let CompiledText {
        text,
        style_runs,
        link_ranges,
        link_urls,
    } = compiled;
    let highlights = style_runs
        .iter()
        .map(|run| (run.range.clone(), highlight_for(run.style, t)))
        .collect::<Vec<_>>();
    let monospace_ranges = style_runs
        .iter()
        .filter(|run| run.style.code || run.style.html)
        .map(|run| (run.range.clone(), gpui::SharedString::from("monospace")))
        .collect::<Vec<_>>();
    let styled = StyledText::new(text)
        .with_highlights(highlights)
        .with_font_family_overrides(monospace_ranges);

    let text: AnyElement = if link_ranges.is_empty() {
        styled.into_any_element()
    } else {
        let on_open_url = on_open_url.clone();
        let interactive =
            InteractiveText::new(format!("markdown-prose-{block_idx}-{part_idx}"), styled)
                .on_click(link_ranges, move |range_idx, window, cx| {
                    let is_primary = take_markdown_mouse_button(cx) == Some(MouseButton::Left);
                    if !is_primary {
                        return;
                    }
                    cancel_pending_block_selection(cx);
                    if let Some(url) = link_urls.get(range_idx) {
                        on_open_url(url, window, cx);
                    }
                });
        div()
            .capture_any_mouse_down(|event, _, cx| {
                record_markdown_mouse_button(event.button, cx);
            })
            .child(interactive)
            .into_any_element()
    };

    div()
        .when(fill_width, |d| d.flex_1().w_0())
        .min_w_0()
        .whitespace_normal()
        .when(cfg!(test), |d| d.debug_selector(|| "md-plain".into()))
        .child(text)
        .into_any_element()
}

fn render_inline_md_image(
    inline: InlineImage<'_>,
    assets: MdRenderAssets<'_>,
    block_idx: usize,
    part_idx: usize,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let image = render_md_image(
        assets.images.get(inline.slot),
        inline.alt,
        ImageLayout::Inline,
        assets.t,
    );
    let Some(url) = inline.link_url else {
        return image;
    };

    let url = url.to_owned();
    let on_open_url = on_open_url.clone();
    div()
        .id(format!("markdown-image-link-{block_idx}-{part_idx}"))
        .cursor_pointer()
        .when(cfg!(test), |d| {
            d.debug_selector(|| "markdown-linked-image".into())
        })
        .on_click(move |_, window, cx| {
            cancel_pending_block_selection(cx);
            on_open_url(&url, window, cx);
        })
        .child(image)
        .into_any_element()
}

pub(super) fn highlight_for(style: InlineStyle, t: &MdColors) -> HighlightStyle {
    // Link wins over code: a code span inside a link (`[`gpui`](https://…)`)
    // has to read as a link, because `underline` is deliberately off
    // (DESIGN.md) and colour is then the only cue. Reaching here at all means
    // `compile_prose` judged the URL openable — an unopenable one arrives with
    // `link` already cleared, so this never colours a dead click.
    let color = if style.link {
        Some(t.link)
    } else if style.code {
        Some(t.text)
    } else if style.strikethrough || style.footnote || style.html {
        Some(t.subtle)
    } else {
        None
    };
    HighlightStyle {
        color,
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(FontStyle::Italic),
        background_color: style.code.then_some(t.fill),
        underline: None,
        strikethrough: style.strikethrough.then_some(StrikethroughStyle {
            thickness: px(theme::MD_STRIKETHROUGH_H),
            color: None,
        }),
        fade_out: None,
    }
}
