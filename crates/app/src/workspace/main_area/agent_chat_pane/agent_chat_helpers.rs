//! GPUI-free (and view-context) helpers for the agent chat pane, shared by the
//! renderer, the row projection, the reconcilers, and the `Workspace` ops.
//!
//! Split out of [`agent_chat_ops`](super::agent_chat_ops) — which now holds only
//! the `impl Workspace` connection / event-pump / dock-routing methods — because
//! these are a distinct responsibility cluster (pure model/derivation helpers +
//! the diff-model builders) with their own test fixtures. Everything here is
//! either GPUI-free or takes a `Context<AgentChatView>` (diff-editor creation);
//! none of it needs `Workspace` state.

use daruda_acp::DiffView;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{AnyWindowHandle, AppContext as _, Context, Entity};

use super::fold::{FoldKey, FoldState};
use super::rows::{RowKind, project};
use super::view::AgentChatView;
use crate::path_ext::PathExt as _;
use crate::workspace::main_area::file_view_pane::diff_editor::{
    DiffColors, DiffEditorModel, build_diff_editor_model,
};
use crate::workspace::main_area::pane_tree::PaneId;

/// The id of the mode after `modes.current` in advertised order, wrapping at
/// the end. `None` when fewer than two modes are advertised (nothing to cycle).
/// If `current` is not in the list, cycling starts from the first mode. Pure
/// logic for `Workspace::cycle_agent_mode` (Shift+Tab).
pub(in crate::workspace) fn next_mode_id(modes: &daruda_acp::ModeStateView) -> Option<String> {
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
/// tool, which carries `subagent_type` — gets [`FoldKey::Subagent`] so its card
/// defaults collapsed: the subagent's flattened inner tool calls nest inside the
/// card and would otherwise fill the transcript while it runs. Every other tool
/// call gets the standard [`FoldKey::Tool`]. Single source shared by the
/// renderer (top-level and nested cards) and [`collect_foldable_keys`] so a
/// subagent's card, its click toggle, and expand/collapse-all all agree on one
/// key — a mismatch would make the toggle write an override the card never reads.
pub(in crate::workspace) fn tool_fold_key(tc: &daruda_acp::ToolCallItem) -> FoldKey {
    if tc.subagent_type().is_some() {
        FoldKey::Subagent(tc.id.clone())
    } else {
        FoldKey::Tool(tc.id.clone())
    }
}

/// The visible foldable-key set for a conversation: each assistant / thinking
/// item by index, each tool call by id plus one `Diff` key per diff it carries
/// (the same `diff_editor_key` the renderer embeds with). User / permission /
/// error items are not foldable and contribute none. Single source of truth for
/// expand-all / collapse-all (`AgentChatView::set_all_folds`) and the coverage
/// test.
pub(in crate::workspace) fn collect_foldable_keys(items: &[daruda_acp::ChatItem]) -> Vec<FoldKey> {
    let mut keys: Vec<FoldKey> = Vec::new();
    // Structural fold levels (response / tool-group) come from the same row
    // projection the renderer uses, so expand/collapse-all covers exactly the
    // headers on screen. Neither the fold state nor live progress changes
    // which headers exist, so project with defaults for both.
    let rows = project(items, &FoldState::default(), false);
    // Assistant prose rendered under a response bar is inline (no per-block
    // header/fold — the response bar owns the speaker label), so its
    // `FoldKey::Assistant` would be a dead toggle. Such rows are `AgentItem`s at
    // indent > 0; skip their keys so the fold set matches the on-screen headers.
    let inline_assistant: std::collections::HashSet<usize> = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::AgentItem(ix) if row.indent > 0 => Some(ix),
            _ => None,
        })
        .collect();
    for row in &rows {
        match &row.kind {
            RowKind::ResponseHeader { anchor, .. } => keys.push(FoldKey::Response(*anchor)),
            RowKind::ToolGroupHeader { gid, .. } => keys.push(FoldKey::ToolGroup(gid.clone())),
            // The conclusion's own `FoldKey::Assistant` is added by the per-block
            // loop below (it is not in `inline_assistant`), so nothing to do here.
            RowKind::User(_)
            | RowKind::AgentItem(_)
            | RowKind::ConclusionItem(_)
            | RowKind::WorkingIndicator => {}
        }
    }
    // Per-block fold levels (assistant / thinking by index, tool + its diffs by
    // id).
    for (ix, item) in items.iter().enumerate() {
        match item {
            daruda_acp::ChatItem::AssistantText { .. } if inline_assistant.contains(&ix) => {}
            daruda_acp::ChatItem::AssistantText { .. } => keys.push(FoldKey::Assistant(ix)),
            daruda_acp::ChatItem::Thinking { .. } => keys.push(FoldKey::Thinking(ix)),
            daruda_acp::ChatItem::ToolCall(tc) => {
                keys.push(tool_fold_key(tc));
                for di in 0..tc.diffs.len() {
                    keys.push(FoldKey::Diff(diff_editor_key(&tc.id, di)));
                }
                // Mirror the renderer's raw-input gate (generic tool, no diffs,
                // has args) so expand/collapse-all covers the disclosure.
                if renders_raw_input(tc) {
                    keys.push(FoldKey::ToolRawInput(tc.id.clone()));
                }
            }
            daruda_acp::ChatItem::UserText(_)
            | daruda_acp::ChatItem::Permission(_)
            | daruda_acp::ChatItem::Error(_) => {}
        }
    }
    keys
}

/// Whether a tool card renders its raw-input (JSON args) disclosure: a generic
/// tool (not a terminal `Execute`, whose command is already the title) that
/// carries args and has no diffs (an edit shows the diff instead). Single
/// source shared by the renderer and [`collect_foldable_keys`], so the fold
/// coverage matches what is actually on screen.
pub(in crate::workspace) fn renders_raw_input(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.raw_input.is_some()
        && tc.diffs.is_empty()
        && !matches!(tc.kind, daruda_acp::ToolKindView::Execute)
}

/// Cache key for a tool call's `di`-th diff editor: one editor per file. Shared
/// with the renderer so the embed lookup matches the insert key.
pub(in crate::workspace) fn diff_editor_key(tool_call_id: &str, di: usize) -> String {
    format!("{tool_call_id}#{di}")
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
pub(in crate::workspace) fn activity_bar_title(
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
fn normalize_prompt_title(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() > FALLBACK_TITLE_MAX {
        let head: String = normalized.chars().take(FALLBACK_TITLE_HEAD).collect();
        format!("{}…", head.trim_end())
    } else {
        normalized
    }
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
pub(in crate::workspace) fn summary_preview_line(text: &str) -> Option<String> {
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

/// The markdown body of a chat item that can carry a ` ```mermaid ` fence —
/// assistant / thinking / user text. Tool / permission / error items carry no
/// markdown body and contribute none. Drives the mermaid scan.
pub(in crate::workspace) fn chat_item_markdown(item: &daruda_acp::ChatItem) -> Option<&str> {
    match item {
        daruda_acp::ChatItem::AssistantText { text, .. }
        | daruda_acp::ChatItem::Thinking { text, .. } => Some(text),
        daruda_acp::ChatItem::UserText(text) => Some(text),
        daruda_acp::ChatItem::ToolCall(_)
        | daruda_acp::ChatItem::Permission(_)
        | daruda_acp::ChatItem::Error(_) => None,
    }
}

/// Stable cache key for a mermaid fence's source *at a given appearance*, shared
/// between the rasterizer (insert) and the renderer (lookup) so the embed
/// matches what was cached. `dark` is part of the key because the diagram is
/// themed to the host appearance (`mermaid_with_theme`): without it a cached
/// raster would keep its old colours after a light/dark toggle. `DefaultHasher`
/// is process-stable, which is all the in-memory cache needs.
pub(in crate::workspace) fn mermaid_key(source: &str, dark: bool) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
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
pub(in crate::workspace) fn mermaid_sources(text: &str) -> Vec<String> {
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

/// Added / removed line counts for one tool-call diff, used by the fold summary
/// (`+N −M`) shown when the diff editor is collapsed. Counted from the *same*
/// hunks that build the diff editor (see [`build_diff_view_model`]), so the
/// numbers match what the editor renders exactly. Cached alongside the editor
/// in `AgentChatView.diff_stats`, keyed by [`diff_editor_key`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct DiffStat {
    pub(in crate::workspace) added: usize,
    pub(in crate::workspace) removed: usize,
}

/// Language id for an editor's syntax tree, from the diff's file extension.
/// Empty when unknown (the editor falls back to `"text"`).
pub(in crate::workspace) fn diff_editor_language(diff: &DiffView) -> &'static str {
    match diff.path.extension_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "py" => "python",
        "go" => "go",
        "toml" => "toml",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sh" | "bash" | "zsh" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        _ => "",
    }
}

/// Convert a tool-call [`DiffView`] into the editor inputs the shared
/// diff-through-editor renderer consumes, plus the [`DiffStat`] for the same
/// diff. Pure / GPUI-free: builds the unified diff from `old_text`/`new_text`,
/// syntax-highlights and word-diffs the hunks exactly as the File viewer's
/// `load_diff` does, then folds them into a [`DiffEditorModel`].
///
/// The stat is counted from those *same* hunks (via [`diff_stat_from_hunks`]),
/// so it matches the rendered editor line-for-line.
///
/// Returns `None` when the two sides are identical (no hunks → nothing to
/// render), so the caller leaves the inline fallback in place and records no
/// stat entry (absent ≡ `0/0`).
pub(in crate::workspace) fn build_diff_view_model(
    diff: &DiffView,
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> Option<(DiffEditorModel, DiffStat)> {
    use crate::workspace::main_area::file_view_pane::highlighter::highlight_hunks;
    use crate::workspace::main_area::file_view_pane::line_diff::unified_diff_text;
    use crate::workspace::main_area::file_view_pane::word_diff::apply_word_diff;
    use crate::workspace::main_area::file_view_pane::{build_diff_rows, parse_diff_hunks};

    let old = diff.old_text.as_deref().unwrap_or("");
    let text = unified_diff_text(old, &diff.new_text);
    let mut hunks = parse_diff_hunks(&text);
    if hunks.is_empty() {
        return None;
    }
    // Count add/remove from the parsed hunks before they are highlighted /
    // word-diffed (those passes only annotate, never reclassify lines), so the
    // stat is from the exact same diff that builds the editor below.
    let stat = diff_stat_from_hunks(&hunks);
    let ext = diff.path.extension_str();
    highlight_hunks(&mut hunks, ext, syntax_theme, is_light);
    apply_word_diff(&mut hunks);
    let rows = build_diff_rows(&hunks, false);
    Some((build_diff_editor_model(&rows, colors), stat))
}

/// Tally a [`DiffStat`] from parsed diff hunks. Pure / GPUI-free wrapper over
/// the File viewer's `count_diff_stats`, which counts `DiffLine::Added` vs
/// `DiffLine::Removed` across the hunks — the same line classification the
/// editor rows are built from.
fn diff_stat_from_hunks(
    hunks: &[crate::workspace::main_area::file_view_pane::DiffHunk],
) -> DiffStat {
    let (added, removed) = crate::workspace::main_area::file_view_pane::count_diff_stats(hunks);
    DiffStat { added, removed }
}

/// Create + configure a read-only diff editor entity inside a single window
/// re-entry against the view's stored `window_handle`. Mirrors the File
/// viewer's editor construction (`multi_line` + `soft_wrap(false)` +
/// `code_editor`) and the diff-config it applies (`set_disabled(true)` for
/// read-only + decorations + injected highlight spans). Returns `None` if the
/// owning window is gone.
///
/// Uses the stored `window_handle` rather than
/// `WindowRegistry::handle_for_workspace(cx.entity_id())` because after the
/// pane became its own entity `cx.entity_id()` is the view, not the Workspace,
/// so the registry would no longer resolve the window.
pub(in crate::workspace) fn create_diff_editor(
    cx: &mut Context<AgentChatView>,
    window_handle: AnyWindowHandle,
    pane_id: PaneId,
    language: &str,
    model: DiffEditorModel,
) -> Option<Entity<gpui_component::input::InputState>> {
    let language = language.to_owned();
    match cx.update_window(window_handle, move |_, window, cx_w| {
        cx_w.new(|cx_state| {
            // One synthetic-buffer line per decoration (no trailing newline in
            // `model.text`), so this is the editor's display-row count. Seeding
            // it up front makes `display_rows()` correct from the first render
            // — the diff body reads it to size the (parent-height-less) editor
            // to its full content instead of a collapsed single line.
            let rows = model.decorations.len().max(1);
            let mut state = gpui_component::input::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false);
            state = if language.is_empty() {
                state.code_editor("text")
            } else {
                state.code_editor(&language)
            };
            state = state.rows(rows);
            state.set_value(model.text, window, cx_state);
            state.set_disabled(true, cx_state);
            state.set_line_decorations(model.decorations, cx_state);
            state.set_highlight_override(Some(model.highlights), cx_state);
            state
        })
    }) {
        Ok(editor) => Some(editor),
        Err(e) => {
            // Window gone mid-stream — drop this editor; the inline fallback
            // renders. Logged so it isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Failed to build agent-chat diff editor")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup(format!("agent_chat.diff_editor.window_gone.{pane_id}"))
                    .build(),
            );
            None
        }
    }
}

/// Whether a chat block is currently streaming / in progress — the `active`
/// input the fold derivation reads. A streaming text or thinking block, or a
/// tool call still `InProgress`, is active; everything else (settled text,
/// finished/failed tool calls, user / permission / error items) is not. Shared
/// by [`AgentChatView::toggle_fold`] and the renderer so both derive the same
/// effective fold state.
pub(in crate::workspace) fn is_active(item: &daruda_acp::ChatItem) -> bool {
    use daruda_acp::ChatItem;
    match item {
        ChatItem::AssistantText { streaming, .. } | ChatItem::Thinking { streaming, .. } => {
            *streaming
        }
        ChatItem::ToolCall(tc) => tc.status.is_live(),
        ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Error(_) => false,
    }
}

/// The `active` input [`FoldState::is_expanded`](super::fold::FoldState::is_expanded)
/// reads for `key`, derived from the live conversation. A block is active while
/// it streams / runs; a response is active while it is the last turn or its run
/// still streams; a tool group is active while any member runs. `Diff` /
/// `ToolRawInput` ignore `active` in their [`FoldPolicy`](super::fold), so this
/// returns `false` for them (the value is irrelevant).
///
/// Single source of truth for "is this fold active", shared by projection and
/// toggle paths so they cannot disagree on default collapse state.
pub(in crate::workspace) fn fold_active(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> bool {
    use daruda_acp::ChatItem;
    match key {
        FoldKey::Assistant(ix) | FoldKey::Thinking(ix) => {
            items.get(*ix).map(is_active).unwrap_or(false)
        }
        FoldKey::Tool(id) => items
            .iter()
            .find_map(|item| match item {
                ChatItem::ToolCall(tc) if tc.id == *id => Some(is_active(item)),
                _ => None,
            })
            .unwrap_or(false),
        // A response is active while it is the last turn or its run (anchor+1 up
        // to the next user message) still streams.
        FoldKey::Response(anchor) => {
            let start = anchor + 1;
            let end = items
                .iter()
                .skip(start)
                .position(|it| matches!(it, ChatItem::UserText(_)))
                .map(|off| start + off)
                .unwrap_or(items.len());
            let is_last = end >= items.len();
            let streaming = items
                .get(start..end)
                .is_some_and(|run| run.iter().any(is_active));
            is_last || streaming
        }
        // The group is the consecutive tool-call run beginning at `gid`; active
        // while any tool in it is still running.
        FoldKey::ToolGroup(gid) => items
            .iter()
            .position(|item| matches!(item, ChatItem::ToolCall(tc) if tc.id == *gid))
            .map(|s| {
                items[s..]
                    .iter()
                    .take_while(|item| matches!(item, ChatItem::ToolCall(_)))
                    .any(is_active)
            })
            .unwrap_or(false),
        // Diff (DefaultExpanded), raw-input and subagent (DefaultCollapsed) all
        // ignore `active` in their policy, so the value is irrelevant here.
        FoldKey::Diff(_) | FoldKey::ToolRawInput(_) | FoldKey::Subagent(_) => false,
    }
}

/// The unresolved permission card carrying request `id`, if `items` still holds
/// one. Found by id, not by position: several permissions can be outstanding at
/// once (parallel tool calls), so the trailing card is not necessarily the one
/// being answered.
pub(in crate::workspace) fn permission_card_mut(
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
pub(in crate::workspace) fn cancel_pending_permission(view: &mut AgentChatView) {
    if view.pending_permissions.is_empty() {
        return;
    }
    for id in std::mem::take(&mut view.pending_permissions) {
        if let Some(handle) = &view.handle {
            handle.respond_permission(id, daruda_acp::PermissionDecision::Cancelled);
        }
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
pub(in crate::workspace) fn apply_info_field(
    slot: &mut Option<String>,
    change: daruda_acp::InfoFieldChange,
) {
    match change {
        daruda_acp::InfoFieldChange::Unchanged => {}
        daruda_acp::InfoFieldChange::Cleared => *slot = None,
        daruda_acp::InfoFieldChange::Set(value) => *slot = Some(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ChatItem, ModeStateView, SessionModeView, ToolCallItem};

    fn asst(text: &str) -> ChatItem {
        ChatItem::AssistantText {
            text: text.to_owned(),
            streaming: false,
            message_id: None,
        }
    }

    #[test]
    fn activity_bar_title_prefers_the_session_title() {
        let items = [ChatItem::UserText("run the tests".to_owned())];
        assert_eq!(
            activity_bar_title(Some("Refactor fold state"), &items).as_deref(),
            Some("Refactor fold state")
        );
    }

    #[test]
    fn activity_bar_title_falls_back_to_first_user_prompt() {
        // No session title yet (pre first turn-end): the first prompt stands in.
        let items = [
            ChatItem::UserText("  fix the   parser  ".to_owned()),
            asst("sure"),
            ChatItem::UserText("second".to_owned()),
        ];
        assert_eq!(
            activity_bar_title(None, &items).as_deref(),
            Some("fix the parser")
        );
    }

    #[test]
    fn activity_bar_title_ignores_blank_session_title_and_falls_back() {
        let items = [ChatItem::UserText("hello".to_owned())];
        assert_eq!(
            activity_bar_title(Some("   "), &items).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn summary_preview_line_flattens_inline_markdown() {
        // Bold one-liner (the common reasoning-block opener) reads as prose.
        assert_eq!(
            summary_preview_line("**Planning the change** and more").as_deref(),
            Some("Planning the change and more")
        );
        // Inline code, links, and italics all flatten to their visible text.
        assert_eq!(
            summary_preview_line("Call `foo()` in [the module](https://x)").as_deref(),
            Some("Call foo() in the module")
        );
        // Leading blank lines and a heading marker are skipped / stripped.
        assert_eq!(
            summary_preview_line("\n\n# Title here\nbody").as_deref(),
            Some("Title here")
        );
        // A list marker on the first line is dropped, keeping the item text.
        assert_eq!(
            summary_preview_line("- first item\n- second").as_deref(),
            Some("first item")
        );
    }

    #[test]
    fn summary_preview_line_is_none_when_empty() {
        assert_eq!(summary_preview_line(""), None);
        assert_eq!(summary_preview_line("   \n\t\n"), None);
    }

    #[test]
    fn activity_bar_title_is_none_for_an_empty_session() {
        // Neither a session title nor a user prompt → blank bar (no placeholder).
        assert_eq!(activity_bar_title(None, &[]), None);
        // Non-user leading items don't seed a title.
        assert_eq!(activity_bar_title(None, &[asst("greeting")]), None);
        // A whitespace-only prompt yields nothing.
        assert_eq!(
            activity_bar_title(None, &[ChatItem::UserText("   ".to_owned())]),
            None
        );
    }

    #[test]
    fn normalize_prompt_title_truncates_long_prompts_on_a_char_boundary() {
        let long = "가".repeat(100);
        let title = normalize_prompt_title(&long);
        // 69 kept glyphs + the ellipsis (never a split multibyte char).
        assert_eq!(title.chars().count(), FALLBACK_TITLE_HEAD + 1);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn normalize_prompt_title_keeps_short_prompts_verbatim() {
        assert_eq!(normalize_prompt_title("short one"), "short one");
    }

    fn modes(ids: &[&str], current: &str) -> ModeStateView {
        ModeStateView {
            available: ids
                .iter()
                .map(|id| SessionModeView {
                    id: (*id).to_string(),
                    name: (*id).to_string(),
                    description: None,
                })
                .collect(),
            current: current.to_string(),
        }
    }

    #[test]
    fn next_mode_id_wraps_through_advertised_order() {
        let m = modes(&["default", "acceptEdits", "bypassPermissions"], "default");
        assert_eq!(next_mode_id(&m).as_deref(), Some("acceptEdits"));
        let m = modes(
            &["default", "acceptEdits", "bypassPermissions"],
            "acceptEdits",
        );
        assert_eq!(next_mode_id(&m).as_deref(), Some("bypassPermissions"));
        // Wrap: last → first.
        let m = modes(
            &["default", "acceptEdits", "bypassPermissions"],
            "bypassPermissions",
        );
        assert_eq!(next_mode_id(&m).as_deref(), Some("default"));
    }

    #[test]
    fn next_mode_id_none_when_not_cyclable() {
        // Zero or one advertised mode → nothing to cycle.
        assert_eq!(next_mode_id(&modes(&[], "")), None);
        assert_eq!(next_mode_id(&modes(&["default"], "default")), None);
    }

    #[test]
    fn next_mode_id_starts_from_first_when_current_unknown() {
        let m = modes(&["default", "acceptEdits"], "stale-id");
        assert_eq!(next_mode_id(&m).as_deref(), Some("acceptEdits"));
    }

    /// A syntax theme id every test reuses for the highlight passes.
    const TEST_SYNTAX_THEME: &str = "base16-ocean.dark";

    /// A flat `DiffColors` fixture so the pure model build is testable without a
    /// live theme.
    fn diff_colors() -> DiffColors {
        let c = |l: f32| gpui::Hsla {
            h: 0.,
            s: 0.,
            l,
            a: 1.,
        };
        DiffColors {
            add_bg: c(0.1),
            del_bg: c(0.11),
            hunk_bg: c(0.12),
            add_text: c(0.2),
            del_text: c(0.21),
            ctx_text: c(0.22),
            hunk_text: c(0.23),
            hunk_ctx_text: c(0.24),
            word_add_bg: c(0.3),
            word_del_bg: c(0.31),
        }
    }

    fn diff(old: Option<&str>, new: &str, path: &str) -> DiffView {
        DiffView {
            path: std::path::PathBuf::from(path),
            old_text: old.map(str::to_owned),
            new_text: new.to_owned(),
        }
    }

    /// `build_diff_view_model` turns a single-line modification into a
    /// `DiffEditorModel` whose synthetic buffer carries the hunk header plus
    /// both sides (no `+`/`-` markers — the kind is in the decorations) and
    /// whose per-row decorations include add/del backgrounds.
    #[test]
    fn diff_view_model_builds_rows_and_decorations() {
        let d = diff(
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet y = 2;\nfn b() {}\n",
            "src/lib.rs",
        );
        let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a modified file produces hunks");
        // Hunk header row + content rows, no marker prefix on content.
        assert!(m.text.starts_with("@@"), "buffer leads with a hunk header");
        assert!(m.text.contains("let x = 1;"), "removed line present");
        assert!(m.text.contains("let y = 2;"), "added line present");
        // Some rows carry an add/del background (the changed pair).
        let with_bg = m
            .decorations
            .iter()
            .filter(|d| d.background.is_some())
            .count();
        assert!(with_bg >= 2, "at least the changed pair is tinted");
        // One decoration per synthetic-buffer line (no trailing newline), so
        // `decorations.len()` is the editor's display-row count — the value
        // `create_diff_editor` seeds and the tool-card diff body uses to size
        // the editor to its full content. Lock that relationship.
        assert_eq!(
            m.decorations.len(),
            m.text.split('\n').count(),
            "one decoration per display row drives the inline diff height"
        );
    }

    /// A newly created file (`old_text == None`) diffs against an empty old
    /// side — every line is an addition, so the model is built (non-empty).
    #[test]
    fn diff_view_model_handles_created_file() {
        let d = diff(None, "line one\nline two\n", "new.txt");
        let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert!(m.text.contains("line one"));
        assert!(m.text.contains("line two"));
    }

    /// Identical sides yield no hunks, so the adapter returns `None` and the
    /// caller keeps the inline fallback.
    #[test]
    fn diff_view_model_none_when_unchanged() {
        let d = diff(Some("same\n"), "same\n", "same.txt");
        assert!(build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors()).is_none());
    }

    /// A simple one-line modification must report the *changed* line on each
    /// side — `added = 1, removed = 1` — not the file's total line counts.
    #[test]
    fn diff_stat_counts_changed_lines_not_totals() {
        let d = diff(
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet y = 2;\nfn b() {}\n",
            "src/lib.rs",
        );
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a modified file produces hunks");
        assert_eq!(
            stat,
            DiffStat {
                added: 1,
                removed: 1
            }
        );
    }

    /// A newly created file (`old_text == None`) diffs against an empty old
    /// side, so every line is an addition: `added = N, removed = 0`.
    #[test]
    fn diff_stat_new_file_is_all_added() {
        let d = diff(None, "line one\nline two\nline three\n", "new.txt");
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert_eq!(
            stat,
            DiffStat {
                added: 3,
                removed: 0
            }
        );
    }

    /// A pure deletion — the new side drops every line of the old — reports
    /// `added = 0, removed = N`, the mirror of the all-added created-file case.
    #[test]
    fn diff_stat_deleted_lines_are_all_removed() {
        let d = diff(Some("first\nsecond\n"), "", "old.rs");
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a fully-deleted file produces an all-removed hunk");
        assert_eq!(
            stat,
            DiffStat {
                added: 0,
                removed: 2
            }
        );
    }

    /// Identical sides produce no hunks → no editor and no stat. This pins the
    /// pure tally directly on empty hunks for clarity.
    #[test]
    fn diff_stat_unchanged_is_zero() {
        assert_eq!(diff_stat_from_hunks(&[]), DiffStat::default());
    }

    /// The cache key is per-(tool-call, diff index) so two files in one tool
    /// call get distinct editors.
    #[test]
    fn diff_editor_keys_are_per_file() {
        assert_eq!(diff_editor_key("call-1", 0), "call-1#0");
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-1", 1));
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-2", 0));
    }

    /// A tool-call item with a given status and diff list, for `is_active` and
    /// key-collection coverage.
    fn tool_call(id: &str, status: daruda_acp::ToolStatusView, diffs: usize) -> ToolCallItem {
        ToolCallItem {
            id: id.to_owned(),
            title: "t".to_owned(),
            kind: daruda_acp::ToolKindView::Edit,
            tool_name: None,
            status,
            diffs: (0..diffs)
                .map(|i| DiffView {
                    path: std::path::PathBuf::from(format!("f{i}.rs")),
                    old_text: None,
                    new_text: "x".to_owned(),
                })
                .collect(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
        }
    }

    /// A subagent launch (`Task` tool carrying `subagent_type`) keys to
    /// `FoldKey::Subagent` (collapsed by default); every other tool call keys to
    /// `FoldKey::Tool`.
    #[test]
    fn tool_fold_key_routes_subagent_launch_to_subagent_variant() {
        use daruda_acp::ToolStatusView::InProgress;

        let plain = tool_call("c1", InProgress, 0);
        assert_eq!(tool_fold_key(&plain), FoldKey::Tool("c1".to_owned()));

        let mut task = tool_call("task-1", InProgress, 0);
        task.raw_input = Some(serde_json::json!({ "subagent_type": "code-reviewer" }));
        assert_eq!(tool_fold_key(&task), FoldKey::Subagent("task-1".to_owned()));

        // An empty `subagent_type` is treated as absent (see `subagent_type`), so
        // it stays a plain tool rather than a subagent box.
        let mut empty = tool_call("task-2", InProgress, 0);
        empty.raw_input = Some(serde_json::json!({ "subagent_type": "" }));
        assert_eq!(tool_fold_key(&empty), FoldKey::Tool("task-2".to_owned()));
    }

    /// `is_active` is true while a block is streaming, or a tool call is live
    /// (`Pending` or `InProgress` — see [`ToolStatusView::is_live`]).
    #[test]
    fn is_active_matches_streaming_and_in_progress() {
        use daruda_acp::ToolStatusView::*;
        assert!(is_active(&ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: true,
            message_id: None,
        }));
        assert!(!is_active(&ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: false,
            message_id: None,
        }));
        assert!(is_active(&ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: true,
            message_id: None,
        }));
        assert!(!is_active(&ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: false,
            message_id: None,
        }));
        assert!(is_active(&ChatItem::ToolCall(tool_call(
            "c1", InProgress, 0
        ))));
        // A live `Pending` tool means an in-flight call in the active turn
        // (leftover `Pending` is settled to `Cancelled` at turn end), so it
        // reads as active — same as `InProgress`.
        assert!(is_active(&ChatItem::ToolCall(tool_call("c1", Pending, 0))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call(
            "c1", Completed, 0
        ))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call("c1", Failed, 0))));
        // Non-foldable / inactive items.
        assert!(!is_active(&ChatItem::UserText("u".to_owned())));
        assert!(!is_active(&ChatItem::Error("e".to_owned())));
    }

    /// `fold_active` — the single source both `rows::project` and
    /// `toggle_fold` read — resolves the `active` flag per fold key.
    #[test]
    fn fold_active_resolves_per_key() {
        use daruda_acp::ToolStatusView::{Completed, InProgress};
        // items: user, streaming assistant, live tool, settled tool, user, assistant
        let items = [
            ChatItem::UserText("q1".to_owned()),
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: true,
                message_id: None,
            },
            ChatItem::ToolCall(tool_call("t-live", InProgress, 0)),
            ChatItem::ToolCall(tool_call("t-done", Completed, 0)),
            ChatItem::UserText("q2".to_owned()),
            ChatItem::AssistantText {
                text: "b".to_owned(),
                streaming: false,
                message_id: None,
            },
        ];
        // Block keys follow the item's own active state.
        assert!(fold_active(&FoldKey::Assistant(1), &items));
        assert!(fold_active(&FoldKey::Tool("t-live".to_owned()), &items));
        assert!(!fold_active(&FoldKey::Tool("t-done".to_owned()), &items));
        // The tool group starting at `t-live` is active (a member runs); a group
        // is scanned as the consecutive run from its gid.
        assert!(fold_active(
            &FoldKey::ToolGroup("t-live".to_owned()),
            &items
        ));
        // Response at anchor 0: not the last turn, but its run streams → active.
        assert!(fold_active(&FoldKey::Response(0), &items));
        // Response at anchor 4: the last turn (no user message after) → active
        // even though its lone block is settled.
        assert!(fold_active(&FoldKey::Response(4), &items));
        // Policy-independent keys ignore `active`.
        assert!(!fold_active(&FoldKey::Diff("t-live#0".to_owned()), &items));
        assert!(!fold_active(
            &FoldKey::ToolRawInput("t-live".to_owned()),
            &items
        ));
        // Unknown ids / out-of-range indices are inactive, not a panic.
        assert!(!fold_active(&FoldKey::Assistant(99), &items));
        assert!(!fold_active(&FoldKey::Tool("nope".to_owned()), &items));
    }

    /// A single closed mermaid fence yields its verbatim body.
    #[test]
    fn mermaid_sources_extracts_a_closed_fence() {
        let text = "intro\n```mermaid\ngraph TD\nA-->B\n```\noutro";
        assert_eq!(mermaid_sources(text), vec!["graph TD\nA-->B".to_string()]);
    }

    /// Multiple closed fences are returned in document order.
    #[test]
    fn mermaid_sources_extracts_multiple_fences() {
        let text = "```mermaid\nA\n```\nmid\n```mermaid\nB\n```";
        assert_eq!(
            mermaid_sources(text),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    /// An unterminated trailing fence (still streaming) is skipped — only the
    /// already-closed fence before it is returned.
    #[test]
    fn mermaid_sources_skips_unterminated_trailing_fence() {
        let text = "```mermaid\nA\n```\n```mermaid\nstill streaming";
        assert_eq!(mermaid_sources(text), vec!["A".to_string()]);
        // A lone unterminated fence yields nothing.
        assert!(mermaid_sources("```mermaid\ngraph TD").is_empty());
    }

    /// Non-mermaid fences (other languages, or none) are ignored.
    #[test]
    fn mermaid_sources_ignores_non_mermaid_fences() {
        let text = "```rust\nfn main() {}\n```\n```\nplain\n```";
        assert!(mermaid_sources(text).is_empty());
    }

    /// The cache key is stable per (source, appearance) and distinct across
    /// sources *and* across the dark/light appearance — so a light/dark toggle
    /// re-rasterizes rather than reusing a stale-coloured diagram.
    #[test]
    fn mermaid_key_is_stable_and_distinct() {
        assert_eq!(
            mermaid_key("graph TD\nA-->B", true),
            mermaid_key("graph TD\nA-->B", true)
        );
        assert_ne!(
            mermaid_key("graph TD\nA-->B", true),
            mermaid_key("graph LR\nA-->B", true)
        );
        // Same source, different appearance → different key.
        assert_ne!(
            mermaid_key("graph TD\nA-->B", true),
            mermaid_key("graph TD\nA-->B", false)
        );
    }

    /// The visible foldable-key set the expand-all / collapse-all op builds.
    #[test]
    fn visible_fold_keys_cover_text_tools_and_diffs() {
        use daruda_acp::ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: false,
                message_id: None,
            },
            ChatItem::Thinking {
                text: "t".to_owned(),
                streaming: false,
                message_id: None,
            },
            ChatItem::ToolCall(tool_call("c1", Completed, 2)),
            ChatItem::Error("e".to_owned()),
        ];
        let keys = collect_foldable_keys(&items);
        // Structural header keys (the response — non-trivial run) first, then
        // the per-block keys. The single tool call is not a group (run < 2). The
        // assistant text (item 1) is the run's conclusion, which carries its own
        // fold toggle, so it contributes an `Assistant` key; thinking keeps its
        // own fold.
        assert_eq!(
            keys,
            vec![
                FoldKey::Response(0),
                FoldKey::Assistant(1),
                FoldKey::Thinking(2),
                FoldKey::Tool("c1".to_owned()),
                FoldKey::Diff("c1#0".to_owned()),
                FoldKey::Diff("c1#1".to_owned()),
            ]
        );
    }

    /// A trivial single-block reply has no response bar, so its assistant prose
    /// keeps the labeled, foldable block — its `Assistant` key is still
    /// collected. Guards the inline-vs-block split in `collect_foldable_keys`.
    #[test]
    fn trivial_reply_keeps_assistant_fold_key() {
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: false,
                message_id: None,
            },
        ];
        assert_eq!(collect_foldable_keys(&items), vec![FoldKey::Assistant(1)]);
    }

    /// A consecutive tool-call run (≥ 2) contributes a `ToolGroup` key on top
    /// of the per-tool keys, so expand/collapse-all reaches the group level.
    #[test]
    fn fold_keys_include_response_and_tool_group() {
        use daruda_acp::ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::ToolCall(tool_call("c1", Completed, 0)),
            ChatItem::ToolCall(tool_call("c2", Completed, 0)),
        ];
        let keys = collect_foldable_keys(&items);
        assert_eq!(
            keys,
            vec![
                FoldKey::Response(0),
                FoldKey::ToolGroup("c1".to_owned()),
                FoldKey::Tool("c1".to_owned()),
                FoldKey::Tool("c2".to_owned()),
            ]
        );
    }

    /// `renders_raw_input` is the single gate shared by the renderer and
    /// `collect_foldable_keys`; pin both the predicate and the resulting fold
    /// coverage so a future edit can't break renderer↔fold sync silently.
    #[test]
    fn raw_input_disclosure_gate_and_fold_coverage() {
        use daruda_acp::{ChatItem, ToolKindView, ToolStatusView};
        let generic = ToolCallItem {
            id: "c1".to_owned(),
            title: "Grep".to_owned(),
            kind: ToolKindView::Search,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: Some(serde_json::json!({ "pattern": "foo" })),
            parent_tool_id: None,
        };
        // Generic tool with args and no diffs → disclosure shown, and the fold
        // key is collected (expand/collapse-all reaches it).
        assert!(renders_raw_input(&generic));
        let keys = collect_foldable_keys(&[ChatItem::ToolCall(generic.clone())]);
        assert!(keys.contains(&FoldKey::ToolRawInput("c1".to_owned())));

        // Execute (terminal): the command is already the title → no disclosure,
        // and no fold key for it.
        let exec = ToolCallItem {
            kind: ToolKindView::Execute,
            ..generic.clone()
        };
        assert!(!renders_raw_input(&exec));
        let exec_keys = collect_foldable_keys(&[ChatItem::ToolCall(exec)]);
        assert!(
            !exec_keys
                .iter()
                .any(|k| matches!(k, FoldKey::ToolRawInput(_)))
        );

        // No args, or a diff present (an edit shows the diff) → nothing to show.
        assert!(!renders_raw_input(&ToolCallItem {
            raw_input: None,
            ..generic.clone()
        }));
        assert!(!renders_raw_input(&ToolCallItem {
            diffs: vec![DiffView {
                path: std::path::PathBuf::from("f.rs"),
                old_text: None,
                new_text: "x".to_owned(),
            }],
            ..generic
        }));
    }
}
