//! The conversation message blocks: user bubble, assistant prose (labeled
//! foldable block, header-less inline render, and the turn conclusion), agent
//! thinking, and surfaced errors. The mermaid `code_block_render` hook shared
//! by every markdown body lives in the sibling `mermaid` module.

use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px, relative};

use super::MermaidImages;
use super::fold_header::{FoldHeader, FoldRow, SummaryLine};
use super::links::AgentChatMarkdownLinks;
use super::mermaid::mermaid_code_block_render;
use crate::surface::strings as s;
use crate::ui::ButtonVariants as _;
use crate::ui::theme;
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

/// Markdown painted on the chat pane. The body colour, the surface it sits on
/// and the configured text size always travel together: a caller that sets only
/// the colour leaves the view deriving its fills and structural lines from the
/// UI canvas, which this pane does not paint on (DESIGN.md §AgentChatPane) —
/// under a light `ui_preset` over a dark terminal that erased every table line,
/// rule, code-block border and inline-code chip at once.
pub(super) fn pane_markdown(
    id: impl Into<gpui::ElementId>,
    text: impl Into<gpui::SharedString>,
    color: gpui::Hsla,
    cx: &App,
) -> crate::ui::Markdown {
    crate::ui::markdown(id, text)
        .color(color)
        .surface(theme::agent_chat_bg(cx))
        .text_size(px(theme::agent_chat_font_size(cx)))
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
    pane_markdown(
        ("agent-chat-md-assistant", ix),
        text.to_string(),
        theme::dim_toward_gray(theme::agent_chat_fg(cx), markdown.dim),
        cx,
    )
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
#[allow(clippy::too_many_arguments)]
pub(super) fn assistant_block(
    ix: usize,
    key: FoldKey,
    expanded: bool,
    text: &str,
    agent_label: &str,
    markdown: MarkdownRender<'_>,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let header = FoldHeader::with_summary(|| SummaryLine::from_markdown(text)).leading(
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
        pane_markdown(
            ("agent-chat-md-thinking", ix),
            text.to_string(),
            theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), markdown.dim),
            cx,
        )
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
/// A button inside the conversation — on the pane body, whose background
/// mirrors the terminal's.
///
/// Its colours come from that background, which is what a `ghost` button gets
/// wrong here twice over: ghost paints no fill at all, so the affordance read
/// as a run of plain text, and it takes its label from the *window* theme,
/// which knows nothing about the terminal one. A light window theme over a dark
/// terminal put dark text on a dark surface and the button vanished.
///
/// The fill and hairline are the same background-derived overlay every card in
/// this pane already uses (white over a dark pane, black over a light one), and
/// the label is the pane's own body colour. Not the error colour it sits under
/// — an action is not part of the message, and painting it like one reads as
/// more error text rather than as something to press.
pub(super) fn pane_action_button(
    id: impl Into<gpui::ElementId>,
    label: String,
    cx: &mut Context<AgentChatView>,
) -> crate::ui::Button {
    crate::ui::button(id, label).custom(
        crate::ui::ButtonCustomVariant::new(cx)
            .color(theme::agent_chat_tint(cx))
            .foreground(theme::agent_chat_fg(cx))
            .border(theme::agent_chat_border_tint(cx))
            // One step up the same overlay, so hover is felt on any pane
            // background rather than borrowed from the window theme.
            .hover(theme::agent_chat_border_tint(cx))
            .active(theme::agent_chat_border_tint(cx)),
    )
}

/// A button on the status banner — which is *not* the pane body.
///
/// The banner composites its tint over the window surface, so it reads light
/// under a light window theme even when the conversation below is dark. Reusing
/// the pane palette here painted the pane's near-white label onto that light
/// banner and the button all but disappeared: the same class of mistake as
/// ghost's, in the opposite direction.
///
/// So the chrome comes from the window theme, which is the surface it actually
/// sits on — and which flips with that theme, so the pairing holds in both.
pub(super) fn banner_action_button(
    id: impl Into<gpui::ElementId>,
    label: String,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> crate::ui::Button {
    crate::ui::button(id, label).custom(
        crate::ui::ButtonCustomVariant::new(cx)
            .color(t.button_widget_bg)
            .foreground(t.text_primary)
            .border(t.border)
            .hover(t.overlay_hover)
            .active(t.overlay_active),
    )
}

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
    ix: usize,
    failure: &daruda_acp::AcpFailure,
    pane_id: crate::workspace::main_area::pane_tree::PaneId,
    window_handle: gpui::AnyWindowHandle,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let sign_in = matches!(failure.remedy(), daruda_acp::Remedy::Reauthenticate).then(|| {
        pane_action_button(
            // Keyed by the item, like every sibling block. Keyed by the pane,
            // two failures in one conversation shared an element id — gpui
            // then treats them as one element, so a click on either ran both
            // of their handlers.
            ("agent-chat-failure-reauth", ix),
            s::agent_chat_sign_in_again(),
            cx,
        )
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
        // Wider than the gap *within* a message (`AGENT_CHAT_MSG_GAP`), so the
        // button reads as a separate thing to press rather than as the last
        // line of the text above it.
        .gap(px(theme::GAP_LG))
        .child(error_block(failure.message(), t, cx))
        .when_some(sign_in, |el, btn| {
            // Centred under the message. The row stretches to the column's
            // width (flex-column default), so `justify_center` has something to
            // centre within; this container is `failure_block`'s alone, so
            // neither the gap nor the alignment reaches any other block.
            el.child(div().flex().flex_row().justify_center().child(btn))
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
