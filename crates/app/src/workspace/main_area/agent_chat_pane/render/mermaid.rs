//! Mermaid diagram embeds for the chat transcript: the shared card chrome
//! (translucent tint + hairline, matching tool cards / code blocks), the
//! markdown `code_block_render` hook, and the hover zoom / copy affordances.
//! The diagram bitmap itself is also clickable, opening the same lightbox
//! as the zoom button — the button stays for discoverability (icon +
//! hover reveal) and to keep a target that isn't the whole card.
//! Shared by the assistant prose body and tool-output markdown.

use gpui::{AnyElement, App, ElementId, IntoElement, SharedString, Window, div, prelude::*, px};

use super::MermaidImages;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{IconName, button_bare, copy_button};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::mermaid_key;
use crate::workspace::main_area::file_view_pane::render::CachedImage;

/// The `code_block_render` hook for a chat markdown body: replace a
/// ```mermaid fence with its cached diagram card, leaving every other code
/// block (and a not-yet-rasterized mermaid fence) to the default code
/// rendering by returning `None`.
pub(super) fn mermaid_code_block_render(
    mermaid_images: &MermaidImages,
    dim: f32,
) -> impl Fn(&str, &str, &mut gpui::Window, &mut gpui::App) -> Option<AnyElement> + Send + Sync + 'static
{
    let images = mermaid_images.clone();
    move |lang, source, _window, cx| mermaid_fence_element(&images, lang, source, dim, cx)
}

/// Shared mermaid-fence branch for both markdown hooks (assistant prose and
/// tool-output): `None` unless `lang == "mermaid"` and its rasterized image is
/// already cached (a not-yet-rasterized fence falls through to the caller's
/// own default rendering). Keyed by the host appearance — the same source
/// `reconcile_mermaid` keys inserts by (see `AgentChatView::host_is_dark`).
pub(super) fn mermaid_fence_element(
    images: &MermaidImages,
    lang: &str,
    source: &str,
    dim: f32,
    cx: &App,
) -> Option<AnyElement> {
    if lang != "mermaid" {
        return None;
    }
    let dark = !crate::ui::theme::agent_chat_syntax_is_light(cx);
    let key = mermaid_key(source, dark);
    let image = images.lock().ok()?.get(&key).cloned()?;
    Some(mermaid_diagram_card(key, source, &image, dim, cx))
}

/// One diagram embed: the card chrome (same tint/hairline/radius family as
/// the tool cards and the user bubble), the bitmap, and the hover-revealed
/// zoom (opens the lightbox) and copy-source buttons. The diagram canvas is
/// transparent, so the card tint shows through it.
pub(super) fn mermaid_diagram_card(
    key: u64,
    source: &str,
    image: &CachedImage,
    dim: f32,
    cx: &App,
) -> AnyElement {
    let group = SharedString::from(format!("mermaid-{key}"));
    let image_for_click = image.clone();
    div()
        .relative()
        .group(group.clone())
        .mb(px(theme::AGENT_CHAT_DIAGRAM_GAP))
        .p(px(theme::AGENT_CHAT_DIAGRAM_PAD))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
        .border_1()
        .border_color(theme::dim_toward_gray(
            theme::agent_chat_border_tint(cx),
            dim,
        ))
        .child(
            div()
                .id(SharedString::from(format!("mermaid-open-{key}")))
                .debug_selector(|| format!("mermaid-open-{key}"))
                // Sized to the bitmap, not stretched to the card: the blank
                // strip beside a narrow diagram is not part of the diagram, so
                // clicking it must not open the lightbox. `max_w_full` keeps a
                // diagram wider than the card shrinking with its image.
                .w(px(image.logical_width()))
                .max_w_full()
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    super::mermaid_lightbox::open(&image_for_click, window, cx);
                })
                .child(image.block_diagram()),
        )
        .child(DiagramActions {
            key,
            group,
            source: SharedString::from(source.to_owned()),
            image: image.clone(),
        })
        .into_any_element()
}

/// The card's hover-revealed zoom + copy row.
///
/// A `RenderOnce` wrapper because [`crate::ui::copy_button`] needs a
/// `&mut Window` and [`mermaid_diagram_card`] is an `&App`-only builder — the
/// same shape `super::embed::CopyOverlay` uses for the embed copy button.
#[derive(IntoElement)]
struct DiagramActions {
    key: u64,
    group: SharedString,
    source: SharedString,
    image: CachedImage,
}

impl RenderOnce for DiagramActions {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.key;
        div()
            .debug_selector(|| format!("mermaid-actions-{key}"))
            .absolute()
            .top_1()
            .right_1()
            .flex()
            .flex_row()
            .gap_1()
            .invisible()
            .group_hover(self.group, |s| s.visible())
            .child(
                button_bare(SharedString::from(format!("mermaid-zoom-{key}")))
                    .icon(IconName::Maximize)
                    .tooltip(s::agent_chat_diagram_zoom())
                    .on_click(move |_, window, cx| {
                        // This row floats over the diagram, and gpui hit-tests
                        // every hitbox under the pointer — without this the
                        // button also runs the diagram's own open handler and
                        // stacks a second lightbox. `occlude()` would instead
                        // break the group hover this row's visibility depends
                        // on (`group_hover` resolves through the group
                        // hitbox), hiding the buttons on approach.
                        cx.stop_propagation();
                        super::mermaid_lightbox::open(&self.image, window, cx);
                    }),
            )
            // `copy_button` already stops propagation for the same reason, and
            // adds the ✓ feedback every other daruda copy affordance has.
            .child(copy_button(
                ElementId::from(SharedString::from(format!("mermaid-copy-{key}"))),
                self.source,
                IconName::Copy.into(),
                SharedString::from(s::agent_chat_diagram_copy()),
                IconName::Check.into(),
                SharedString::from(s::agent_chat_diagram_copied()),
                window,
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, TestAppContext, VisualTestContext, Window, point, px};

    use super::*;
    use crate::test_support::init_gpui_component;
    use crate::ui::WindowExt as _;
    use crate::workspace::main_area::file_view_pane::visual::RasterImage;

    const KEY: u64 = 7;
    const SOURCE: &str = "flowchart TD\n  A[hello]\n";
    /// Card width for the probe. Deliberately far wider than the probe bitmap
    /// (100 logical px) so the blank strip beside the diagram is clickable.
    const CARD_W: f32 = 400.0;

    fn selector(name: &str) -> &'static str {
        Box::leak(format!("{name}-{KEY}").into_boxed_str())
    }

    /// The action row is two equal buttons plus one gap, so the quarter and
    /// three-quarter points land inside the first and last button whatever
    /// size the theme gives them.
    fn nth_quarter(row: gpui::Bounds<gpui::Pixels>, quarter: f32) -> gpui::Point<gpui::Pixels> {
        point(row.left() + row.size.width * quarter, row.center().y)
    }

    struct CardProbe {
        image: CachedImage,
    }

    impl gpui::Render for CardProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| "mermaid-card".into())
                .w(px(CARD_W))
                .child(mermaid_diagram_card(KEY, SOURCE, &self.image, 0.0, cx))
        }
    }

    /// Mount the real card in a `Root`-wrapped window and hand back its painted
    /// bounds. Nothing is hovered yet, so the action row is still invisible.
    fn mounted_card(cx: &mut TestAppContext) -> (VisualTestContext, gpui::Bounds<gpui::Pixels>) {
        init_gpui_component(cx);
        let raster = RasterImage {
            width: 200,
            height: 100,
            rgba: vec![255; 200 * 100 * 4],
            scale: 2.0,
        };
        let image = CachedImage::from_raster(&raster).expect("probe image converts");
        let window = cx.add_window(|window, cx| {
            let probe = cx.new(|_| CardProbe { image });
            gpui_component::Root::new(probe, window, cx)
        });
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let card = vcx.debug_bounds("mermaid-card").expect("card painted");
        (vcx, card)
    }

    /// [`mounted_card`] plus the hover that reveals the action row, and that
    /// row's painted bounds.
    fn hovered_action_row(
        cx: &mut TestAppContext,
    ) -> (VisualTestContext, gpui::Bounds<gpui::Pixels>) {
        let (mut vcx, card) = mounted_card(cx);
        vcx.simulate_mouse_move(card.center(), None, Default::default());
        vcx.run_until_parked();
        let row = vcx
            .debug_bounds(selector("mermaid-actions"))
            .expect("action row painted once the card is hovered");
        (vcx, row)
    }

    /// The action row floats over the diagram, whose own click opens the
    /// lightbox. Copying must not open it too — gpui delivers the click to
    /// every hitbox under the pointer unless a handler stops propagation.
    #[gpui::test]
    async fn copy_button_does_not_also_open_the_lightbox(cx: &mut TestAppContext) {
        let (mut cx, row) = hovered_action_row(cx);
        // Copy is the last of the two buttons.
        let copy = nth_quarter(row, 0.75);
        cx.simulate_mouse_move(copy, None, Default::default());
        cx.run_until_parked();
        cx.simulate_click(copy, Default::default());
        cx.run_until_parked();

        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(
            copied.as_deref(),
            Some(SOURCE),
            "the copy button must still copy the diagram source"
        );
        assert!(
            !cx.update(|window, cx| window.has_active_dialog(cx)),
            "copying must not also open the lightbox"
        );
    }

    /// The zoom button sits over the same diagram click target, so without
    /// stopped propagation it opens the lightbox twice — invisible on screen,
    /// but it takes two dismissals to close.
    #[gpui::test]
    async fn zoom_button_opens_exactly_one_lightbox(cx: &mut TestAppContext) {
        let (mut cx, row) = hovered_action_row(cx);
        // Zoom is the first of the two buttons.
        let zoom = nth_quarter(row, 0.25);
        cx.simulate_mouse_move(zoom, None, Default::default());
        cx.run_until_parked();
        cx.simulate_click(zoom, Default::default());
        cx.run_until_parked();

        assert!(
            cx.update(|window, cx| window.has_active_dialog(cx)),
            "the zoom button must open the lightbox"
        );
        cx.update(|window, cx| window.close_dialog(cx));
        cx.run_until_parked();
        assert!(
            !cx.update(|window, cx| window.has_active_dialog(cx)),
            "one dismissal must close it — a second stacked dialog was opened"
        );
    }

    /// The lightbox belongs to the diagram, not to the card: a diagram narrower
    /// than the card leaves a blank strip beside it, and clicking that strip
    /// must do nothing. The click target used to stretch the full card width.
    #[gpui::test]
    async fn only_the_diagram_itself_opens_the_lightbox(cx: &mut TestAppContext) {
        let (mut cx, card) = mounted_card(cx);
        let diagram = cx
            .debug_bounds(selector("mermaid-open"))
            .expect("diagram click target painted");
        assert!(
            diagram.right() < card.right(),
            "the probe must leave blank card space beside the diagram \
             (diagram={diagram:?}, card={card:?})"
        );

        let blank = point(
            diagram.right() + (card.right() - diagram.right()) / 2.0,
            diagram.center().y,
        );
        cx.simulate_click(blank, Default::default());
        cx.run_until_parked();
        assert!(
            !cx.update(|window, cx| window.has_active_dialog(cx)),
            "blank card space beside the diagram must not open the lightbox"
        );

        cx.simulate_click(diagram.center(), Default::default());
        cx.run_until_parked();
        assert!(
            cx.update(|window, cx| window.has_active_dialog(cx)),
            "the diagram itself must still open the lightbox"
        );
    }
}
