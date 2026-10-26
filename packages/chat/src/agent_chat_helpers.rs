//! GPUI-free (and view-context) helpers for the agent chat pane, shared by the
//! renderer, the row projection, the reconcilers, and the `Workspace` ops.
//!
//! Pure model/derivation helpers and the diff-model builders. Everything
//! here is either GPUI-free or takes a `Context<AgentChatView>`
//! (diff-editor creation); none of it needs `Workspace` state.

use super::fold::FoldKey;
use super::tool_status::{LiveSubagentUnits, effective_tool_status};
// Re-exported so the callers and tests that reach these through this module
// keep one path; `transcript_structure` is where the rules live.
pub(super) use super::transcript_structure::{is_active, response_run};
mod diff;
pub(crate) use diff::{
    DiffStat, build_diff_view_model, create_diff_editor, diff_build_fingerprint, diff_editor_key,
    diff_editor_language, diff_theme_fingerprint,
};
mod fold_context;
use super::view::AgentChatView;
pub(crate) use fold_context::{TurnBoundary, fold_context, fold_context_at, fold_key_item_index};
#[cfg(test)]
pub(crate) use fold_context::{fold_active, fold_turn};

/// Localize host-owned failure classes while retaining raw diagnostics in logs.
pub fn failure_message(failure: &daruda_acp::AcpFailure) -> String {
    match failure {
        daruda_acp::AcpFailure::TransportClosed { .. } => {
            crate::strings::agent_chat::transport_closed()
        }
        daruda_acp::AcpFailure::AdapterInstall { kind, .. } => {
            use crate::strings as s;
            use daruda_acp::preparation::PreparationKind;
            match kind {
                PreparationKind::Network | PreparationKind::Timeout | PreparationKind::Io => {
                    s::agent_chat::adapter_install_failed()
                }
                PreparationKind::Integrity => s::agent_chat::adapter_integrity_failed(),
                PreparationKind::Canceled => s::agent_chat::adapter_preparation_canceled(),
                PreparationKind::Configuration
                | PreparationKind::InvalidPackage
                | PreparationKind::Process => s::agent_chat::adapter_setup_required(),
            }
        }
        _ => failure.message().to_owned(),
    }
}

/// The id of the mode after `modes.current` in advertised order, wrapping at
/// the end. `None` when fewer than two modes are advertised (nothing to cycle).
/// If `current` is not in the list, cycling starts from the first mode. Pure
/// logic for `Workspace::cycle_agent_mode` (Shift+Tab).
pub(super) fn next_mode_id(modes: &daruda_acp::ModeStateView) -> Option<String> {
    if modes.available.len() < 2 {
        return None;
    }
    let current = modes
        .available
        .iter()
        .position(|m| m.id == modes.current)
        .unwrap_or(0);
    let next = (current + 1) % modes.available.len();
    Some(modes.available[next].id.clone())
}

/// The fold key for a tool call's card. A subagent launch — Claude's `Task`
/// tool, which carries `subagent_type` and/or `prompt` (see
/// [`daruda_acp::ToolCallItem::is_subagent_launch`]) — gets [`FoldKey::Subagent`]
/// so its card defaults collapsed: the subagent's flattened inner tool calls
/// nest inside the card and would otherwise fill the transcript while it runs.
/// Every other tool call gets the standard [`FoldKey::Tool`]. Single source
/// shared by the renderer (top-level and nested cards) and
/// [`collect_foldable_keys`] so a subagent's card, its click toggle, and
/// expand/collapse-all all agree on one key — a mismatch would make the toggle
/// write an override the card never reads.
pub(crate) fn tool_fold_key(tc: &daruda_acp::ToolCallItem) -> FoldKey {
    if tc.is_subagent_launch() {
        FoldKey::Subagent(tc.id.clone())
    } else {
        FoldKey::Tool(tc.id.clone())
    }
}

/// Whether a tool card renders its raw-input (JSON args) disclosure: a generic
/// tool (not a terminal `Execute`, whose command is already the title) that
/// carries args and has no diffs (an edit shows the diff instead). A subagent
/// launch is excluded too — it gets its own, purpose-built
/// [`renders_subagent_instructions`] section instead of the generic JSON dump.
/// Single source shared by the renderer and [`collect_foldable_keys`], so the
/// fold coverage matches what is actually on screen.
pub(crate) fn renders_raw_input(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.raw_input.is_some()
        && tc.diffs.is_empty()
        && !matches!(tc.kind, daruda_acp::ToolKindView::Execute)
        && !renders_subagent_instructions(tc)
}

/// Whether a tool card renders its "Instructions" section: the prompt handed
/// to a spawned subagent, plus its dispatch metadata (type, background),
/// formatted for a human rather than dumped as raw JSON. Always visible (no
/// fold) once shown — the prompt is the spec the subagent's work is judged
/// against, so hiding it behind a click undersells it. Gated on the prompt
/// actually being present — a `Task`-shaped call that is missing it falls back
/// to the generic [`renders_raw_input`] disclosure so its args are still
/// reachable. Single source shared by the renderer and this module's other
/// subagent-aware gates.
pub(crate) fn renders_subagent_instructions(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.subagent_prompt().is_some()
}

/// Whether a tool card's generic `Output` section must stay hidden because it
/// would just repeat the "Instructions" section above it: a subagent launch
/// that is still live echoes its own `prompt` back as `output` (the adapter's
/// content-block content), and that channel is only overwritten with the
/// subagent's actual result once the call settles (`fold_output`'s replace
/// semantics — see [`daruda_acp::ToolCallItem::subagent_prompt`]'s doc). Once
/// settled, `output` holds the distinct result summary and renders normally.
pub(super) fn suppresses_live_subagent_output(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.is_subagent_launch() && tc.status.is_live()
}

/// The outcome a fold header's rollup glyph summarizes over the run it stands
/// for. Single source shared by the response bar, the tool-group bar, and a
/// top-level assistant block, so the three can never disagree on treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rollup {
    /// At least one child still in progress / streaming (not settled).
    Running,
    /// All children succeeded.
    Ok,
    /// A mix — at least one failure alongside at least one success.
    Partial,
    /// Everything failed; nothing succeeded.
    Failed,
}

impl Rollup {
    /// The same classification, with the verdict scoped to the rows `keep`
    /// admits.
    ///
    /// The glyph sits on a disclosure, so ✓/⚠/✗ has to describe what expanding
    /// that row puts on screen — a failure the display filter removed leaves no
    /// visible row to explain the mark. Progress is the exception and stays
    /// filter-blind: a live descendant is what holds the row on screen at all
    /// (`group_live` ignores the filter too), so
    /// a glyph that settled while the work continued would deny its own row's
    /// reason for being there.
    pub(super) fn of_kept_run(
        items: &[daruda_acp::ChatItem],
        indices: impl Iterator<Item = usize>,
        live_units: &LiveSubagentUnits,
        keep: impl Fn(&daruda_acp::ChatItem) -> bool,
    ) -> Self {
        Self::of_items(
            indices.filter_map(|k| items.get(k)),
            |tc| effective_tool_status(tc, live_units),
            keep,
        )
    }

    fn of_items<'a>(
        items: impl Iterator<Item = &'a daruda_acp::ChatItem>,
        status: impl Fn(&daruda_acp::ToolCallItem) -> daruda_acp::ToolStatusView,
        keep: impl Fn(&daruda_acp::ChatItem) -> bool,
    ) -> Self {
        use daruda_acp::{ChatItem, ToolStatusView};

        let (mut running, mut any_failed, mut any_ok) = (false, false, false);
        for item in items {
            let counts = keep(item);
            match item {
                ChatItem::ToolCall(tc) => match status(tc) {
                    ToolStatusView::InProgress | ToolStatusView::Pending => running = true,
                    ToolStatusView::Failed => any_failed |= counts,
                    ToolStatusView::Completed => any_ok |= counts,
                    // Settled, neither success nor failure — sets no flag, so the
                    // run stops pulsing without turning the glyph red.
                    ToolStatusView::Cancelled => {}
                },
                ChatItem::AssistantText {
                    text, streaming, ..
                }
                | ChatItem::Thinking {
                    text, streaming, ..
                } => {
                    // Produced output counts as success, so a response that
                    // answered *and* hit a tool failure reads partial (⚠), not a
                    // hard failure (✗).
                    if counts && !text.trim().is_empty() {
                        any_ok = true;
                    }
                    running |= *streaming;
                }
                ChatItem::Failure(_) => any_failed |= counts,
                // A user message never belongs to a run; a permission card is
                // neither an outcome nor progress. A stop marker ends the run
                // rather than sitting in it (`agent_run`), so it reaches here
                // only through a whole-conversation scan.
                ChatItem::UserText(_)
                | ChatItem::Permission(_)
                | ChatItem::Interrupted
                | ChatItem::Compaction(_) => {}
            }
        }
        if running {
            Self::Running
        } else if !any_failed {
            Self::Ok
        } else if any_ok {
            Self::Partial
        } else {
            Self::Failed
        }
    }
}

/// Max glyphs the first-prompt fallback title keeps before ellipsizing, and the
/// head kept when it must (leaving room for the `…`). Mirrors Superset's
/// 72/69 first-message-title budget.
const FALLBACK_TITLE_MAX: usize = 72;
const FALLBACK_TITLE_HEAD: usize = 69;

/// The activity-bar title: the agent-supplied session title when set, else a
/// fallback derived from the first user prompt (whitespace-normalized and
/// glyph-truncated), else `None` for a still-empty session (the caller supplies
/// the pane's agent name fallback). Precedence mirrors Superset's
/// session-selector (`session title → first-message fallback → agent name`);
/// zed's constant-string fallback is intentionally *not* copied.
pub fn activity_bar_title(
    session_title: Option<&str>,
    items: &[daruda_acp::ChatItem],
) -> Option<String> {
    if let Some(title) = session_title.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(title.to_string());
    }
    items.iter().find_map(|item| match item {
        daruda_acp::ChatItem::UserText(text) => {
            let title = normalize_prompt_title(text);
            (!title.is_empty()).then_some(title)
        }
        _ => None,
    })
}

/// Collapse a user prompt to a single-line title: trim, collapse internal
/// whitespace runs to one space, and glyph-truncate (never byte-slice, so a
/// multibyte prompt can't split a char). Empty when the prompt is whitespace.
///
/// Stops one glyph past the budget rather than normalizing the whole prompt
/// first: the result is at most [`FALLBACK_TITLE_MAX`] glyphs, so the cost must
/// track the title, not the message. Reaching that glyph is also what decides
/// ellipsis — the full text is longer than the budget exactly when the walk
/// doesn't run out of words first.
fn normalize_prompt_title(text: &str) -> String {
    let decisive = FALLBACK_TITLE_MAX + 1;
    let mut head = String::new();
    let mut glyphs = 0usize;
    'walk: for word in text.split_whitespace() {
        if glyphs > 0 {
            head.push(' ');
            glyphs += 1;
            if glyphs == decisive {
                break 'walk;
            }
        }
        for ch in word.chars() {
            head.push(ch);
            glyphs += 1;
            if glyphs == decisive {
                break 'walk;
            }
        }
    }
    if glyphs < decisive {
        return head;
    }
    let kept = daruda_core::text::char_prefix(&head, FALLBACK_TITLE_HEAD).0;
    format!("{}…", kept.trim_end())
}

/// The single-line preview shown next to a collapsed assistant / thinking
/// header: the first non-empty line of `text` with inline markdown flattened to
/// plain text, so `**bold**`, `` `code` ``, `[link](url)`, headings, and list
/// markers read as clean prose instead of raw syntax (agents routinely open a
/// reasoning block with a bolded one-liner like `**Planning the change**`).
///
/// The whole source is parsed, then text / code spans are concatenated with a
/// newline on each soft/hard break and block end, so the "first line" is the
/// first non-empty *rendered* line even when an emphasis run wraps across a
/// source newline. Returns `None` when there is no visible content.
pub(super) fn summary_preview_line(text: &str) -> Option<String> {
    use pulldown_cmark::{Event, Options, Parser, TagEnd};

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut flattened = String::with_capacity(text.len());
    for event in Parser::new_ext(text, opts) {
        match event {
            Event::Text(t) | Event::Code(t) => flattened.push_str(&t),
            Event::SoftBreak | Event::HardBreak => flattened.push('\n'),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                flattened.push('\n')
            }
            _ => {}
        }
    }
    flattened
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// Every markdown body of a chat item that can carry a ```mermaid fence —
/// assistant / thinking / user text, each `Text` output block of a tool call
/// (a tool that reads or writes a .md file streams fences there), and a
/// subagent launch's `prompt` (rendered as markdown in the "Instructions"
/// section — see `renders_subagent_instructions`). Permission / error items
/// carry none. A `RawText` block is excluded on purpose — it is verbatim shell
/// output, not a markdown body, so a fence-shaped run of characters a command
/// printed must not become a diagram. Drives the mermaid scan
/// (`AgentChatView::reconcile_mermaid`) that rasterizes a fence *before* the
/// render hook (`mermaid_code_block_render`) can show it — a text source
/// missing here never gets its diagram cached, no matter how the renderer is
/// wired.
pub(super) fn chat_item_mermaid_texts(item: &daruda_acp::ChatItem) -> Vec<&str> {
    match item {
        daruda_acp::ChatItem::AssistantText { text, .. }
        | daruda_acp::ChatItem::Thinking { text, .. } => vec![text],
        daruda_acp::ChatItem::UserText(text) => vec![text],
        daruda_acp::ChatItem::ToolCall(tc) => tc
            .output
            .iter()
            // `Text` only: a `SourceText` block is a file's contents, so a fence
            // inside it is text the file holds, not a diagram the tool drew.
            .filter_map(|block| match block {
                daruda_acp::ToolOutputBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .chain(tc.subagent_prompt())
            .collect(),
        daruda_acp::ChatItem::Compaction(c) => c
            .summary
            .iter()
            .filter_map(|block| match block {
                daruda_acp::ToolOutputBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect(),
        daruda_acp::ChatItem::Permission(_)
        | daruda_acp::ChatItem::Failure(_)
        | daruda_acp::ChatItem::Interrupted => Vec::new(),
    }
}

/// Stable cache key for a mermaid fence's source *at a given appearance*, shared
/// between the rasterizer (insert) and the renderer (lookup) so the embed
/// matches what was cached. `dark` is part of the key because the diagram is
/// themed to the host appearance (`daruda_content`'s `mermaid_host_theme_profile`): without it a cached
/// raster would keep its old colours after a light/dark toggle. `DefaultHasher`
/// is process-stable, which is all the in-memory cache needs.
pub(super) fn mermaid_key(source: &str, dark: bool) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

/// Stable cache key for a tool-output `Image` block's base64 payload, shared
/// between the decoder (`reconcile_tool_images`, insert) and the renderer
/// (`output_block_view`, lookup) so the embed matches what was cached.
/// `DefaultHasher` is process-stable, which is all the in-memory cache needs —
/// not cryptographic, so a collision would at worst reuse a cached texture.
pub(super) fn tool_image_key(data: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// Extract the source of every **closed** ` ```mermaid ` fence in `text`, in
/// document order. Only closed fences are returned: a still-streaming (never
/// terminated) trailing `mermaid` fence is skipped so a half-arrived diagram
/// isn't rasterized until it completes. Non-mermaid fences are ignored.
///
/// A mermaid fence opens on a line whose trimmed content is exactly ```` ```mermaid ````
/// (optionally with trailing spaces) and closes on the next line whose trimmed
/// content is ```` ``` ````. Leading indentation on the fence lines is tolerated;
/// the captured source keeps the lines between the fences verbatim.
pub(super) fn mermaid_sources(text: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "```mermaid" {
            continue;
        }
        // Inside a mermaid fence — collect until the closing ``` line. If the
        // text ends first the fence is unterminated (still streaming): drop it.
        let mut body: Vec<&str> = Vec::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if inner.trim() == "```" {
                closed = true;
                break;
            }
            body.push(inner);
        }
        if closed {
            sources.push(body.join("\n"));
        }
    }
    sources
}

/// True when `items` holds conversation content that a session teardown would
/// destroy. [`ChatItem::Failure`](daruda_acp::ChatItem::Failure) does not count:
/// it is a session-failure notice, not the user's transcript, so a pane whose
/// only content is "session limit reached" — the usual reason to reach for
/// another account — still counts as empty and can reconnect in place.
///
/// The host's account switch reads it to decide whether it may reuse the pane.
pub fn has_conversation(items: &[daruda_acp::ChatItem]) -> bool {
    use daruda_acp::ChatItem;
    items.iter().any(|it| !matches!(it, ChatItem::Failure(_)))
}

/// The unresolved permission card carrying request `id`, if `items` still holds
/// one. Found by id, not by position: several permissions can be outstanding at
/// once (parallel tool calls), so the trailing card is not necessarily the one
/// being answered.
pub(super) fn permission_card_mut(
    view: &mut AgentChatView,
    id: u64,
) -> Option<&mut daruda_acp::PermissionItem> {
    view.items.iter_mut().rev().find_map(|item| match item {
        daruda_acp::ChatItem::Permission(card) if card.id == id && card.resolved.is_none() => {
            Some(card)
        }
        _ => None,
    })
}

/// Cancel-drain *every* outstanding permission request: respond to the agent
/// with a `Cancelled` outcome for each parked id and mark each unresolved card
/// cancelled so its buttons disable. No-op when nothing is pending; idempotent.
/// ACP requires the client to resolve a pending permission with a cancelled
/// outcome on `session/cancel`; this also runs when a turn ends or errors before
/// the user decided, so no card is left stuck with live buttons.
pub(super) fn cancel_pending_permission(
    view: &mut AgentChatView,
    cx: &mut gpui::Context<AgentChatView>,
) {
    if view.pending_permissions.is_empty() {
        return;
    }
    for id in std::mem::take(&mut view.pending_permissions) {
        if let Some(handle) = view.any_handle() {
            handle.respond_permission(id, daruda_acp::PermissionDecision::Cancelled);
        }
        cx.emit(crate::view::AgentChatEvent::PermissionSettled {
            session: view.permission_session,
            request_id: id,
            outcome: crate::view::PermissionOutcome::Cancelled,
        });
    }
    // Mark *every* unresolved card cancelled — a UI safety net that intentionally
    // does not key off the set. Under the invariant the two are equal, but the
    // asymmetry (a backend `Cancelled` is sent only for set ids) is deliberate:
    // don't couple the UI marking to set membership to "align" them.
    for item in view.items.iter_mut() {
        if let daruda_acp::ChatItem::Permission(card) = item
            && card.resolved.is_none()
        {
            card.resolved = Some(daruda_acp::PermissionResolution::Cancelled);
        }
    }
}

/// Apply one `SessionInfoUpdate` field change to a cached `Option<String>`
/// slot. `Unchanged` leaves the slot as-is (the update omitted the field);
/// `Cleared` resets it to `None`; `Set` overwrites it. Shared by the title and
/// last-activity fields so both honour the protocol's per-field tri-state.
pub(super) fn apply_info_field(slot: &mut Option<String>, change: daruda_acp::InfoFieldChange) {
    match change {
        daruda_acp::InfoFieldChange::Unchanged => {}
        daruda_acp::InfoFieldChange::Cleared => *slot = None,
        daruda_acp::InfoFieldChange::Set(value) => *slot = Some(value),
    }
}

#[cfg(test)]
mod tests;
