//! A read-only editor embedded in a tool card, height-capped so `InputState`
//! shapes and paints only its visible rows, plus the thumb + copy chrome that
//! capping makes necessary.

use gpui::{
    AnyElement, App, ElementId, Entity, Hsla, IntoElement, Pixels, RenderOnce, SharedString,
    Window, div, point, prelude::*, px,
};

use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::output_editor::{
    bounded_embed_height, embed_text_height,
};

/// Embed `editor` at [`bounded_embed_height`], with its own scrollbar thumbs and
/// — when `copy_source` is present — a hover-revealed copy button. Shared by the
/// verbatim tool-output block and the per-file diff block. `id` keys every
/// element inside and must be stable and unique per embed.
///
/// Reaches past `render` so `workspace/tests/agent_output_layout.rs` measures
/// this builder itself; a probe that re-declares the element tree cannot fail
/// when a height bound is lost here.
pub(in crate::workspace) fn bounded_editor_embed(
    id: &str,
    editor: &Entity<crate::ui::InputState>,
    copy_source: Option<SharedString>,
    max_rows: usize,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> AnyElement {
    let rows = editor.read(cx).display_rows().max(1);
    // The bound is the whole point: `calculate_visible_range`
    // (`gpui_component/src/input/element.rs`) derives the shaped row range from
    // the painted height, so an unbounded embed shapes every row on every paint.
    let row_height = theme::agent_chat_embed_row_height(cx);
    let height = bounded_embed_height(rows, max_rows, row_height);
    // `scroll_handle().bounds()` / `.max_offset()` are permanently zero for this
    // element (see `InputState::scroll_size`'s doc comment), so the viewport
    // extent comes from `last_bounds()` — the same pairing the File viewer's
    // thumb uses.
    let (viewport, content_w, offset) = {
        let state = editor.read(cx);
        (
            state.last_bounds().map(|b| b.size).unwrap_or_default(),
            state.scroll_size().width,
            state.scroll_handle().offset(),
        )
    };
    // The vertical thumb measures the text extent, not `scroll_size().height`:
    // in code-editor mode `gpui_component`'s `element.rs` pads the scrollable
    // height by half a viewport, so that value always overflows and would draw a
    // thumb even when the cap hid nothing. The width carries no such pad.
    let content_h = embed_text_height(rows, row_height);
    // The text element owns wheel gestures in capture while its offset changes,
    // then lets them bubble at the extremes so the transcript can keep chaining.
    let group = SharedString::from(format!("agent-chat-out-{id}"));
    let surface = theme::dim_toward_gray(theme::agent_chat_bg(cx), dim);
    div()
        .relative()
        .flex()
        .w_full()
        // `flex_none` is load-bearing: the chat list lays rows out at min-content
        // height (gpui `list.rs` `available_item_space`), so a shrinkable item
        // would let the row undercount the embed and clip it — see
        // `render/diff.rs`'s container for the full reasoning.
        .flex_none()
        .h(height)
        .group(group.clone())
        .debug_selector(|| format!("agent-chat-out-embed-{id}"))
        // Opaque terminal-preset surface (not the UI theme's editor bg) — the
        // background `embedded_code_viewer` derives its fallback text colour
        // from, and the tool card chrome already tracks.
        .bg(surface)
        // Wrapper and `Input` carry the height redundantly: either alone
        // delivers the bound (`Input::render`'s `h_auto()` stretches to a
        // definite-height parent), setting neither collapses it to one row.
        .child(crate::ui::embedded_code_viewer(editor, surface, cx).h(height))
        // Rows past the cap are reachable only by scrolling inside the embed, so
        // the embed needs a vertical thumb as well as the horizontal one
        // `soft_wrap(false)` calls for.
        .children(
            crate::ui::scrollbar::vertical_thumb(
                format!("agent-chat-out-vthumb-{id}"),
                viewport.height,
                content_h,
                offset.y,
                px(0.),
                t.scrollbar_thumb,
                t.file_viewer_scrollbar_thumb_hover,
            )
            .map(|thumb| thumb.on_drag(drag_axis(editor, Axis::Vertical))),
        )
        .children(
            crate::ui::scrollbar::horizontal_thumb(
                format!("agent-chat-out-hthumb-{id}"),
                viewport.width,
                content_w,
                offset.x,
                t.scrollbar_thumb,
                t.file_viewer_scrollbar_thumb_hover,
            )
            .map(|thumb| thumb.on_drag(drag_axis(editor, Axis::Horizontal))),
        )
        .children(copy_source.map(|code| CopyOverlay {
            group,
            id: ElementId::Name(format!("agent-chat-out-copy-{id}").into()),
            code,
            chip_bg: surface,
        }))
        .into_any_element()
}

/// Which axis a [`drag_axis`] handler writes.
#[derive(Clone, Copy)]
enum Axis {
    Vertical,
    Horizontal,
}

/// Add a dragged thumb's scroll delta to `editor` on `axis`, leaving the other
/// axis where it was. Reads the offset live rather than closing over a
/// render-time copy, so moves that arrive between two paints accumulate instead
/// of each overwriting the last. `set_scroll_offset` owns the clamp and the
/// repaint, so the host never has to know the editor's content extent.
fn drag_axis(
    editor: &Entity<crate::ui::InputState>,
    axis: Axis,
) -> impl Fn(Pixels, &mut Window, &mut App) + use<> {
    let editor = editor.clone();
    move |delta, _window, cx| {
        editor.update(cx, |state, cx| {
            let current = state.scroll_handle().offset();
            let offset = match axis {
                Axis::Vertical => point(current.x, current.y + delta),
                Axis::Horizontal => point(current.x + delta, current.y),
            };
            state.set_scroll_offset(offset, cx);
        });
    }
}

/// Hover-revealed copy button over an embed, so replacing a markdown-rendered
/// fence with an editor does not drop the copy affordance that fence had.
///
/// A `RenderOnce` wrapper because [`crate::ui::code_copy_button`] needs a
/// `&mut Window` and [`bounded_editor_embed`] is a `&App`-only builder; the
/// wrapper defers the button until its own `render`, which is handed one. The
/// markdown path needs no wrapper — gpui_component invokes its
/// `code_block_actions` callback with a live window already.
#[derive(IntoElement)]
struct CopyOverlay {
    group: SharedString,
    id: ElementId,
    code: SharedString,
    /// Opaque fill behind the icon so it never sits on a code row, matching the
    /// chip the vendored code-block actions overlay paints (`text/node.rs`).
    /// The embed's own surface, since the tint tokens are translucent and would
    /// leave the text legible underneath.
    chip_bg: Hsla,
}

impl RenderOnce for CopyOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .top_1()
            .right_1()
            .invisible()
            .group_hover(self.group, |s| s.visible())
            .bg(self.chip_bg)
            .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
            .child(crate::ui::code_copy_button(self.id, self.code, window, cx))
    }
}
