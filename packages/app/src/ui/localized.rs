//! Widgets whose text the app's locale owns. `daruda_ui` takes those strings
//! as arguments; these wrappers fill them from `surface::strings` so call
//! sites stay one line.

use gpui::{App, ElementId, IntoElement, SharedString, Window};

use super::{Button, CopyTooltips, Markdown, button_icon, button_icon_danger, copy_button, icons};
use crate::surface::strings as s;

/// Neutral dismissal, distinct from deleting an item.
pub fn button_close(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon(id, icons::CLOSE, cx).tooltip(s::common::btn_close())
}

/// Delete action; the caller owns any row-hover visibility gating.
pub fn button_delete_glyph(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon_danger(id, icons::DELETE, cx).tooltip(s::common::btn_delete())
}

/// A copy-to-clipboard button for a rendered code block. `id` must be stable
/// per block across renders, or the keyed ✓ feedback state resets.
pub fn code_copy_button<I: Into<ElementId>>(
    id: I,
    code: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<I> {
    let tooltips = copy_tooltips();
    copy_button(
        id,
        code,
        icons::icon(icons::COPY),
        tooltips.copy,
        icons::icon(icons::CHECK),
        tooltips.copied,
        window,
        cx,
    )
}

/// Rendered, drag-selectable Markdown whose code blocks carry a copy button.
pub fn markdown(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Markdown {
    daruda_ui::markdown(id, text, copy_tooltips())
}

fn copy_tooltips() -> CopyTooltips {
    CopyTooltips {
        copy: s::code_block::copy().into(),
        copied: s::code_block::copied().into(),
    }
}
