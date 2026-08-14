//! The conversation message blocks: user bubble, assistant prose (labeled
//! foldable block, header-less inline render, and the turn conclusion), agent
//! thinking, and surfaced errors. The mermaid `code_block_render` hook shared
//! by every markdown body lives in the sibling `mermaid` module.

use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px, relative};

use super::MermaidImages;
use super::fold_header::{FoldHeader, FoldRow, SummaryLine, rollup_glyph};
use super::links::AgentChatMarkdownLinks;
use super::mermaid::mermaid_code_block_render;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::Rollup;
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

#[derive(Clone, Copy)]
pub(super) struct MarkdownRender<'a> {
    pub(super) mermaid_images: &'a MermaidImages,
    pub(super) dim: f32,
    pub(super) links: AgentChatMarkdownLinks,
}

impl<'a> MarkdownRender<'a> {
    pub(super) fn new(
        mermaid_images: &'a MermaidImages,
        dim: f32,
        links: AgentChatMarkdownLinks,
    ) -> Self {
        Self {
            mermaid_images,
            dim,
            links,
        }
    }
}

/// User prompt — right-aligned accent-tinted bubble. The body renders as
/// selectable **plain text** via `crate::ui::selectable_text` (verbatim, no
/// markdown interpretation) inside the bubble chrome (bg / padding / rounded),
/// keyed by `ix` for stable selection identity. A user prompt is a
/// question/instruction, not authored markup, so `#`, `---` (setext heading),
/// lists, and code fences must display literally rather than being promoted to
/// headings / rules / blocks the way the assistant's markdown reply is.
pub(super) fn user_bubble(
    ix: usize,
    text: &str,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let body = crate::ui::selectable_text(("agent-chat-user", ix), text.to_string())
        .color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .full_width(false);
    let inner = div()
        .max_w(relative(0.85))
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        // Translucent accent tint (not the neutral code tint): sets the user's
        // turn off by hue while riding the pane background on any theme. Border
        // is the shared neutral hairline (same as code blocks / tool cards), so
        // accent stays on the fill alone — the card border system is uniform.
        .bg(theme::dim_toward_gray(theme::AGENT_CHAT_USER_TINT, dim))
        .border_1()
        .border_color(theme::dim_toward_gray(
            theme::agent_chat_border_tint(cx),
            dim,
        ))
        .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
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
    markdown: MarkdownRender<'_>,
    cx: &App,
) -> AnyElement {
    crate::ui::markdown(("agent-chat-md-assistant", ix), text.to_string())
        .color(theme::dim_toward_gray(
            theme::agent_chat_fg(cx),
            markdown.dim,
        ))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .code_block_render(mermaid_code_block_render(
            markdown.mermaid_images,
            markdown.dim,
        ))
        .link_click_handler(markdown.links.handler())
        .into_any_element()
}

/// Assistant response — left-aligned, foldable block (default expanded). The
/// body renders as rendered, drag-selectable markdown via `crate::ui::markdown`
/// (keyed by `ix` for stable selection identity); a still-streaming block shows
/// its partial markdown fine (no per-message caret — the streaming signal lives
/// on the input dock). Collapsed, the header shows the first non-empty line of
/// `text`, dimmed and single-line ellipsized.
///
/// `rollup` is `Some` only when this block *is* the whole response — a bar-less
/// anchored run of one block (`RowKind::SoloResponse`), where no response bar
/// exists to carry the verdict. `None` for a block among siblings, so a run never
/// shows a rollup computed over one of its items.
#[allow(clippy::too_many_arguments)]
pub(super) fn assistant_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    agent_label: &str,
    rollup: Option<Rollup>,
    t: &theme::DarudaTheme,
    markdown: MarkdownRender<'_>,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let mut header = FoldHeader::with_summary(|| SummaryLine::from_markdown(text)).leading(
        div()
            .flex_none()
            .text_color(theme::dim_toward_gray(
                theme::agent_chat_fg(cx),
                markdown.dim,
            ))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(agent_label.to_string()))
            .into_any_element(),
    );
    if let Some(rollup) = rollup {
        header = header.trailing(rollup_glyph(rollup, t, cx));
    }
    FoldRow::block(("agent-chat-assistant", ix), key, expanded, header, |cx| {
        assistant_markdown(ix, text, markdown, cx)
    })
    .render(markdown.dim, cx)
}

/// The turn's conclusion — the run's final assistant message rendered under a
/// response bar. Same drag-selectable markdown body as [`assistant_block`] but
/// with no speaker label and no rollup glyph (the response bar above already
/// names the speaker and carries the run's verdict): just the bare disclosure
/// chevron, so the conclusion folds to its first-line summary independently of
/// the response's process fold.
pub(super) fn conclusion_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    markdown: MarkdownRender<'_>,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    FoldRow::block(
        ("agent-chat-conclusion", ix),
        key,
        expanded,
        FoldHeader::with_summary(|| SummaryLine::from_markdown(text)),
        |cx| assistant_markdown(ix, text, markdown, cx),
    )
    .render(markdown.dim, cx)
}

/// Agent reasoning — dimmed, foldable block under a "Thinking" label (default
/// collapsed once settled, expanded while streaming, handled by the fold
/// derivation). The body renders as rendered, drag-selectable markdown via
/// `crate::ui::markdown` (keyed by `ix`), dimmed via `theme::agent_chat_fg_subtle(cx)`. Collapsed,
/// the header shows the first non-empty line of `text`, dimmed italic.
//
// `crate::ui::markdown` (TextView) owns its own typography, so the body is
// not italicized; the "Thinking" label plus the dimmer `text_subtle` colour
// distinguish reasoning from the assistant body.
pub(super) fn thinking_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    markdown: MarkdownRender<'_>,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let header =
        FoldHeader::with_summary(|| SummaryLine::from_markdown(text).map(SummaryLine::reasoning))
            .leading(
                div()
                    .flex_none()
                    .text_color(theme::dim_toward_gray(
                        theme::agent_chat_fg(cx),
                        markdown.dim,
                    ))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat_thinking_label()))
                    .into_any_element(),
            );
    FoldRow::block(("agent-chat-thinking", ix), key, expanded, header, |cx| {
        crate::ui::markdown(("agent-chat-md-thinking", ix), text.to_string())
            .color(theme::dim_toward_gray(
                theme::agent_chat_fg_subtle(cx),
                markdown.dim,
            ))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .code_block_render(mermaid_code_block_render(
                markdown.mermaid_images,
                markdown.dim,
            ))
            .link_click_handler(markdown.links.handler())
            .into_any_element()
    })
    .render(markdown.dim, cx)
}

/// Surfaced error item — error-tinted block.
/// A turn failure, with the one action its classification implies.
///
/// This is the *main* path an expired login surfaces on, not the connect
/// banner: the agent creates the session without checking credentials and only
/// refuses at the first real request, so the session stays alive and the
/// failure lands here. Without an affordance the conversation just stops with
/// a red line and no way forward inside the app.
///
/// Only `Reauthenticate` gets a button. `Retry` on a turn means re-sending the
/// prompt rather than reconnecting (the events differ, so the action does),
/// and the rest are either the user's to fix elsewhere or have no action at
/// all — for those the message alone is the honest answer.
pub(super) fn failure_block(
    failure: &daruda_acp::AcpFailure,
    pane_id: crate::workspace::main_area::pane_tree::PaneId,
    window_handle: gpui::AnyWindowHandle,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let sign_in = matches!(failure.remedy(), daruda_acp::Remedy::Reauthenticate).then(|| {
        crate::ui::button(
            ("agent-chat-failure-reauth", pane_id as usize),
            s::agent_chat_sign_in_again(),
        )
        .ghost()
        .xsmall()
        .on_click(cx.listener(move |_this, _ev, _window, cx| {
            // The login op reaches this same view through `Workspace`, which
            // would double-lease-panic inline (CLAUDE.md Pitfall #5).
            cx.defer(move |cx| {
                if let Some(workspace) =
                    crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
                {
                    // SILENT-OK: the workspace window may already be closed by the time this deferred callback runs — nothing left to sign in for
                    let _ = workspace.update(cx, |ws, cx| {
                        ws.reauthenticate_pane_account(pane_id, cx);
                    });
                }
            });
        }))
    });
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .child(error_block(failure.message(), t, cx))
        .when_some(sign_in, |el, btn| {
            el.child(div().flex().flex_row().child(btn))
        })
}

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
