//! The conversation message blocks: user bubble, assistant prose (labeled
//! foldable block, header-less inline render, and the turn conclusion), agent
//! thinking, surfaced errors, plus the mermaid `code_block_render` hook shared
//! by every markdown body.

use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px, relative};

use super::{MermaidImages, collapsed_text_summary, foldable_block};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{IconName, button_bare};
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::mermaid_key;
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

/// The `code_block_render` hook for a chat markdown body: replace a
/// ` ```mermaid ` fence with its cached diagram bitmap, leaving every other code
/// block (and a not-yet-rasterized mermaid fence) to the default code rendering
/// by returning `None`. Captures a cheap clone of the images map (`Arc` values)
/// so the closure stays `Send + Sync + 'static` (the `TextView` requirement)
/// without borrowing or cloning the image bytes.
fn mermaid_code_block_render(
    mermaid_images: &MermaidImages,
) -> impl Fn(&str, &str, &mut gpui::Window, &mut gpui::App) -> Option<AnyElement> + Send + Sync + 'static
{
    let images = mermaid_images.clone();
    move |lang, source, _window, cx| {
        if lang != "mermaid" {
            return None;
        }
        // Key by the current appearance so a light/dark toggle looks up the
        // matching raster (the ops layer re-rasterizes on theme change); a miss
        // during the brief re-raster falls back to the default code block.
        let dark = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .unwrap_or(true);
        let key = mermaid_key(source, dark);
        // Read the live shared cache (not a snapshot) — see `MermaidImages`.
        // Cloning the cached `CachedImage` is an `Arc` bump, so gpui reuses the
        // already-uploaded texture instead of re-uploading the bitmap.
        let image = images.lock().ok()?.get(&key).cloned()?;
        let diagram = image.block();
        // The diagram is a bitmap (not selectable), so overlay a hover-revealed
        // button that copies the mermaid source to the clipboard.
        let group = SharedString::from(format!("mermaid-{key}"));
        let src = source.to_string();
        Some(
            div()
                .relative()
                .group(group.clone())
                .child(diagram)
                .child(
                    div()
                        .absolute()
                        .top_1()
                        .right_1()
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .child(
                            button_bare(SharedString::from(format!("mermaid-copy-{key}")))
                                .icon(IconName::Copy)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        src.clone(),
                                    ));
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// User prompt — right-aligned accent-tinted bubble. The body renders as
/// selectable markdown via `crate::ui::markdown` inside the bubble chrome
/// (bg / padding / rounded), keyed by `ix` for stable selection identity.
/// Mermaid fences render as diagrams via the `code_block_render` hook.
pub(super) fn user_bubble(
    ix: usize,
    text: &str,
    mermaid_images: &MermaidImages,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body = crate::ui::markdown(("agent-chat-md-user", ix), text.to_string())
        .color(theme::agent_chat_fg(cx))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .full_width(false)
        .code_block_render(mermaid_code_block_render(mermaid_images));
    let inner = div()
        .max_w(relative(0.85))
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        // Translucent accent tint (not the neutral code tint): sets the user's
        // turn off by hue while riding the pane background on any theme. Border
        // is the shared neutral hairline (same as code blocks / tool cards), so
        // accent stays on the fill alone — the card border system is uniform.
        .bg(theme::AGENT_CHAT_USER_TINT)
        .border_1()
        .border_color(theme::agent_chat_border_tint(cx))
        .text_color(theme::agent_chat_fg(cx))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(body);
    div().flex().flex_row().justify_end().child(inner)
}

/// The assistant prose body — drag-selectable rendered markdown with mermaid
/// fences rasterized. Shared by the labeled [`assistant_block`] (trivial /
/// top-level reply) and the header-less inline render used under a response bar.
pub(super) fn assistant_markdown(
    ix: usize,
    text: &str,
    mermaid_images: &MermaidImages,
    cx: &App,
) -> AnyElement {
    crate::ui::markdown(("agent-chat-md-assistant", ix), text.to_string())
        .color(theme::agent_chat_fg(cx))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .code_block_render(mermaid_code_block_render(mermaid_images))
        .into_any_element()
}

/// Assistant response — left-aligned, foldable block (default expanded). The
/// body renders as rendered, drag-selectable markdown via `crate::ui::markdown`
/// (keyed by `ix` for stable selection identity); a still-streaming block shows
/// its partial markdown fine (no per-message caret — the streaming signal lives
/// on the input dock). Collapsed, the header shows the first non-empty line of
/// `text`, dimmed and single-line ellipsized.
pub(super) fn assistant_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body_el = assistant_markdown(ix, text, mermaid_images, cx);
    let header = div()
        .flex_none()
        .text_color(theme::agent_chat_fg(cx))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(s::agent_chat_label_agent()))
        .into_any_element();
    let summary = collapsed_text_summary(text, false, cx);
    foldable_block(
        ("agent-chat-assistant", ix),
        key,
        expanded,
        header,
        summary,
        body_el,
        |row| row,
        cx,
    )
}

/// The turn's conclusion — the run's final assistant message rendered under a
/// response bar. Same drag-selectable markdown body as [`assistant_block`] but
/// with no "Agent" label (the response bar above already names the speaker):
/// just the bare disclosure chevron, so the conclusion folds to its first-line
/// summary independently of the response's process fold.
pub(super) fn conclusion_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body_el = assistant_markdown(ix, text, mermaid_images, cx);
    let summary = collapsed_text_summary(text, false, cx);
    foldable_block(
        ("agent-chat-conclusion", ix),
        key,
        expanded,
        gpui::Empty.into_any_element(),
        summary,
        body_el,
        |row| row,
        cx,
    )
}

/// Agent reasoning — dimmed, foldable block under a "Thinking" label (default
/// collapsed once settled, expanded while streaming, handled by the fold
/// derivation). The body renders as rendered, drag-selectable markdown via
/// `crate::ui::markdown` (keyed by `ix`), dimmed via `theme::agent_chat_fg_subtle(cx)`. Collapsed,
/// the header shows the first non-empty line of `text`, dimmed italic.
//
// NOTE: the previous italic treatment of the body is not preserved —
// `crate::ui::markdown` (TextView) owns its own typography. The "Thinking"
// label plus the dimmer `text_subtle` colour still distinguish reasoning from
// the assistant body.
pub(super) fn thinking_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    mermaid_images: &MermaidImages,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body_el = crate::ui::markdown(("agent-chat-md-thinking", ix), text.to_string())
        .color(theme::agent_chat_fg_subtle(cx))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .code_block_render(mermaid_code_block_render(mermaid_images))
        .into_any_element();
    let header = div()
        .flex_none()
        .text_color(theme::agent_chat_fg(cx))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(s::agent_chat_thinking_label()))
        .into_any_element();
    let summary = collapsed_text_summary(text, true, cx);
    foldable_block(
        ("agent-chat-thinking", ix),
        key,
        expanded,
        header,
        summary,
        body_el,
        |row| row,
        cx,
    )
}

/// Surfaced error item — error-tinted block.
pub(super) fn error_block(
    message: &str,
    t: &theme::DarudaTheme,
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.banner_error_bg)
        .text_color(t.banner_error_text)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(message.to_string()))
}
