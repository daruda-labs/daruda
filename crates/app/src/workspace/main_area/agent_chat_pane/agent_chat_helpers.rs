//! GPUI-free (and view-context) helpers for the agent chat pane, shared by the
//! renderer, the row projection, the reconcilers, and the `Workspace` ops.
//!
//! Split out of [`agent_chat_ops`](super::agent_chat_ops) — whose `impl
//! Workspace` methods (notification, pane construction, mode/config; the
//! connect lifecycle and prompt queue live in their own sibling files) need
//! `Workspace` state — because these are a distinct responsibility cluster
//! (pure model/derivation helpers + the diff-model builders) with their own
//! test fixtures. Everything here is either GPUI-free or takes a
//! `Context<AgentChatView>` (diff-editor creation); none of it needs
//! `Workspace` state.

use daruda_acp::DiffView;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{AnyWindowHandle, AppContext as _, Context, Entity};

use super::fold::{FoldKey, FoldState};
use super::rows::{RowKind, SUBAGENT_NEST_DEPTH_CAP, project};
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

/// Content fingerprint of a diff's editor-relevant fields (`old_text` +
/// `new_text`). `reconcile_diff_editors` stores this alongside each built
/// editor and compares it against the diff's *current* fingerprint on every
/// pass: unchanged means the cached editor still matches; changed means a
/// `ToolCallUpdate` replaced the diff since the editor was built (e.g. a
/// streaming write growing from a partial snapshot to the final content) and
/// the editor is stale and must be rebuilt. Not cryptographic — a same-key
/// collision would only skip a rebuild it should have done, an acceptable
/// cost for a `DefaultHasher` over ordinary diff sizes.
pub(in crate::workspace) fn diff_source_fingerprint(diff: &DiffView) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    diff.old_text.hash(&mut hasher);
    diff.new_text.hash(&mut hasher);
    hasher.finish()
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
/// themed to the host appearance (`mermaid_host_theme_profile`): without it a cached
/// raster would keep its old colours after a light/dark toggle. `DefaultHasher`
/// is process-stable, which is all the in-memory cache needs.
pub(in crate::workspace) fn mermaid_key(source: &str, dark: bool) -> u64 {
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
pub(in crate::workspace) fn tool_image_key(data: &str) -> u64 {
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
/// Resolves through the shared registry-backed resolver, so it never names a
/// language the highlighter cannot load.
pub(in crate::workspace) fn diff_editor_language(diff: &DiffView) -> gpui::SharedString {
    crate::ui::highlighter::language_for_extension(diff.path.extension_str())
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
///
/// The gutter shows line numbers only for a full-file diff (`old_text` is
/// `None` — a file *creation* / whole-file write, where the numbers are the
/// real file lines). An `Edit` sends only the replaced snippet as
/// `old_text`/`new_text`, so its diff would be numbered from 1 regardless of
/// where in the file the edit lands (the ACP `Diff` carries no line offset) —
/// those numbers would mislead, so they are hidden.
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
    let show_line_numbers = diff.old_text.is_none();
    Some((
        build_diff_editor_model(&rows, colors, show_line_numbers),
        stat,
    ))
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
            // `language` selects CodeEditor mode (gutter, indent guides), not
            // the colours: `set_highlight_override` below short-circuits the
            // tree-sitter path entirely, because a synthetic +/- diff buffer
            // is not valid source for any grammar.
            let mut state = gpui_component::input::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false)
                .code_editor(language)
                .rows(rows);
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

/// True when `items` holds conversation content that a session teardown would
/// destroy. [`ChatItem::Error`](daruda_acp::ChatItem::Error) does not count: it
/// is a session-failure notice, not the user's transcript, so a pane whose only
/// content is "session limit reached" — the usual reason to reach for another
/// account — still counts as empty and can reconnect in place.
///
/// Read by [`switch_kind`](crate::workspace::account_ops::switch_kind) to
/// decide whether an account switch may reuse the pane.
pub(in crate::workspace) fn has_conversation(items: &[daruda_acp::ChatItem]) -> bool {
    use daruda_acp::ChatItem;
    items.iter().any(|it| !matches!(it, ChatItem::Error(_)))
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

/// The `items` index of the tool call that owns a `RenderRow` for `tool_id`:
/// itself if it has no live parent in `items`, else walked up through
/// `parent_tool_id` to the ancestor `rows::project` actually gives a row
/// (nested subagent children render inside their parent's card and earn no
/// row of their own — see `is_nested_child`). Depth-bounded like
/// `subagent_subtree_live`, for the same malformed/cyclic-id safety.
fn top_level_tool_item_index(items: &[daruda_acp::ChatItem], tool_id: &str) -> Option<usize> {
    use daruda_acp::ChatItem;
    let mut current = tool_id;
    let mut owned: String;
    for _ in 0..SUBAGENT_NEST_DEPTH_CAP {
        let (ix, parent) = items.iter().enumerate().find_map(|(ix, item)| match item {
            ChatItem::ToolCall(tc) if tc.id == current => Some((ix, tc.parent_tool_id.clone())),
            _ => None,
        })?;
        match parent {
            Some(pid)
                if items
                    .iter()
                    .any(|it| matches!(it, ChatItem::ToolCall(p) if p.id == pid)) =>
            {
                owned = pid;
                current = &owned;
            }
            _ => return Some(ix),
        }
    }
    None
}

/// The `self.rows` index a fold toggle on `key` must remeasure so its stale
/// cached row height doesn't clip/overlap neighboring rows. `Assistant` /
/// `Thinking` are their own row, keyed directly by item index. `Tool` /
/// `Subagent` / `ToolRawInput` / `Diff` (keyed by tool-call id, `Diff` as
/// `"{tool_id}#{diff_index}"`) only ever change *their owning row's* rendered
/// height in place — never a `RenderRow::hidden` flip anywhere — so
/// `rebuild_rows`'s hidden-range diff can't see them and falls back to
/// remeasuring the tail, leaving a stale height on whichever row actually
/// changed (the diff-collapse clipping bug). `Response` / `ToolGroup`
/// collapse instead hides their child rows, which the hidden-range diff
/// already catches correctly, so they resolve to `None` here.
pub(in crate::workspace) fn fold_key_item_index(
    key: &FoldKey,
    items: &[daruda_acp::ChatItem],
) -> Option<usize> {
    match key {
        FoldKey::Assistant(ix) | FoldKey::Thinking(ix) => Some(*ix),
        FoldKey::Tool(id) | FoldKey::Subagent(id) | FoldKey::ToolRawInput(id) => {
            top_level_tool_item_index(items, id)
        }
        FoldKey::Diff(diff_key) => {
            let tool_id = diff_key.split('#').next().unwrap_or(diff_key.as_str());
            top_level_tool_item_index(items, tool_id)
        }
        FoldKey::Response(_) | FoldKey::ToolGroup(_) => None,
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
mod tests;
