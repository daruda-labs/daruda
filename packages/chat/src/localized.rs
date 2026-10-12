//! The chat's own wording on `daruda_ui`'s label-taking widgets.

use daruda_ui::{Button, CopyTooltips, Markdown, button_icon, copy_button, icons};
use gpui::{App, ElementId, IntoElement, SharedString, Window};

use crate::strings as s;

/// Close action, tooltipped in the chat's own words.
pub(crate) fn button_close(id: impl Into<ElementId>, cx: &App) -> Button {
    button_icon(id, icons::CLOSE, cx).tooltip(s::agent_chat::close())
}

/// A copy-to-clipboard button for a rendered code block. `id` must be stable
/// per block across renders, or the keyed ✓ feedback state resets.
pub(crate) fn code_copy_button<I: Into<ElementId>>(
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
pub(crate) fn markdown(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Markdown {
    daruda_ui::markdown(id, text, copy_tooltips())
}

fn copy_tooltips() -> CopyTooltips {
    CopyTooltips {
        copy: s::agent_chat::code_copy().into(),
        copied: s::agent_chat::code_copied().into(),
    }
}
