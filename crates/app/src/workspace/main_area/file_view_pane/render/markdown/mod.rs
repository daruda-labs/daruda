//! Markdown preview renderer — block + span tree → GPUI elements.
//!
//! This file owns only what every level needs: the colours ([`MdColors`]), the
//! link-opening callback, and the walker over a document's blocks. The work
//! itself is split by level — [`block`] renders one [`MdBlock`], [`inline`]
//! shapes one run of spans on top of the pure compiler in [`prose`], and
//! [`image`] sizes and rasterizes. [`selection`] holds the click/drag state the
//! walker and [`inline`] both touch, because a link and a block selection
//! compete for the same press.

mod block;
mod image;
mod inline;
mod prose;
mod selection;

#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use gpui::{App, Context, IntoElement, Window, div, prelude::*, px};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::CharSelection;
use crate::workspace::main_area::file_view_pane::images::MdImages;
use crate::workspace::main_area::file_view_pane::markdown_viewer::MdBlock;

use self::block::render_md_block;
pub(in crate::workspace) use self::image::CachedImage;
use self::prose::is_openable_markdown_url;
use self::selection::{block_with_selection, is_block_selected};

/// The colours this preview paints with, resolved against the surface it is
/// painted on.
///
/// The preview sits on `file_viewer_pane_bg`, which mirrors the *terminal*
/// palette (DESIGN.md §AgentChatPane — the file viewer shares that surface so a
/// file opened from a transcript does not land on an unrelated one). Every
/// colour therefore comes from [`PaneSurfaceTokens`], never from the UI theme:
/// `ui_preset` and `terminal_preset` are independent, so a light UI theme over
/// a dark terminal previously painted `#2d2d2d` prose onto a near-black pane
/// while the table and code fills stayed light — the body vanished and the
/// blocks inverted.
///
/// The semantic `SUCCESS` hue for a ticked task box stays outside this surface
/// palette. Inline code uses the pane's own fill and foreground roles.
struct MdColors {
    /// Body prose, and headings — a terminal-mirrored surface has no tone
    /// above its own foreground, so heading rank is carried by size and weight
    /// (the same way the agent-chat markdown renders them).
    text: gpui::Hsla,
    /// Blockquote prose, its bar, and list bullets.
    muted: gpui::Hsla,
    /// Footnotes, HTML passthrough, strikethrough.
    subtle: gpui::Hsla,
    link: gpui::Hsla,
    /// The weaker of the two fills — code block, table body rows (the UI
    /// theme's `BG_PANEL` rung).
    fill: gpui::Hsla,
    /// The stronger fill, one step above [`Self::fill`] — table header (the
    /// `BG_RAISED` rung).
    raised: gpui::Hsla,
    /// Code-block border, table lines, the `<hr>` rule.
    line: gpui::Hsla,
}

type OpenUrl = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// What every level of the walk needs and neither level decides: the colours,
/// and the pane's GPU image table an `MdSpan::Image` / `MdBlock::Mermaid`
/// slot indexes into. Borrowed as one parameter instead of threaded through
/// `render_md_block` → `render_list` → `render_md_prose` → `render_prose_run`
/// → `render_inline_md_image` twice over. Read-only here: the table is built
/// by the load funnel in `file_view_pane/images.rs`.
#[derive(Clone, Copy)]
struct MdRenderAssets<'a> {
    t: &'a MdColors,
    images: &'a MdImages,
}

impl MdColors {
    fn for_pane(cx: &App) -> Self {
        let tokens = PaneSurfaceTokens::file_viewer(cx);
        Self {
            text: tokens.foreground,
            muted: tokens.foreground_muted,
            subtle: tokens.foreground_subtle,
            link: theme::file_viewer_pane_link_color(cx),
            fill: tokens.tint,
            raised: tokens.active_tint,
            line: tokens.border_tint,
        }
    }
}

/// Top-level Markdown body: a padded column of selectable blocks.
pub(super) fn render_md_body(
    blocks: &[MdBlock],
    images: &MdImages,
    char_selection: Option<&CharSelection>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let t = MdColors::for_pane(cx);
    // Boundary check, not a second decision: `compile_prose` already withholds
    // the click from anything unopenable, so this only guards the handoff to
    // the OS for a caller that reaches it another way.
    let open_url: OpenUrl = Rc::new(|url, _window, cx| {
        if is_openable_markdown_url(url) {
            cx.open_url(url);
        }
    });
    render_md_body_layout(
        blocks,
        char_selection,
        MdRenderAssets { t: &t, images },
        open_url,
        |block, block_idx| block_with_selection(block, block_idx, cx),
    )
}

/// Layout half of [`render_md_body`]. Selection listeners are supplied by the
/// host so layout probes can exercise this exact path without constructing a
/// `Workspace` merely to obtain its `Context`.
fn render_md_body_layout(
    blocks: &[MdBlock],
    char_selection: Option<&CharSelection>,
    assets: MdRenderAssets<'_>,
    on_open_url: OpenUrl,
    mut decorate_block: impl FnMut(gpui::Div, usize) -> gpui::Div,
) -> gpui::Div {
    let body_text = assets.t.text;
    let block_sel_bg = theme::SELECTION_BG;
    let mut col = div()
        .flex()
        .flex_col()
        .px(px(theme::MD_BODY_PAD_X))
        .py(px(theme::MD_BODY_PAD_Y))
        .gap(px(theme::MD_BLOCK_GAP))
        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
        .text_color(body_text);

    for (i, block) in blocks.iter().enumerate() {
        let is_sel = is_block_selected(char_selection, i);
        let mut next_part_idx = 0;
        let block_el = div()
            .rounded(px(theme::MD_BLOCK_RADIUS))
            .when(is_sel, |d| d.bg(block_sel_bg))
            .child(render_md_block(
                block,
                assets,
                i,
                &mut next_part_idx,
                &on_open_url,
            ));
        col = col.child(decorate_block(block_el, i));
    }
    col
}
