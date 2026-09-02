//! Tool-call cards (title + status badge + foldable body of diffs and output)
//! and the inline permission cards with their per-choice buttons.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use daruda_acp::CommandExit;
use daruda_acp::{
    ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
};
use gpui::{
    AnyElement, AnyWindowHandle, App, Hsla, IntoElement, Pixels, SharedString, Window, div,
    prelude::*, px,
};

use super::RenderAssets;
use super::chrome::pulse_dots;
use super::diff::diff_block;
use super::embed::bounded_editor_embed;
use super::fold_header::{
    FoldHeader, FoldRow, SummaryLine, outside_window_rail, window_boundary_row,
};
use super::links::AgentChatMarkdownLinks;
use super::mermaid::{mermaid_code_block_render, mermaid_fence_element};
use super::tail_row::call_boundary_label;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{Icon, IconName, Sizable as _};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    TurnBoundary, diff_editor_key, fold_context_at, renders_raw_input,
    renders_subagent_instructions, suppresses_live_subagent_output, tool_fold_key, tool_image_key,
};
use crate::workspace::main_area::agent_chat_pane::fold::{FoldContext, FoldKey, FoldState};
use crate::workspace::main_area::agent_chat_pane::output_editor::{
    output_editor_key, output_editor_source,
};
use crate::workspace::main_area::agent_chat_pane::rows::subagent::{
    SubagentChildren, SubagentLens,
};
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::rows::{
    FilterMatchIndex, LiveSubagentUnits, effective_tool_status,
};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

#[derive(Clone, Copy)]
struct OutputBlockContext<'a> {
    assets: RenderAssets<'a>,
    t: &'a theme::DarudaTheme,
    dim: f32,
    links: AgentChatMarkdownLinks,
}

/// Tool invocation card — foldable (default collapsed once done, expanded while
/// in progress). The header leads with a fixed-width label (the agent's own tool
/// name, else the kind) so it never grows with a long or multiline title, keeps
/// the status badge right-anchored in the trailing slot, and — collapsed — fills
/// the stretch slot between them with a dimmed one-line summary of the title. The
/// body — full untruncated title, then diffs + plain-text output — shows only
/// when expanded; the card's border / bg chrome wraps the fold assembly either
/// way. The nested diffs are independently foldable.
#[allow(clippy::too_many_arguments)]
pub(super) fn tool_card(
    key: FoldKey,
    expanded: bool,
    ix: usize,
    tc: &ToolCallItem,
    items: &[ChatItem],
    live_units: &LiveSubagentUnits,
    filter_matches: &FilterMatchIndex,
    filter_revealed: bool,
    boundary: TurnBoundary,
    assets: RenderAssets<'_>,
    fold: &FoldState,
    tail: TailWindow,
    t: &theme::DarudaTheme,
    dim: f32,
    depth: usize,
    pane_id: PaneId,
    window_handle: AnyWindowHandle,
    window: &mut Window,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let markdown_links = AgentChatMarkdownLinks::new(pane_id, window_handle);
    let turn = boundary.at(ix);
    // A subagent parent (Task/Agent) whose flattened children keep running past
    // its own completion must not read "done": the adapter marks the parent
    // `Completed` when its SDK call returns, but the child tool calls stream in
    // and run afterward (see `LiveSubagentUnits`). While any nested
    // descendant is live the unit is still working, so the badge reads
    // in-progress until the whole subtree settles.
    let effective_status = effective_tool_status(tc, live_units);
    let (badge_text, badge_fg) = tool_status_badge(effective_status, t, dim, cx);
    // A live tool gets animated trailing dots (Running. / .. / ...) so the
    // in-progress state reads as live, not just a static amber label. `Pending`
    // counts as live too (see `ToolStatusView::is_live`).
    let badge_text = if effective_status.is_live() {
        SharedString::from(format!("{badge_text}{}", pulse_dots(cx)))
    } else {
        badge_text
    };

    // Header: a tool-kind icon + a short label — the agent's own tool name
    // (Bash/Grep/…) when it surfaced one, else the fixed-vocabulary kind label
    // (Read/Edit/Search/…). Either way it's a short identifier, never the long
    // or multiline title, so the header line never grows — the full title is not
    // a summary here, it moves to the expanded body (below) instead. Collapsed,
    // a dimmed one-line summary of the title fills the stretch slot before the
    // badge so the card still reads at a glance without expanding.
    let fg = theme::dim_toward_gray(theme::agent_chat_fg(cx), dim);
    let failed = matches!(tc.status, ToolStatusView::Failed);
    let font_size = px(theme::agent_chat_font_size(cx));
    let mut header =
        FoldHeader::with_summary(|| Some(SummaryLine::plain(tool_title_summary(&tc.title))))
            .leading(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_SM))
                    .child(Icon::new(tool_kind_icon(tc.kind)).xsmall().text_color(fg))
                    .child(div().flex_none().text_color(fg).text_size(font_size).child(
                        SharedString::from(tool_header_label(tc.tool_name.as_deref(), tc.kind)),
                    ))
                    .into_any_element(),
            );
    // Detached shell command (`run_in_background: true`): the tool completes
    // immediately with an ack while the real process keeps running, so a
    // chip marks it as launched-in-background — otherwise the card is
    // indistinguishable from a normal one-shot command.
    if tc.is_background() {
        header = header.trailing(
            crate::ui::Badge::new(SharedString::from(s::agent_chat_tool_background()))
                .bg_color(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
                .border_color(theme::dim_toward_gray(
                    theme::agent_chat_border_tint(cx),
                    dim,
                ))
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                .into_any_element(),
        );
    }
    // A shell command that exited abnormally (nonzero code or killed by a
    // signal) gets its own badge ahead of the status one, so a collapsed
    // "Failed" card still says *what kind* of failure it was. Pushed before
    // the status badge so the status badge — always present — stays the
    // rightmost element and card-to-card right alignment doesn't shift.
    if let Some(exit_label) = exit_badge_label(&tc.exit) {
        header = header.trailing(
            crate::ui::Badge::new(SharedString::from(exit_label))
                .text_color(t.banner_error_text)
                .into_any_element(),
        );
    }
    let header = header.trailing(
        div()
            .flex_none()
            .text_color(badge_fg)
            .text_size(font_size)
            .child(badge_text)
            .into_any_element(),
    );

    // Body: the full, untruncated title first (dropped from the header above,
    // so it needs a home — un-muted `fg` so it reads as the primary statement
    // ahead of the muted raw-input/output labels and the diff blocks' own
    // hunk-bg chrome below it), then an optional raw-input disclosure (generic
    // tools), then nested diffs (each independently foldable), then plain-text
    // output. Built inside `FoldRow::block`'s closure, so a collapsed card never
    // pays for it.
    let body = move |cx: &mut Context<AgentChatView>| {
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .child(
                div().text_color(fg).text_size(font_size).child(
                    crate::ui::selectable_text(
                        SharedString::from(format!("agent-chat-tool-title-{}", tc.id)),
                        SharedString::from(tc.title.clone()),
                    )
                    .color(fg)
                    .text_size(font_size),
                ),
            );
        if renders_raw_input(tc)
            && let Some(raw) = &tc.raw_input
        {
            // Collapsed-by-default disclosure so the detail is on tap without
            // cluttering the card. `FoldRow::block` builds its body only when
            // expanded, so the pretty-print of a large `raw_input` blob stays off the
            // render hot path (GPUI has no partial redraw) with no manual gate.
            let raw_key = FoldKey::ToolRawInput(tc.id.clone());
            let raw_expanded = fold.is_expanded(&raw_key, FoldContext::new(turn, false));
            let raw_header = FoldHeader::bare().leading(
                div()
                    .flex_none()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(font_size)
                    .child(SharedString::from(s::agent_chat_raw_input_label()))
                    .into_any_element(),
            );
            body = body.child(
                FoldRow::block(
                    SharedString::from(format!("agent-chat-rawin-{}", tc.id)),
                    raw_key,
                    raw_expanded,
                    raw_header,
                    |cx| {
                        let pretty =
                            serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string());
                        div()
                            .min_w_0()
                            .font_family(theme::FONT_FAMILY_MONOSPACE)
                            .text_color(theme::dim_toward_gray(
                                theme::agent_chat_fg_subtle(cx),
                                dim,
                            ))
                            .text_size(font_size)
                            .child(
                                crate::ui::selectable_text(
                                    SharedString::from(format!("agent-chat-tool-rawin-{}", tc.id)),
                                    SharedString::from(pretty),
                                )
                                .text_size(font_size),
                            )
                            .into_any_element()
                    },
                )
                .toggle_on_chevron()
                .render(dim, cx),
            );
        }
        if renders_subagent_instructions(tc)
            && let Some(prompt) = tc.subagent_prompt()
        {
            // Always on, not tucked behind a fold: the prompt is the spec the
            // subagent's work is judged against, so it reads better as plain
            // markdown prose sitting in the card the same way `Output` does
            // below, rather than as one more disclosure to click open. Unlike
            // `output`, which a completion event overwrites with the
            // subagent's own result summary, `raw_input` (and so `prompt`)
            // survives for the card's whole lifetime. Kept out of the generic
            // raw-input JSON dump (`renders_raw_input` excludes this case) so
            // it reads as prose, not technical args sharing a section with
            // `subagent_type`/`run_in_background`. No section label: the
            // `Type: <kind>` chip already marks this as a subagent, and the
            // prompt text itself reads as instructions without a header
            // announcing it.
            let chip_bg = theme::dim_toward_gray(theme::agent_chat_tint(cx), dim);
            let chip_border = theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim);
            let chip_fg = theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim);
            let mut meta = div().flex().flex_row().flex_wrap().gap(px(theme::GAP_SM));
            let mut has_meta = false;
            if let Some(kind) = tc.subagent_type() {
                has_meta = true;
                meta = meta.child(
                    crate::ui::Badge::new(SharedString::from(s::agent_chat_subagent_type_chip(
                        kind,
                    )))
                    .bg_color(chip_bg)
                    .border_color(chip_border)
                    .text_color(chip_fg)
                    .into_any_element(),
                );
            }
            if tc.subagent_run_in_background() == Some(true) {
                has_meta = true;
                meta = meta.child(
                    crate::ui::Badge::new(SharedString::from(s::agent_chat_tool_background()))
                        .bg_color(chip_bg)
                        .border_color(chip_border)
                        .text_color(chip_fg)
                        .into_any_element(),
                );
            }
            if has_meta {
                body = body.child(meta);
            }
            // Same mermaid hook the assistant-prose body uses (`blocks.rs`), no
            // bare-fence unwrap layered on: that unwrap exists only to undo
            // Claude's `content`-channel markdownEscape wrapping (see
            // `output_block_view`'s `Text` branch), and `prompt` — sourced from
            // `raw_input`, never that channel — was never wrapped that way. A
            // fence the prompt genuinely contains (a code example in the
            // instructions) is real code, so it keeps the default
            // boxed/highlighted rendering.
            body = body.child(
                super::blocks::pane_markdown(
                    SharedString::from(format!("agent-chat-subagent-instructions-{}", tc.id)),
                    SharedString::from(prompt.to_string()),
                    theme::dim_toward_gray(theme::agent_chat_fg(cx), dim),
                    cx,
                )
                .code_block_render(mermaid_code_block_render(assets.mermaid_images, dim))
                .link_click_handler(markdown_links.handler()),
            );
        }
        for (di, diff) in tc.diffs.iter().enumerate() {
            let editor = assets.diff_editors.get(&diff_editor_key(&tc.id, di));
            body = body.child(diff_block(
                &tc.id,
                di,
                diff,
                editor,
                assets.diff_stats,
                fold,
                turn,
                t,
                dim,
                pane_id,
                window_handle,
                window,
                cx,
            ));
        }
        if !tc.output.is_empty() && !suppresses_live_subagent_output(tc) {
            body = body.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat_tool_output_label())),
            );
            let output_context = OutputBlockContext {
                assets,
                t,
                dim,
                links: markdown_links,
            };
            for (ix, block) in tc.output.iter().enumerate() {
                body = body.child(output_block_view(&tc.id, ix, block, output_context, cx));
            }
        }

        // Subagent activity: the Claude adapter flattens a spawned subagent's inner
        // tool calls into this session, linking each to this Task/Agent call only
        // via `parent_tool_id`. `rows::project` skips those children in the main
        // flow; render them nested here (recursively — a child may itself spawn one)
        // so the subagent reads as one unit that folds with its parent card.
        //
        // Which of them the card shows — the filter's admission, the step
        // window, the live escape and the nesting cap together — is decided by
        // `SubagentChildren`, so this loop only iterates the answer.
        let tail_key = FoldKey::SubagentTail(tc.id.clone());
        let tail_revealed =
            fold.is_expanded(&tail_key, fold_context_at(&tail_key, ix, items, boundary));
        let children = SubagentChildren::of(
            items,
            tc.id.as_str(),
            depth,
            SubagentLens {
                filter: filter_matches,
                filter_revealed,
                live_units,
                tail,
                revealed: tail_revealed,
            },
        );
        if !children.shown.is_empty() {
            // Name the spawned subagent when the Task input carries its type
            // (`subagent_type`); fall back to the generic label otherwise.
            let subagent_label = match tc.subagent_type() {
                Some(kind) => s::agent_chat_subagent_label_typed(kind),
                None => s::agent_chat_subagent_label(),
            };
            body = body.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(subagent_label)),
            );
            // The card's own step-axis boundary. These children own no row, so
            // it renders here rather than in the list — the same place the
            // raw-input disclosure lives.
            if children.offers_reveal() {
                body = body.child(window_boundary_row(
                    SharedString::from(format!("agent-chat-subagent-tail-{}", tc.id)),
                    tail_key,
                    tail_revealed,
                    SharedString::from(call_boundary_label(
                        children.hidden,
                        children.kept,
                        !tail_revealed,
                    )),
                    dim,
                    cx,
                ));
            }
            for child in children.shown {
                // A nested child may itself be a subagent launch (a subagent that
                // spawns its own subagent), so key it the same way as a top-level
                // card — collapsed by default when it is one.
                let child_key = tool_fold_key(child.call);
                let child_expanded = fold.is_expanded(
                    &child_key,
                    fold_context_at(&child_key, child.ix, items, boundary),
                );
                let card = tool_card(
                    child_key,
                    child_expanded,
                    child.ix,
                    child.call,
                    items,
                    live_units,
                    filter_matches,
                    filter_revealed,
                    boundary,
                    assets,
                    fold,
                    tail,
                    t,
                    dim,
                    depth + 1,
                    pane_id,
                    window_handle,
                    window,
                    cx,
                )
                .into_any_element();
                // A covered child is on screen only because the boundary above
                // it released it — or because it is still running. The rail says
                // so; without it the reveal just appends cards that read as part
                // of the window.
                body = body.child(if child.covered {
                    outside_window_rail(card, dim, cx)
                } else {
                    card
                });
            }
        }
        body.into_any_element()
    };

    let block = FoldRow::block(
        SharedString::from(format!("agent-chat-tool-{}", tc.id)),
        key,
        expanded,
        header,
        body,
    )
    .toggle_on_chevron()
    .render(dim, cx);

    // Card chrome (border + bg) wraps the fold assembly.
    div()
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        // Background-derived tint (not a fixed surface): a translucent lift
        // over the pane background so the card sits one step above it on any
        // background color / theme / opacity. Border is the same overlay one
        // step stronger, so the edge tracks the background too.
        .bg(theme::dim_toward_gray(theme::agent_chat_tint(cx), dim))
        .border_1()
        // A failed tool call gets an error-tinted dashed border so it reads as
        // failed at a glance, not just via the badge (mirrors zed's dashed
        // failure card). `t.banner_error_text` is already dimmed (t is the
        // dimmed theme); the normal border tint is a global-read, so it is
        // dim-wrapped here.
        .border_color(if failed {
            t.banner_error_text
        } else {
            theme::dim_toward_gray(theme::agent_chat_border_tint(cx), dim)
        })
        .when(failed, |d| d.border_dashed())
        .child(block)
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
            crate::ui::selectable_text(id, SharedString::from(source.to_string()))
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
                .child(s::agent_chat_tool_output_truncated(original_len)),
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
fn output_block_view(
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
            theme::AGENT_CHAT_EMBED_MAX_H,
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
            let markdown = super::blocks::pane_markdown(
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
                Some(Some(image)) => image.block(),
                // Decode failed (malformed base64 / unsupported format) —
                // fall back to the binary-descriptor label (mime + the
                // approximate decoded byte size, `base64_len / 4 * 3`) so the
                // card still shows something useful instead of a dead
                // placeholder that never resolves.
                Some(None) => div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat_tool_media_label(
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
                    .child(SharedString::from(s::agent_chat_tool_image_placeholder()))
                    .into_any_element(),
            }
        }
        ToolOutputBlock::Media { mime, byte_len } => div()
            .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(SharedString::from(s::agent_chat_tool_media_label(
                mime, *byte_len,
            )))
            .into_any_element(),
        ToolOutputBlock::ResourceLink { uri, name, .. } => {
            let cached = resource_images.lock().unwrap().get(&key).cloned();
            let uri = uri.clone();
            let link = crate::ui::button(
                SharedString::from(format!("agent-chat-tool-link-{tool_id}-{ix}")),
                SharedString::from(name.clone()),
            )
            .on_click(move |_, _, cx| cx.open_url(&uri));
            match cached {
                Some(Some(image)) => div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::GAP_SM))
                    .child(image.block())
                    .child(link)
                    .into_any_element(),
                // Missing, still loading, or failed: the original resource
                // link remains usable and no permanent placeholder is shown.
                Some(None) | None => link.into_any_element(),
            }
        }
    }
}

/// Collapse a multiline tool title to its first line + "…" so the collapsed
/// header's inline summary always stays a single line. Safe to truncate for
/// every kind (including `Execute`) now: the full, untruncated title has its
/// own line in the expanded body (see `tool_card`), so nothing is lost by
/// truncating here — the card is always expandable. A single-line title — or
/// one whose only newline is trailing (no real second line) — is returned
/// unchanged, so the "…" only appears when content is actually hidden.
fn tool_title_summary(title: &str) -> String {
    match title.split_once('\n') {
        Some((first, rest)) if !rest.trim().is_empty() => format!("{first}…"),
        _ => title.to_string(),
    }
}

/// The header's primary label. Prefers the agent's own tool name (`Bash`,
/// `Grep`, …) — the vocabulary the user knows from the agent's CLI and more
/// specific than the normalized kind — falling back to the fixed-vocabulary
/// kind label when the agent surfaced no tool name. The leading icon already
/// conveys the kind, so the specific name here adds information rather than
/// duplicating the icon.
fn tool_header_label(tool_name: Option<&str>, kind: ToolKindView) -> String {
    tool_name
        .map(str::to_owned)
        .unwrap_or_else(|| tool_kind_label(kind))
}

/// Map a tool kind to its short, fixed-vocabulary header label (Read/Edit/
/// Search/…), independent of the (possibly long) per-call title.
fn tool_kind_label(kind: ToolKindView) -> String {
    match kind {
        ToolKindView::Read => s::agent_chat_tool_kind_read(),
        ToolKindView::Edit => s::agent_chat_tool_kind_edit(),
        ToolKindView::Delete => s::agent_chat_tool_kind_delete(),
        ToolKindView::Move => s::agent_chat_tool_kind_move(),
        ToolKindView::Search => s::agent_chat_tool_kind_search(),
        ToolKindView::Execute => s::agent_chat_tool_kind_execute(),
        ToolKindView::Think => s::agent_chat_tool_kind_think(),
        ToolKindView::Fetch => s::agent_chat_tool_kind_fetch(),
        ToolKindView::SwitchMode => s::agent_chat_tool_kind_switch_mode(),
        ToolKindView::Other => s::agent_chat_tool_kind_other(),
    }
}

/// Map a tool kind to a leading header icon, so a tool call's type reads at a
/// glance (terminal vs read vs edit …), mirroring zed's kind-based icon.
pub(super) fn tool_kind_icon(kind: ToolKindView) -> IconName {
    // The vendored `IconName` set has no pencil/edit glyph, so Edit falls back
    // to `File` (Read already uses `Eye`, so no visual collision).
    match kind {
        ToolKindView::Read => IconName::Eye,
        ToolKindView::Edit => IconName::File,
        ToolKindView::Delete => IconName::Delete,
        ToolKindView::Move => IconName::ArrowRight,
        ToolKindView::Search => IconName::Search,
        ToolKindView::Execute => IconName::SquareTerminal,
        ToolKindView::Think => IconName::Bot,
        ToolKindView::Fetch => IconName::Globe,
        ToolKindView::SwitchMode => IconName::Refresh,
        ToolKindView::Other => IconName::Settings2,
    }
}

/// The exit-status badge text for a shell tool call, if abnormal. `None` for a
/// clean exit (code 0, no signal), an unreported exit, or a reported-but-empty
/// one (`code: None, signal: None` — a side channel that had nothing usable).
/// A signal takes priority over the code when both are present, since a
/// signal-killed process's reported exit code is rarely the meaningful part.
fn exit_badge_label(exit: &Option<CommandExit>) -> Option<String> {
    let exit = exit.as_ref()?;
    if let Some(signal) = exit.signal.as_deref() {
        return Some(s::agent_chat_tool_exit_signal(signal));
    }
    match exit.code {
        Some(code) if code != 0 => Some(s::agent_chat_tool_exit_code(code)),
        _ => None,
    }
}

/// Map a tool status to its badge label + colour.
fn tool_status_badge(
    status: ToolStatusView,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &App,
) -> (SharedString, Hsla) {
    match status {
        // `Pending` and `InProgress` both read as "running": the adapter marks
        // every call `Pending` until an SDK progress ping (which many tools
        // never get), and a live `Pending` always means an in-flight call in the
        // active turn (see `ToolStatusView::is_live`). Amber accent so a running
        // tool reads stronger than a settled green ✓ / red ✗; `tool_card`
        // appends animated dots to the label.
        ToolStatusView::Pending | ToolStatusView::InProgress => (
            s::agent_chat_tool_status_running().into(),
            t.status_executing_tool_dark,
        ),
        ToolStatusView::Completed => (
            s::agent_chat_tool_status_done().into(),
            t.file_diff_stat_add,
        ),
        ToolStatusView::Failed => (
            s::agent_chat_tool_status_failed().into(),
            t.banner_error_text,
        ),
        // Stopped before settling — muted like Pending (no error red, no
        // success green): it neither failed nor completed.
        ToolStatusView::Cancelled => (
            s::agent_chat_tool_status_cancelled().into(),
            theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim),
        ),
    }
}

/// Inline permission card — title + one button per choice. Once resolved,
/// the buttons are gone and the chosen option is shown instead.
pub(super) fn permission_card(
    ix: usize,
    card: &PermissionItem,
    t: &theme::DarudaTheme,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = card
        .tool_title
        .clone()
        .unwrap_or_else(s::agent_chat_permission_title)
        .into();

    let mut root = div()
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .w_full()
        .px(px(theme::AGENT_CHAT_INPUT_INNER_PAD_X))
        .py(px(theme::AGENT_CHAT_INPUT_INNER_PAD_Y))
        .rounded(px(theme::AGENT_CHAT_INPUT_RADIUS))
        .bg(t.banner_warning_bg)
        .child(
            div()
                .text_color(t.banner_warning_text)
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(SharedString::from(s::agent_chat_permission_title())),
        )
        .child(
            div()
                .text_color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                .text_size(px(theme::agent_chat_font_size(cx)))
                .child(
                    crate::ui::selectable_text(("agent-chat-perm-title", ix), title)
                        .color(theme::dim_toward_gray(theme::agent_chat_fg(cx), dim))
                        .text_size(px(theme::agent_chat_font_size(cx))),
                ),
        );

    match &card.resolved {
        Some(PermissionResolution::Chosen(option_id)) => {
            // Resolved: surface the chosen option's name (fall back to its
            // id) instead of the buttons.
            let chosen = card
                .options
                .iter()
                .find(|o| &o.option_id == option_id)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| option_id.clone());
            root = root.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(format!(
                        "{} {}",
                        s::agent_chat_permission_resolved_prefix(),
                        chosen
                    ))),
            );
        }
        Some(PermissionResolution::Cancelled) => {
            // The turn was cancelled before the user decided — drop the
            // buttons and surface that the request was cancelled.
            root = root.child(
                div()
                    .text_color(theme::dim_toward_gray(theme::agent_chat_fg_muted(cx), dim))
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .child(SharedString::from(s::agent_chat_permission_cancelled())),
            );
        }
        None => {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(theme::GAP_SM));
            for (choice_ix, choice) in card.options.iter().enumerate() {
                row = row.child(permission_button(ix, card.id, choice_ix, choice, cx));
            }
            root = root.child(row);
        }
    }

    root
}

/// One permission choice button. Allow kinds use the accent (primary)
/// treatment; reject kinds use the danger treatment.
fn permission_button(
    ix: usize,
    request_id: u64,
    choice_ix: usize,
    choice: &PermissionChoice,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    // Distinct id per (item, choice) pair; string-formatted rather than packed
    // arithmetically, so a card with many choices can't collide.
    let id = SharedString::from(format!("agent-chat-perm-{ix}-{choice_ix}"));
    let label: SharedString = choice.name.clone().into();
    let kind = choice.kind;
    let option_id = choice.option_id.clone();

    let button = match kind {
        PermissionKindView::AllowOnce | PermissionKindView::AllowAlways => {
            crate::ui::button_primary(id, label)
        }
        PermissionKindView::RejectOnce | PermissionKindView::RejectAlways => {
            crate::ui::button_danger(id, label)
        }
    };
    button.on_click(cx.listener(move |this, _, _window, cx| {
        this.respond_permission(request_id, option_id.clone(), kind, cx);
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_label_prefers_tool_name_over_kind() {
        // The agent's own tool name wins: "Bash" reads better than the generic
        // "Execute" kind, and the leading icon still carries the kind.
        assert_eq!(
            tool_header_label(Some("Bash"), ToolKindView::Execute),
            "Bash"
        );
    }

    #[test]
    fn header_label_falls_back_to_kind_without_tool_name() {
        // No tool name (e.g. an adapter that omits it) → the kind label, same as
        // before this feature. Locale-independent: compare against the kind label
        // rather than a hardcoded string.
        assert_eq!(
            tool_header_label(None, ToolKindView::Execute),
            tool_kind_label(ToolKindView::Execute)
        );
    }

    #[test]
    fn multiline_title_summarizes_to_first_line() {
        assert_eq!(
            tool_title_summary("Read src/main.rs\nlines 1-40"),
            "Read src/main.rs…"
        );
    }

    #[test]
    fn single_line_title_is_unchanged() {
        assert_eq!(tool_title_summary("Search TODO"), "Search TODO");
    }

    #[test]
    fn trailing_only_newline_does_not_add_ellipsis() {
        // A trailing newline (or a blank/whitespace second line) hides no real
        // content, so the title is returned as-is — no misleading "…".
        assert_eq!(tool_title_summary("Read foo\n"), "Read foo\n");
        assert_eq!(tool_title_summary("Read foo\n   "), "Read foo\n   ");
    }

    #[test]
    fn multiline_execute_title_also_summarizes() {
        // The full multiline command still has its own line in the expanded
        // body, so the collapsed one-line summary truncates it like any other
        // kind now — nothing is lost since the card is always expandable.
        let cmd = "cd /repo &&\n  cargo build\n  cargo test";
        assert_eq!(tool_title_summary(cmd), "cd /repo &&…");
    }

    #[test]
    fn exit_badge_hidden_when_no_exit_reported() {
        assert_eq!(exit_badge_label(&None), None);
    }

    #[test]
    fn exit_badge_hidden_for_clean_exit() {
        let exit = Some(CommandExit {
            code: Some(0),
            signal: None,
        });
        assert_eq!(exit_badge_label(&exit), None);
    }

    #[test]
    fn exit_badge_hidden_for_empty_report() {
        // Reported but carrying nothing usable — treat the same as absent.
        let exit = Some(CommandExit {
            code: None,
            signal: None,
        });
        assert_eq!(exit_badge_label(&exit), None);
    }

    #[test]
    fn exit_badge_shown_for_nonzero_code() {
        let exit = Some(CommandExit {
            code: Some(1),
            signal: None,
        });
        assert_eq!(
            exit_badge_label(&exit),
            Some(s::agent_chat_tool_exit_code(1))
        );
    }

    #[test]
    fn exit_badge_shown_for_signal_only() {
        let exit = Some(CommandExit {
            code: None,
            signal: Some("SIGKILL".to_string()),
        });
        assert_eq!(
            exit_badge_label(&exit),
            Some(s::agent_chat_tool_exit_signal("SIGKILL"))
        );
    }
}
