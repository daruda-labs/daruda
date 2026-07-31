//! Lightbox dialog for a mermaid diagram: the bitmap at its natural logical
//! size (no fit-to-pane shrink) inside a pannable scroll container. The 2×
//! raster keeps it crisp at this size. Opened from the diagram card's zoom
//! button; Dialog supplies Escape/outside-click close, while the body owns the
//! visible close affordance.

use gpui::{
    App, AppContext as _, ClickEvent, Context, FocusHandle, Focusable, ParentElement as _, Window,
    div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{WindowExt as _, button_close};
use crate::workspace::main_area::file_view_pane::render::CachedImage;
use crate::workspace::modal_view::ModalView;

/// Dialog width: the diagram's natural width plus the card padding on both
/// sides, clamped to the viewport fraction so an oversized diagram pans
/// inside the body instead of overflowing the window.
fn lightbox_width(image_logical_width: f32, viewport_width: f32) -> f32 {
    let frac = theme::MERMAID_LIGHTBOX_VIEWPORT_FRACTION;
    (image_logical_width + 2.0 * theme::AGENT_CHAT_DIAGRAM_PAD).min(viewport_width * frac)
}

/// Body scroll height after reserving room for the in-modal close row.
fn lightbox_body_max_height(viewport_height: f32) -> f32 {
    let frac = theme::MERMAID_LIGHTBOX_VIEWPORT_FRACTION;
    let reserved = theme::PANE_HEADER_CLOSE_H + theme::GAP_STANDARD;
    (viewport_height * frac - reserved).max(theme::PANE_HEADER_CLOSE_H)
}

/// Vertically center the 90%-viewport lightbox enough that its own max body
/// height has bottom breathing room after Dialog's absolute positioning.
fn lightbox_margin_top(viewport_height: f32) -> f32 {
    viewport_height * (1.0 - theme::MERMAID_LIGHTBOX_VIEWPORT_FRACTION) / 2.0
}

pub(super) fn open(image: &CachedImage, window: &mut Window, cx: &mut App) {
    let viewport = window.viewport_size();
    let width = lightbox_width(image.logical_width(), f32::from(viewport.width));
    let margin_top = lightbox_margin_top(f32::from(viewport.height));
    let image = image.clone();
    let entity = cx.new(|cx_modal| MermaidLightbox {
        image,
        focus_handle: cx_modal.focus_handle(),
    });
    let entity_for_focus = entity.clone();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .child(entity.clone())
            .close_button(false)
            .overlay_closable(true)
            .p(px(0.0))
            .margin_top(px(margin_top))
            .width(px(width))
    });

    let handle = entity_for_focus.read(cx).focus_handle(cx);
    let wh = window.window_handle();
    cx.defer(move |cx| {
        let _ = cx.update_window(wh, |_, window, cx| window.focus(&handle, cx));
    });
}

pub(in crate::workspace) struct MermaidLightbox {
    image: CachedImage,
    focus_handle: FocusHandle,
}

impl MermaidLightbox {
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
    }
}

impl Render for MermaidLightbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let max_h = lightbox_body_max_height(f32::from(window.viewport_size().height));
        let image_w = self.image.logical_width();
        div()
            .key_context("MermaidLightbox")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .gap(px(theme::GAP_STANDARD))
            .child(
                div()
                    .id("mermaid-lightbox-header")
                    .flex()
                    .flex_row()
                    .justify_end()
                    .child(
                        button_close("mermaid-lightbox-close", cx)
                            .tooltip(s::error_modal_button_close())
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.dismiss(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("mermaid-lightbox-body")
                    .max_h(px(max_h))
                    .overflow_scroll()
                    .child(div().flex_none().w(px(image_w)).child(self.image.natural())),
            )
    }
}

impl Focusable for MermaidLightbox {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for MermaidLightbox {}

#[cfg(test)]
mod tests {
    use super::{lightbox_body_max_height, lightbox_margin_top, lightbox_width};
    use crate::ui::theme;

    #[test]
    fn width_fits_image_plus_padding_when_under_viewport_fraction() {
        let width = lightbox_width(400.0, 2000.0);
        assert_eq!(width, 400.0 + 2.0 * theme::AGENT_CHAT_DIAGRAM_PAD);
    }

    #[test]
    fn width_clamps_to_viewport_fraction_for_an_oversized_diagram() {
        let viewport_width = 1200.0;
        let width = lightbox_width(5000.0, viewport_width);
        assert_eq!(
            width,
            viewport_width * theme::MERMAID_LIGHTBOX_VIEWPORT_FRACTION
        );
    }

    #[test]
    fn body_height_reserves_room_for_close_row() {
        let viewport_height = 1000.0;
        let height = lightbox_body_max_height(viewport_height);
        assert_eq!(
            height,
            viewport_height * theme::MERMAID_LIGHTBOX_VIEWPORT_FRACTION
                - theme::PANE_HEADER_CLOSE_H
                - theme::GAP_STANDARD
        );
    }

    #[test]
    fn margin_top_centers_the_viewport_fraction() {
        let viewport_height = 1000.0;
        assert_eq!(
            lightbox_margin_top(viewport_height),
            viewport_height * (1.0 - theme::MERMAID_LIGHTBOX_VIEWPORT_FRACTION) / 2.0
        );
    }

    #[test]
    fn margin_and_body_height_leave_bottom_room() {
        let viewport_height = 1000.0;
        let content_height = theme::PANE_HEADER_CLOSE_H
            + theme::GAP_STANDARD
            + lightbox_body_max_height(viewport_height);
        assert!(lightbox_margin_top(viewport_height) + content_height < viewport_height);
    }
}
