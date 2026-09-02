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
use gpui::{AppContext as _, Context, Entity};

use super::fold::{FoldContext, FoldKey, FoldState};
use super::rows::{LiveSubagentUnits, RowKind, effective_tool_status, project};
use super::tool_hierarchy::ToolHierarchy;
use super::view::AgentChatView;
use super::window_access::WindowAccess;
use crate::path_ext::PathExt as _;
use crate::transcript::fold_mode::TurnPosition;
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
/// tool, which carries `subagent_type` and/or `prompt` (see
/// [`daruda_acp::ToolCallItem::is_subagent_launch`]) — gets [`FoldKey::Subagent`]
/// so its card defaults collapsed: the subagent's flattened inner tool calls
/// nest inside the card and would otherwise fill the transcript while it runs.
/// Every other tool call gets the standard [`FoldKey::Tool`]. Single source
/// shared by the renderer (top-level and nested cards) and
/// [`collect_foldable_keys`] so a subagent's card, its click toggle, and
/// expand/collapse-all all agree on one key — a mismatch would make the toggle
/// write an override the card never reads.
pub(in crate::workspace) fn tool_fold_key(tc: &daruda_acp::ToolCallItem) -> FoldKey {
    if tc.is_subagent_launch() {
        FoldKey::Subagent(tc.id.clone())
    } else {
        FoldKey::Tool(tc.id.clone())
    }
}

/// Fold keys controlled by expand-all and collapse-all. Tail and filter reveals
/// are excluded because their chips own those states.
pub(in crate::workspace) fn collect_foldable_keys(items: &[daruda_acp::ChatItem]) -> Vec<FoldKey> {
    let mut keys: Vec<FoldKey> = Vec::new();
    // Defaults preserve the structural header set while avoiding pane state.
    let rows = project(
        items,
        &FoldState::default(),
        false,
        &super::rows::LiveSubagentUnits::default(),
        super::rows::tail::TailWindow::All,
        &crate::transcript::display_filter::DisplayFilter::default(),
    );
    // Inline assistant prose has no independent fold control.
    let inline_assistant: std::collections::HashSet<usize> = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::AgentItem(ix) if row.indent > 0 => Some(ix),
            _ => None,
        })
        .collect();
    for row in &rows {
        match &row.kind {
            RowKind::ResponseHeader { run_start, .. } => keys.push(FoldKey::Response(*run_start)),
            RowKind::ToolGroupHeader { gid, .. } => keys.push(FoldKey::ToolGroup(gid.clone())),
            RowKind::ThinkingGroupHeader { first_ix, .. } => {
                keys.push(FoldKey::ThinkingGroup(*first_ix))
            }
            RowKind::TailMore { .. } => {}
            RowKind::User(_)
            | RowKind::AgentItem(_)
            | RowKind::ConclusionItem(_)
            | RowKind::WorkingIndicator => {}
        }
    }
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
                // has args) so expand/collapse-all covers the disclosure. The
                // "Instructions" section (`renders_subagent_instructions`) has
                // no fold key of its own — it is always visible once shown, not
                // a disclosure — so it contributes nothing here.
                if renders_raw_input(tc) {
                    keys.push(FoldKey::ToolRawInput(tc.id.clone()));
                }
            }
            daruda_acp::ChatItem::UserText(_)
            | daruda_acp::ChatItem::Permission(_)
            | daruda_acp::ChatItem::Failure(_) => {}
        }
    }
    keys
}

/// Whether a tool card renders its raw-input (JSON args) disclosure: a generic
/// tool (not a terminal `Execute`, whose command is already the title) that
/// carries args and has no diffs (an edit shows the diff instead). A subagent
/// launch is excluded too — it gets its own, purpose-built
/// [`renders_subagent_instructions`] section instead of the generic JSON dump.
/// Single source shared by the renderer and [`collect_foldable_keys`], so the
/// fold coverage matches what is actually on screen.
pub(in crate::workspace) fn renders_raw_input(tc: &daruda_acp::ToolCallItem) -> bool {
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
pub(in crate::workspace) fn renders_subagent_instructions(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.subagent_prompt().is_some()
}

/// Whether a tool card's generic `Output` section must stay hidden because it
/// would just repeat the "Instructions" section above it: a subagent launch
/// that is still live echoes its own `prompt` back as `output` (the adapter's
/// content-block content), and that channel is only overwritten with the
/// subagent's actual result once the call settles (`fold_output`'s replace
/// semantics — see [`daruda_acp::ToolCallItem::subagent_prompt`]'s doc). Once
/// settled, `output` holds the distinct result summary and renders normally.
pub(in crate::workspace) fn suppresses_live_subagent_output(tc: &daruda_acp::ToolCallItem) -> bool {
    tc.is_subagent_launch() && tc.status.is_live()
}

/// Cache key for a tool call's `di`-th diff editor: one editor per file. Shared
/// with the renderer so the embed lookup matches the insert key.
pub(in crate::workspace) fn diff_editor_key(tool_call_id: &str, di: usize) -> String {
    format!("{tool_call_id}#{di}")
}

/// Fingerprint of **every input** `build_diff_view_model` consumes for one
/// diff: its content, the `path` the editor's language comes from, and `theme`
/// from [`diff_theme_fingerprint`].
///
/// `reconcile_diff_editors` stores this alongside each built editor and compares
/// it against the current one on every pass, so one rule — "the fingerprint
/// moved" — covers every reason a built editor can be stale: a `ToolCallUpdate`
/// replaced the diff (a streaming write growing from a partial snapshot), the
/// path changed the language, or the theme swapped under it. Covering the theme
/// is what an earlier content-only fingerprint missed, and a diff embed cannot
/// recover from that on its own — see [`diff_theme_fingerprint`].
///
/// Not cryptographic — a same-key collision would only skip a rebuild it should
/// have done, an acceptable cost for a `DefaultHasher` over ordinary diff sizes.
pub(in crate::workspace) fn diff_build_fingerprint(diff: &DiffView, theme: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    diff.path.hash(&mut hasher);
    diff.old_text.hash(&mut hasher);
    diff.new_text.hash(&mut hasher);
    theme.hash(&mut hasher);
    hasher.finish()
}

/// Fingerprint of the theme inputs every diff in one pass shares — the syntax
/// theme id, its light/dark variant, and the snapshotted palette.
///
/// A diff embed's colours are baked into `set_highlight_override` at build time,
/// and that override is what the editor reads *instead of* resolving
/// `cx.theme().highlight_theme` on every paint (`gpui_component`'s
/// `input/element.rs`) — deliberately, since an interleaved +/- buffer is not
/// valid source for any grammar. The consequence is that a built diff embed
/// cannot follow a theme swap the way an output embed does; only a rebuild moves
/// it. Folding this into [`diff_build_fingerprint`] is what makes the swap look
/// like any other staleness to the reconciler.
pub(in crate::workspace) fn diff_theme_fingerprint(
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    syntax_theme.hash(&mut hasher);
    is_light.hash(&mut hasher);
    // `Hsla` is `f32`-based and so not `Hash`; hash the bit patterns. Equal
    // colours always share a bit pattern here — these are copied straight out of
    // the theme, never arithmetic results that could differ by a NaN or a signed
    // zero.
    for c in colors.channels() {
        c.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// The agent run starting at `start`: every item up to (not including) the next
/// user message, or the end of the conversation. Single source for "where does
/// this response end" — [`crate::workspace::main_area::agent_chat_pane::rows`]
/// walks turns with it and the response bar recomputes its own run from the
/// anchor with it (the projected header carries only the anchor index). An empty
/// range when `start` is past the end, so a prompt with no reply yet is not a
/// special case.
pub(in crate::workspace) fn agent_run(
    items: &[daruda_acp::ChatItem],
    start: usize,
) -> std::ops::Range<usize> {
    let end = items
        .iter()
        .skip(start)
        .position(|item| matches!(item, daruda_acp::ChatItem::UserText(_)))
        .map_or(items.len(), |offset| start + offset);
    start.min(end)..end
}

/// The outcome a fold header's rollup glyph summarizes over the run it stands
/// for. Single source shared by the response bar, the tool-group bar, and a
/// top-level assistant block, so the three can never disagree on treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Rollup {
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
    pub(in crate::workspace) fn of_kept_run(
        items: &[daruda_acp::ChatItem],
        range: std::ops::Range<usize>,
        live_units: &LiveSubagentUnits,
        keep: impl Fn(&daruda_acp::ChatItem) -> bool,
    ) -> Self {
        Self::of_items(
            range.filter_map(|k| items.get(k)),
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
                // neither an outcome nor progress.
                ChatItem::UserText(_) | ChatItem::Permission(_) => {}
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
    let kept: String = head.chars().take(FALLBACK_TITLE_HEAD).collect();
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
pub(in crate::workspace) fn chat_item_mermaid_texts(item: &daruda_acp::ChatItem) -> Vec<&str> {
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
        daruda_acp::ChatItem::Permission(_) | daruda_acp::ChatItem::Failure(_) => Vec::new(),
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
    use crate::workspace::main_area::file_view_pane::highlighter::{LanguageHint, highlight_hunks};
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
    highlight_hunks(
        &mut hunks,
        LanguageHint::Extension(ext),
        syntax_theme,
        is_light,
    );
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

/// Create + configure a read-only diff editor entity against the live window
/// `access` resolves. Mirrors the File viewer's editor construction
/// (`multi_line` + `soft_wrap(false)` + `code_editor`) and the diff-config it
/// applies (`set_disabled(true)` for read-only + decorations + injected
/// highlight spans). Returns `None` if the owning window is gone.
///
/// The by-handle half of [`WindowAccess`] uses the view's stored handle rather
/// than `WindowRegistry::handle_for_workspace(cx.entity_id())` because after the
/// pane became its own entity `cx.entity_id()` is the view, not the Workspace,
/// so the registry would no longer resolve the window.
pub(in crate::workspace) fn create_diff_editor(
    cx: &mut Context<AgentChatView>,
    access: &mut WindowAccess<'_>,
    pane_id: PaneId,
    language: &str,
    model: DiffEditorModel,
) -> Option<Entity<gpui_component::input::InputState>> {
    let language = language.to_owned();
    match access.with(cx, move |window, cx_w| {
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
        ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Failure(_) => false,
    }
}

/// True when `items` holds conversation content that a session teardown would
/// destroy. [`ChatItem::Failure`](daruda_acp::ChatItem::Failure) does not count:
/// it is a session-failure notice, not the user's transcript, so a pane whose
/// only content is "session limit reached" — the usual reason to reach for
/// another account — still counts as empty and can reconnect in place.
///
/// Read by [`switch_kind`](crate::workspace::account_ops::switch_kind) to
/// decide whether an account switch may reuse the pane.
pub(in crate::workspace) fn has_conversation(items: &[daruda_acp::ChatItem]) -> bool {
    use daruda_acp::ChatItem;
    items.iter().any(|it| !matches!(it, ChatItem::Failure(_)))
}

/// Cached start index of the newest turn.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct TurnBoundary(usize);

impl TurnBoundary {
    pub(in crate::workspace) fn of(items: &[daruda_acp::ChatItem]) -> Self {
        use daruda_acp::ChatItem;
        Self(
            items
                .iter()
                .rposition(|item| matches!(item, ChatItem::UserText(_)))
                .unwrap_or(0),
        )
    }

    pub(in crate::workspace) fn at(self, ix: usize) -> TurnPosition {
        if ix >= self.0 {
            TurnPosition::Last
        } else {
            TurnPosition::Past
        }
    }
}

/// Resolve a key's activity and turn position from the conversation.
pub(in crate::workspace) fn fold_context(
    key: &FoldKey,
    items: &[daruda_acp::ChatItem],
) -> FoldContext {
    match fold_key_index(key, items) {
        Some(ix) => fold_context_at(key, ix, items, TurnBoundary::of(items)),
        None => FoldContext::new(TurnPosition::Past, false),
    }
}

/// Resolve fold context when the item index and turn boundary are already known.
pub(in crate::workspace) fn fold_context_at(
    key: &FoldKey,
    ix: usize,
    items: &[daruda_acp::ChatItem],
    boundary: TurnBoundary,
) -> FoldContext {
    let context = FoldContext::new(boundary.at(ix), fold_active_at(key, ix, items));
    match (key, items.get(ix)) {
        (FoldKey::Tool(_), Some(daruda_acp::ChatItem::ToolCall(tc))) => {
            context.with_tool_category(crate::transcript::tool_category::classify_tool(tc))
        }
        _ => context,
    }
}

fn fold_key_index(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> Option<usize> {
    match key {
        FoldKey::Assistant(ix)
        | FoldKey::Thinking(ix)
        | FoldKey::ThinkingGroup(ix)
        | FoldKey::Response(ix)
        | FoldKey::Tail(ix)
        | FoldKey::Filtered(ix) => Some(*ix),
        FoldKey::Tool(id)
        | FoldKey::Subagent(id)
        | FoldKey::ToolRawInput(id)
        | FoldKey::ToolGroup(id) => tool_item_index(items, id),
        FoldKey::Diff(diff_key) => {
            let tool_id = diff_key.split('#').next().unwrap_or(diff_key.as_str());
            tool_item_index(items, tool_id)
        }
    }
}

#[cfg(test)]
pub(in crate::workspace) fn fold_turn(
    key: &FoldKey,
    items: &[daruda_acp::ChatItem],
) -> TurnPosition {
    match fold_key_index(key, items) {
        Some(ix) => TurnBoundary::of(items).at(ix),
        None => TurnPosition::Past,
    }
}

fn tool_item_index(items: &[daruda_acp::ChatItem], tool_id: &str) -> Option<usize> {
    use daruda_acp::ChatItem;
    items
        .iter()
        .position(|item| matches!(item, ChatItem::ToolCall(tc) if tc.id == tool_id))
}

#[cfg(test)]
pub(in crate::workspace) fn fold_active(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> bool {
    match fold_key_index(key, items) {
        Some(ix) => fold_active_at(key, ix, items),
        None => false,
    }
}

fn fold_active_at(key: &FoldKey, ix: usize, items: &[daruda_acp::ChatItem]) -> bool {
    use daruda_acp::ChatItem;
    match key {
        FoldKey::Assistant(_) | FoldKey::Thinking(_) | FoldKey::Tool(_) => {
            items.get(ix).map(is_active).unwrap_or(false)
        }
        // Keyed by the response's own first item (`rows::project` passes
        // `run.start`), so the scan starts at `ix` — not after it. An `ix` that
        // is itself a `UserText` yields an empty run, which reads inactive.
        FoldKey::Response(_) => {
            let end = items
                .iter()
                .skip(ix)
                .position(|it| matches!(it, ChatItem::UserText(_)))
                .map(|off| ix + off)
                .unwrap_or(items.len());
            items
                .get(ix..end)
                .is_some_and(|run| run.iter().any(is_active))
        }
        FoldKey::ToolGroup(_) => items.get(ix..).is_some_and(|rest| {
            rest.iter()
                .take_while(|item| matches!(item, ChatItem::ToolCall(_)))
                .any(is_active)
        }),
        FoldKey::ThinkingGroup(_) => items.get(ix..).is_some_and(|rest| {
            rest.iter()
                .take_while(|item| matches!(item, ChatItem::Thinking { .. }))
                .any(is_active)
        }),
        FoldKey::Diff(_)
        | FoldKey::ToolRawInput(_)
        | FoldKey::Subagent(_)
        | FoldKey::Tail(_)
        | FoldKey::Filtered(_) => false,
    }
}

/// The `self.rows` index a fold toggle on `key` must remeasure so its stale
/// cached row height doesn't clip/overlap neighboring rows. `Assistant` /
/// `Thinking` are their own row, keyed directly by item index. `Tool` /
/// `Subagent` / `ToolRawInput` / `Diff` (keyed by tool-call id, `Diff` as
/// `"{tool_id}#{diff_index}"`) only ever change *their owning row's* rendered
/// height in place — never a `RenderRow::hidden` flip anywhere — so
/// `rebuild_rows`'s hidden-range diff can't see them and falls back to
/// remeasuring the tail, leaving a stale height on whichever row actually
/// changed (the diff-collapse clipping bug). `Response` / `ToolGroup` /
/// `ThinkingGroup` collapse instead hides their child rows, which the
/// hidden-range diff already catches correctly, so they resolve to `None` here.
pub(in crate::workspace) fn fold_key_item_index(
    key: &FoldKey,
    items: &[daruda_acp::ChatItem],
) -> Option<usize> {
    // Built only for the keys that ask a hierarchy question; nested subagent
    // children render inside their parent's card and earn no row of their own.
    let owner = |id: &str| ToolHierarchy::build(items).owning_row_index(id);
    match key {
        FoldKey::Assistant(ix) | FoldKey::Thinking(ix) => Some(*ix),
        FoldKey::Tool(id) | FoldKey::Subagent(id) | FoldKey::ToolRawInput(id) => owner(id),
        FoldKey::Diff(diff_key) => owner(diff_key.split('#').next().unwrap_or(diff_key.as_str())),
        FoldKey::Response(_)
        | FoldKey::ToolGroup(_)
        | FoldKey::ThinkingGroup(_)
        | FoldKey::Tail(_)
        | FoldKey::Filtered(_) => None,
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
