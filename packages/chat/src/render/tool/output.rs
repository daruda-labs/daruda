//! Tool-output block bodies: verbatim text, markdown, embeds, images and
//! resource links, with their truncation notes.

use super::*;

#[derive(Clone)]
pub(in crate::render) struct OutputBlockContext<'a> {
    pub(in crate::render) assets: RenderAssets<'a>,
    pub(in crate::render) t: &'a theme::DarudaTheme,
    pub(in crate::render) dim: f32,
    pub(in crate::render) links: AgentChatMarkdownLinks,
}

/// Flush plain-monospace text: no markdown pass, no code-block chrome, just
/// selectable verbatim content. Shared by the two entry points that must not
/// let markdown reinterpret a tool's bytes — a bare (language-less) fence in a
/// markdown body, and a `RawText` block that never had a fence at all.
fn plain_monospace_text(
    id: SharedString,
    source: &str,
    color: Hsla,
    font_size: Pixels,
) -> AnyElement {
    div()
        .min_w_0()
        .font_family(theme::FONT_FAMILY_MONOSPACE)
        .text_color(color)
        .text_size(font_size)
        .child(
            daruda_ui::selectable_text(id, SharedString::from(source.to_string()))
                .color(color)
                .text_size(font_size),
        )
        .into_any_element()
}

/// Append the "output was capped at N bytes" marker under `body` when the block
/// carries one. Shared by every text-shaped output block so the marker reads and
/// sits the same regardless of which channel the text arrived through.
fn with_truncation_note(
    body: AnyElement,
    truncated_from: Option<usize>,
    dim: f32,
    cx: &App,
) -> AnyElement {
    let Some(original_len) = truncated_from else {
        return body;
    };
    div()
        .flex()
        .flex_col()
        .child(body)
        .child(
            div()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(s::agent_chat::tool_output_truncated(original_len)),
        )
        .into_any_element()
}

/// The byte-cap marker a text-shaped output block carries, if any. Shared by the
/// editor-embed path and the per-variant fallbacks so both reach
/// [`with_truncation_note`] through the same read.
fn output_truncated_from(block: &ToolOutputBlock) -> Option<usize> {
    match block {
        ToolOutputBlock::Text { truncated_from, .. }
        | ToolOutputBlock::RawText { truncated_from, .. }
        | ToolOutputBlock::SourceText { truncated_from, .. } => *truncated_from,
        _ => None,
    }
}

/// Render one tool-output block: a height-capped read-only editor when the
/// reconciler built one for this block (verbatim, non-markdown content), else
/// rendered markdown (drag-selectable, keyed per block for stable selection
/// state), verbatim monospace for raw shell output, or a resource link as an
/// optional local image preview plus open button. The ACP spec says clients
/// SHOULD render tool text as Markdown; code blocks keep their own monospace +
/// syntax highlight.
pub(in crate::render) fn output_block_view(
    tool_id: &str,
    ix: usize,
    block: &ToolOutputBlock,
    context: OutputBlockContext<'_>,
    cx: &App,
) -> AnyElement {
    let key = output_editor_key(tool_id, ix);
    if let Some(editor) = context.assets.output_editors.get(&key) {
        // The copy text comes from the same classifier the reconciler fed the
        // editor, so it is the fence body rather than the raw block text — what
        // the user actually sees in the embed.
        let copy = output_editor_source(block).map(|src| SharedString::from(src.text.to_string()));
        let body = bounded_editor_embed(
            &key,
            editor,
            copy,
            theme::AGENT_CHAT_EMBED_MAX_ROWS,
            context.t,
            context.dim,
            cx,
        );
        return with_truncation_note(body, output_truncated_from(block), context.dim, cx);
    }
    let dim = context.dim;
    let mermaid_images = context.assets.mermaid_images;
    let tool_images = context.assets.tool_images;
    let resource_images = context.assets.resource_images;
    match block {
        ToolOutputBlock::Text {
            text,
            truncated_from,
        } => {
            let plain_id_prefix = format!("agent-chat-tool-out-{tool_id}-{ix}");
            let plain_color = theme::dim_toward_gray(theme::agent_chat_fg(cx), dim);
            let mermaid_images = mermaid_images.clone();
            let markdown = super::super::blocks::pane_markdown(
                SharedString::from(plain_id_prefix.clone()),
                text.clone(),
                plain_color,
                cx,
            )
            .link_click_handler(context.links.handler())
            .code_block_render(move |lang, source, _window, cx| {
                // A ```mermaid fence in tool output renders as the same
                // diagram card as chat prose (shared builder); a cache
                // miss (still rasterizing) falls through to the plain
                // bare-fence branch below, same as the prose hook.
                if let Some(card) = mermaid_fence_element(&mermaid_images, lang, source, dim, cx) {
                    return Some(card);
                }
                // The Claude ACP adapter wraps every tool result in a bare,
                // language-less fence (`markdownEscape`) even when the
                // content isn't source code (command output, search hits,
                // …). A tagged fence keeps the default boxed/highlighted
                // rendering; a bare one renders as flush plain text
                // instead, so non-code output doesn't double the tool
                // card's own border/bg chrome for content that reads as
                // prose, not code.
                if !lang.is_empty() {
                    return None;
                }
                let mut hasher = DefaultHasher::new();
                source.hash(&mut hasher);
                let id = SharedString::from(format!("{plain_id_prefix}-plain-{}", hasher.finish()));
                Some(plain_monospace_text(
                    id,
                    source,
                    plain_color,
                    px(theme::agent_chat_font_size(cx)),
                ))
            })
            .into_any_element();
            with_truncation_note(markdown, *truncated_from, dim, cx)
        }
        ToolOutputBlock::RawText {
            text,
            truncated_from,
        }
        | ToolOutputBlock::SourceText {
            text,
            truncated_from,
            ..
        } => {
            // Verbatim bytes — shell output recovered from the terminal sideband,
            // or a file's own contents. Straight to monospace rather than through
            // markdown, which would eat a `#` or `*` that is literal here. Only
            // reached when the reconciler built no editor (window gone), so the
            // language a `SourceText` carries has nothing to colour.
            let body = plain_monospace_text(
                SharedString::from(format!("agent-chat-tool-rawout-{tool_id}-{ix}")),
                text,
                theme::dim_toward_gray(theme::agent_chat_fg(cx), dim),
                px(theme::agent_chat_font_size(cx)),
            );
            with_truncation_note(body, *truncated_from, dim, cx)
        }
        ToolOutputBlock::Image { data, mime } => {
            let key = tool_image_key(data);
            let cached = tool_images.lock().unwrap().get(&key).cloned();
            match cached {
                // Decoded and GPU-ready — render the real bitmap.
                Some(Some(image)) => zoomable_image(
                    SharedString::from(format!("agent-chat-tool-image-{tool_id}-{ix}")),
                    &image,
                ),
                // Decode failed (malformed base64 / unsupported format) —
                // fall back to the binary-descriptor label (mime + the
                // approximate decoded byte size, `base64_len / 4 * 3`) so the
                // card still shows something useful instead of a dead
                // placeholder that never resolves.
                Some(None) => div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat::tool_media_label(
                        mime,
                        data.len() / 4 * 3,
                    )))
                    .into_any_element(),
                // Not yet decoded / still in flight — a dimmed placeholder
                // stands in so the card shows *something* useful instead of a
                // (truncated) base64 blob. `reconcile_tool_images` fills the
                // cache and notifies once the decode lands.
                None => div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat::tool_image_placeholder()))
                    .into_any_element(),
            }
        }
        ToolOutputBlock::Media { mime, byte_len } => div()
            .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(s::agent_chat::tool_media_label(
                mime, *byte_len,
            )))
            .into_any_element(),
        ToolOutputBlock::ResourceLink { uri, name, mime } => {
            let cached = resource_images.lock().unwrap().get(&key).cloned();
            // Through the pane's link opener, not the platform's: Codex sends
            // a bare path with no scheme, which `open_url` refuses silently,
            // and only the workspace knows the pane's cwd and the file's kind.
            let links = context.links;
            let links_for_menu = links.clone();
            let uri_for_click = uri.clone();
            let mime_for_click = mime.clone();
            let link = daruda_ui::button(
                SharedString::from(format!("agent-chat-tool-link-{tool_id}-{ix}")),
                SharedString::from(name.clone()),
            )
            .on_click(move |_, _window, cx| {
                links.open_resource(&uri_for_click, mime_for_click.as_deref(), cx)
            });
            let uri_for_menu = uri.clone();
            let mime_for_menu = mime.clone();
            let card = div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_SM))
                // A right press records the resource ahead of the pane's menu,
                // so the menu resolves it as the left click does
                // (`take_pane_click_info`).
                .on_mouse_down(MouseButton::Right, move |event, _window, cx| {
                    links_for_menu.record_resource_right_click(
                        event.position,
                        uri_for_menu.clone(),
                        mime_for_menu.clone(),
                        cx,
                    )
                });
            match cached {
                Some(Some(image)) => card
                    .child(zoomable_image(
                        SharedString::from(format!("agent-chat-tool-link-image-{tool_id}-{ix}")),
                        &image,
                    ))
                    .child(link)
                    .into_any_element(),
                // Missing, still loading, or failed: the original resource
                // link remains usable and no permanent placeholder is shown.
                Some(None) | None => card.child(link).into_any_element(),
            }
        }
    }
}

/// A tool-output bitmap at block size, opening in the lightbox on click — the
/// same gesture the mermaid card has. Sized to the image, not the row, so the
/// blank strip beside a narrow image is not a target.
pub(super) fn zoomable_image(id: impl Into<ElementId>, image: &CachedImage) -> AnyElement {
    let image_for_click = image.clone();
    div()
        .id(id)
        .w(px(image.block_width()))
        .max_w_full()
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            super::super::image_lightbox::open(&image_for_click, window, cx);
        })
        .child(image.block())
        .into_any_element()
}
