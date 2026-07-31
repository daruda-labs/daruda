//! Mermaid diagram embeds for the chat transcript: the shared card chrome
//! (translucent tint + hairline, matching tool cards / code blocks), the
//! markdown `code_block_render` hook, and the hover zoom / copy affordances.
//! Shared by the assistant prose body and tool-output markdown.

use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px};

use super::MermaidImages;
use crate::ui::theme;
use crate::ui::{IconName, button_bare};
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
    let src_for_copy = source.to_string();
    let image_for_zoom = image.clone();
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
        .child(image.block_diagram())
        .child(
            div()
                .absolute()
                .top_1()
                .right_1()
                .flex()
                .flex_row()
                .gap_1()
                .invisible()
                .group_hover(group, |s| s.visible())
                .child(
                    button_bare(SharedString::from(format!("mermaid-zoom-{key}")))
                        .icon(IconName::Maximize)
                        .on_click(move |_, window, cx| {
                            super::mermaid_lightbox::open(&image_for_zoom, window, cx);
                        }),
                )
                .child(
                    button_bare(SharedString::from(format!("mermaid-copy-{key}")))
                        .icon(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                src_for_copy.clone(),
                            ));
                        }),
                ),
        )
        .into_any_element()
}
