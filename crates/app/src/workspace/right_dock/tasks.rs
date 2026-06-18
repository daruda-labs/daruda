//! Tasks tab body — renders the lane-isolated Claude Code agent
//! task list pulled from `Workspace::tasks` (R-11 ~ R-18 + R-26 / R-27).
//!
//! Layout (top to bottom):
//! ```text
//! ┌─ Tasks ──────────────────────────────────── [All ▼] [+ New] ─┐
//! │  🔍 Search tasks…                                       ✕   │
//! │  ◉ fix-auth-bug        [Running ▾]   abc12345                │
//! │  ○ add-payment-page    [Backlog ▾]                           │
//! │  ✓ refactor-db-layer   [Done ▾]                              │
//! │  ✕ upgrade-deps        [Error ▾]                             │
//! │  ⊘ wip-experiment      [Cancelled ▾]                         │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Filter chip cycles `All → Backlog → Running → Done → All` on
//! click; `[+ New]` opens `CreateTaskModal` (R-13). Every state
//! transition + meta action (Edit / Delete / Open lane) lives
//! inside the per-row status-pill dropdown — see `status_pill.rs`
//! for the state → menu matrix (plan I-9 / R-26).
//!
//! Search input (R-27) substring-filters `title / prompt / notes /
//! branch_name` simultaneously with the state filter (AND). The
//! in-field `✕` clears the query.
//!
//! Static text comes from `surface::strings::TASK_*` /
//! `daruda_terminal::ux::strings::RIGHT_PANEL_TASK_*`; pixel + color
//! values from `daruda_terminal::ux::theme`. Direct `hsla(...)` /
//! `px(N)` literals are caught by `scripts/lint-inline-literals.sh`.

use crate::ui::Sizable as _;
use crate::ui::theme;
use chrono::{DateTime, Utc};
use daruda_claude::SessionStatus;
use daruda_store::tasks::{
    SessionEndReason, TASK_TOOL_USE_FAILURE_THRESHOLD, Task, TaskFilter, TaskState,
};
use daruda_terminal::ux::strings as ux_strings;
use gpui::{
    AnyElement, ClickEvent, Context, Hsla, IntoElement, MouseButton, SharedString, div, prelude::*,
    px,
};

use super::super::Workspace;
use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;
use super::status_pill;
use crate::surface::strings;
use crate::ui::{Badge, button, button_primary};

pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    // Pipeline: state filter → search filter → newest-first sort.
    // The search filter is a no-op when the query is blank, so empty
    // searches still go through `filter_by_state` unchanged.
    let query = snap.task_search_query.trim().to_ascii_lowercase();
    let mut visible: Vec<&Task> = snap
        .tasks
        .filter_by_state(snap.task_filter)
        .filter(|t| query.is_empty() || matches_task(t, &query))
        .collect();
    visible.sort_by_key(|t| std::cmp::Reverse(t.created_at));

    let header = header_row(snap);
    let search = search_row(snap, cx);

    let mut body = crate::workspace::right_dock::right_panel_body()
        .child(header)
        .child(search);

    if visible.is_empty() {
        // Distinguish "no tasks match this query" from "no tasks in
        // this filter bucket" — the former is search-recoverable
        // (the inline `✕` clears the query), the latter is a state
        // hint.
        let empty = if query.is_empty() {
            empty_state(snap.task_filter, cx)
        } else {
            search_empty_hint(snap.task_search_query.clone()).into_any_element()
        };
        body = body.child(empty);
    } else {
        let rows = div().flex().flex_col().children(
            visible
                .iter()
                .map(|t| task_row(t, snap, cx).into_any_element()),
        );
        body = body.child(rows);
    }

    body.into_any_element()
}

/// Lowercase substring match across the task's plain-text metadata —
/// `title`, `prompt`, `notes`, the derived `branch_name`, and every
/// `SubTask::title` (R-21 + R-27). Session-id prefixes are
/// intentionally excluded (UUID false positives — plan C-4).
///
/// Subtask matching covers both manual (`source_session_id == None`)
/// and auto-injected (`source_session_id == Some(...)`) rows so the
/// hook-fed TODO list is searchable just like the user-typed one.
fn matches_task(t: &Task, query_lower: &str) -> bool {
    t.title.to_ascii_lowercase().contains(query_lower)
        || t.prompt.to_ascii_lowercase().contains(query_lower)
        || t.notes.to_ascii_lowercase().contains(query_lower)
        || t.branch_name.to_ascii_lowercase().contains(query_lower)
        || t.subtasks
            .iter()
            .any(|s| s.title.to_ascii_lowercase().contains(query_lower))
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

/// Top row: filter chip on the left, `[+ New]` button on the right.
fn header_row(snap: &RightDockSnapshot) -> impl IntoElement {
    let ws = snap.workspace.clone();
    let new_ws = snap.workspace.clone();

    let filter_label = match snap.task_filter {
        TaskFilter::All => strings::task_filter_all(),
        TaskFilter::Backlog => strings::task_filter_backlog(),
        TaskFilter::Running => strings::task_filter_running(),
        TaskFilter::Done => strings::task_filter_done(),
    };

    let filter_chip = button("task-filter", filter_label).xsmall().on_click(
        move |_evt: &ClickEvent, _window, app| {
            if let Some(w) = ws.upgrade() {
                w.update(app, |this: &mut Workspace, cx| this.cycle_task_filter(cx));
            }
        },
    );

    let new_btn = button_primary("task-new", strings::task_new_button())
        .xsmall()
        .on_click(move |_evt: &ClickEvent, window, app| {
            if let Some(w) = new_ws.upgrade() {
                w.update(app, |this: &mut Workspace, cx| {
                    this.open_task_edit_pane(None, window, cx);
                });
            }
        });

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .py(px(theme::RIGHT_PANEL_HEADER_PAD_Y))
        .child(filter_chip)
        .child(new_btn)
}

// ---------------------------------------------------------------------------
// Search bar
// ---------------------------------------------------------------------------

/// Search input row. Mirrors `right_panel/skills/render.rs::search_row`
/// — wraps `RightDockSnapshot::task_search_input` in a relative container
/// so the in-field `✕` button can sit absolutely on the trailing edge.
/// The icon only renders while the query is non-empty.
fn search_row(snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    let has_query = !snap.task_search_query.trim().is_empty();
    let workspace = snap.workspace.clone();
    let chip_text = theme::TEXT_SECONDARY;
    let chip_hover_text = theme::TEXT_PRIMARY;
    div()
        .relative()
        .flex()
        .w_full()
        .child(crate::ui::input(&snap.task_search_input, cx, ()))
        .when(has_query, |row| {
            row.child(
                div()
                    .id("task-search-clear")
                    .absolute()
                    .right(px(theme::SKILL_ROW_PAD_X))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .px(px(theme::SKILL_BADGE_PAD_X))
                    .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                    .text_color(chip_text)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(chip_hover_text))
                    .child(strings::TASK_SEARCH_CLEAR_ICON)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        // Land on the overlay rather than the Input,
                        // so the input never observes this click. The
                        // stop is belt-and-braces.
                        cx.stop_propagation();
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws, cx| ws.clear_task_search(window, cx));
                        }
                    }),
            )
        })
}

/// Body shown when a non-empty search yields zero matches across
/// every state bucket. Text-only — the in-field `✕` already provides
/// one-click recovery so a second affordance here would be redundant.
fn search_empty_hint(query: String) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::TEXT_DISABLED)
        .child(SharedString::from(format!(
            "{}\"{}\".",
            strings::task_search_empty_prefix(),
            query.trim()
        )))
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

fn empty_state(filter: TaskFilter, cx: &gpui::App) -> AnyElement {
    let msg = match filter {
        TaskFilter::All => ux_strings::RIGHT_PANEL_TASK_EMPTY_ALL,
        TaskFilter::Backlog => ux_strings::RIGHT_PANEL_TASK_EMPTY_BACKLOG,
        TaskFilter::Running => ux_strings::RIGHT_PANEL_TASK_EMPTY_RUNNING,
        TaskFilter::Done => ux_strings::RIGHT_PANEL_TASK_EMPTY_DONE,
    };
    crate::ui::placeholder_text(msg)
        .py(px(theme::RIGHT_PANEL_PAD_Y))
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).text_subtle)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Per-task row
// ---------------------------------------------------------------------------

fn task_row(task: &Task, snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    let pill = status_pill::status_pill(task, snap, state_label(&task.state), cx);
    let session_badge = session_badge(task, snap);
    let duration = duration_cell(task, snap);
    let failures = failure_indicator(task, snap);
    let subtask_progress = subtask_progress_cell(task, cx);

    let row_hover_bg = theme::OVERLAY_HOVER;
    let ws = snap.workspace.clone();
    let id_for_open = task.id.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .py(px(theme::RIGHT_PANEL_ROW_PAD_Y))
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .hover(move |s| s.bg(row_hover_bg))
        .on_mouse_down(MouseButton::Left, move |_evt, window, app| {
            if let Some(w) = ws.upgrade() {
                let id = id_for_open.clone();
                w.update(app, |this: &mut Workspace, cx| {
                    this.open_task_edit_pane(Some(id), window, cx);
                });
            }
        })
        .child(indicator_cell(&task.state, snap.now, cx))
        .child(title_cell(&task.title))
        .children(duration)
        .children(session_badge)
        .children(failures)
        .child(subtask_progress)
        // Status pill sits at the trailing edge — its own mouse-down
        // closes the popup; we still need to stop propagation so the
        // row's expansion toggle doesn't fire when the user is just
        // dismissing the dropdown.
        .child(
            div()
                .flex_none()
                .on_mouse_down(MouseButton::Left, |_evt, _window, cx| {
                    cx.stop_propagation();
                })
                .child(pill),
        )
}

/// Subtask progress badge — `☑done/total`. Rendered for every row,
/// including `0/0`, so the column position stays stable across rows
/// regardless of whether the task has any subtasks yet, keeping the
/// column position stable across rows. Clicking the row body already
/// opens the TaskEdit pane, so the badge itself carries no click handler.
fn subtask_progress_cell(task: &Task, cx: &gpui::App) -> AnyElement {
    let (done, total) = task.subtask_progress();
    div()
        .flex_none()
        .text_color(theme::current(cx).text_muted)
        .child(SharedString::from(format!(
            "{}{}/{}",
            ux_strings::RIGHT_PANEL_SUBTASK_PROGRESS_GLYPH,
            done,
            total,
        )))
        .into_any_element()
}

/// Leading state-indicator cell. For `Running` rows we paint a
/// filled circle whose alpha oscillates between
/// [`theme::RIGHT_PANEL_TASK_PULSE_MIN_ALPHA`] and
/// [`theme::RIGHT_PANEL_TASK_PULSE_MAX_ALPHA`] over
/// [`theme::RIGHT_PANEL_TASK_PULSE_PERIOD_SEC`]; the workspace's
/// background live tick (`spawn_task_live_tick`) drives the redraws
/// that make the dot animate. Every other state stays static and
/// just renders its glyph in the matching state color.
fn indicator_cell(state: &TaskState, now: DateTime<Utc>, cx: &gpui::App) -> AnyElement {
    let cell = div()
        .w(px(theme::RIGHT_PANEL_TASK_INDICATOR_W))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    match state {
        TaskState::Running { .. } => {
            let alpha = pulse_alpha(now);
            let dot_color = Hsla {
                a: alpha,
                ..theme::current(cx).right_panel_task_running_color
            };
            cell.child(
                div()
                    .w(px(theme::RIGHT_PANEL_TASK_DOT_SIZE_PX))
                    .h(px(theme::RIGHT_PANEL_TASK_DOT_SIZE_PX))
                    .rounded(px(theme::RIGHT_PANEL_TASK_DOT_SIZE_PX / 2.0))
                    .bg(dot_color),
            )
            .into_any_element()
        }
        _ => {
            let (glyph, color) = state_indicator(state, cx);
            cell.text_color(color).child(glyph).into_any_element()
        }
    }
}

/// Triangular pulse wave bounded by
/// [`theme::RIGHT_PANEL_TASK_PULSE_MIN_ALPHA`] /
/// [`theme::RIGHT_PANEL_TASK_PULSE_MAX_ALPHA`] with period
/// [`theme::RIGHT_PANEL_TASK_PULSE_PERIOD_SEC`]. The phase is derived
/// from `now`'s sub-period offset, so every `Running` row across the
/// workspace breathes in lockstep — visually consistent and free of
/// per-row state.
///
/// The modulo runs in integer-millisecond space because the raw
/// `timestamp() as f32` path burns ~31 of f32's 24 mantissa bits on
/// the seconds-since-epoch magnitude alone, leaving sub-second
/// resolution below the noise floor — the pulse would visibly snap
/// to a single alpha and never animate.
fn pulse_alpha(now: DateTime<Utc>) -> f32 {
    let period_ms = (theme::RIGHT_PANEL_TASK_PULSE_PERIOD_SEC * 1000.0) as i64;
    let phase_ms = now.timestamp_millis().rem_euclid(period_ms);
    let phase = phase_ms as f32 / period_ms as f32;
    let min = theme::RIGHT_PANEL_TASK_PULSE_MIN_ALPHA;
    let max = theme::RIGHT_PANEL_TASK_PULSE_MAX_ALPHA;
    let span = max - min;
    if phase < 0.5 {
        max - span * (phase * 2.0)
    } else {
        min + span * ((phase - 0.5) * 2.0)
    }
}

fn title_cell(title: &str) -> impl IntoElement {
    // `flex_1` claims the row's slack, but a flex item's implicit
    // `min-width: auto` would keep it at its content width and shove
    // the trailing fixed cells (duration / badge / status pill) past
    // the dock edge. `min_w_0` resets that floor so the cell shrinks,
    // and `truncate` clips the overflow with an ellipsis on one line.
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_color(theme::TEXT_SECONDARY)
        .child(SharedString::from(title.to_string()))
}

fn state_indicator(state: &TaskState, cx: &gpui::App) -> (&'static str, Hsla) {
    let t = theme::current(cx);
    match state {
        TaskState::Backlog => (ux_strings::AGENT_TASK_QUEUED, theme::TEXT_DISABLED),
        TaskState::Running { .. } => (
            ux_strings::AGENT_TASK_RUNNING,
            t.right_panel_task_running_color,
        ),
        TaskState::Done { .. } => (ux_strings::AGENT_TASK_DONE, theme::TEXT_TERTIARY),
        TaskState::Error { .. } => (ux_strings::AGENT_TASK_ERROR, theme::ERROR),
        TaskState::Cancelled { .. } => (ux_strings::AGENT_TASK_CANCELLED, theme::TEXT_DISABLED),
    }
}

fn state_label(state: &TaskState) -> SharedString {
    match state {
        TaskState::Backlog => SharedString::from(ux_strings::RIGHT_PANEL_TASK_BACKLOG_LABEL),
        TaskState::Running { .. } => SharedString::from(ux_strings::RIGHT_PANEL_TASK_RUNNING_LABEL),
        TaskState::Done { end_reason, .. } => SharedString::from(format!(
            "{} ({})",
            ux_strings::RIGHT_PANEL_TASK_DONE_LABEL_PREFIX,
            done_flavour_label(*end_reason),
        )),
        TaskState::Error { message, .. } => {
            let truncated = if message.chars().count() > theme::RIGHT_PANEL_TASK_ERROR_TRUNCATE {
                let mut s: String = message
                    .chars()
                    .take(theme::RIGHT_PANEL_TASK_ERROR_TRUNCATE)
                    .collect();
                s.push('…');
                s
            } else {
                message.clone()
            };
            SharedString::from(format!(
                "{}: {}",
                ux_strings::RIGHT_PANEL_TASK_ERROR_LABEL_PREFIX,
                truncated,
            ))
        }
        TaskState::Cancelled { .. } => {
            SharedString::from(ux_strings::RIGHT_PANEL_TASK_CANCELLED_LABEL)
        }
    }
}

fn done_flavour_label(reason: SessionEndReason) -> String {
    match reason {
        SessionEndReason::Stop => strings::task_done_flavour_stop(),
        SessionEndReason::PromptInputExit => strings::task_done_flavour_prompt_input_exit(),
        SessionEndReason::Logout => strings::task_done_flavour_logout(),
        SessionEndReason::Other => strings::task_done_flavour_other(),
        // `Error` lives on the `Error` state, not the `Done` state —
        // any path that reaches here means the row was migrated from
        // an older daruda. Fall back to the generic "Other" flavour.
        SessionEndReason::Error => strings::task_done_flavour_other(),
    }
}

// ---------------------------------------------------------------------------
// Duration cell — `now - created_at` or `finished_at - created_at`
// ---------------------------------------------------------------------------

/// Inline span showing the task's run duration — live for `Running`
/// rows (re-rendered by `spawn_task_live_tick`) and frozen at
/// `finished_at - created_at` for terminal states. `Backlog` rows
/// have no meaningful duration so the cell drops out entirely. Sub-
/// second rounding follows `surface::strings::format_duration_compact`,
/// which already powers the long-running notification body so a
/// single helper keeps both surfaces in sync.
fn duration_cell(task: &Task, snap: &RightDockSnapshot) -> Option<AnyElement> {
    let end = match &task.state {
        TaskState::Backlog => return None,
        TaskState::Running { .. } => snap.now,
        TaskState::Done { .. } | TaskState::Error { .. } | TaskState::Cancelled { .. } => {
            task.finished_at.unwrap_or(snap.now)
        }
    };
    let elapsed = (end - task.created_at).to_std().ok()?;
    if elapsed.as_secs() == 0 && !matches!(task.state, TaskState::Running { .. }) {
        // A near-instant terminal transition (cancel-before-start) is
        // not worth a "0s" badge — drop the cell entirely so the row
        // stays clean.
        return None;
    }
    let text = crate::surface::strings::format_duration_compact(elapsed);
    Some(
        div()
            .flex_none()
            .text_size(px(theme::RIGHT_PANEL_TASK_DURATION_FONT_SIZE))
            .text_color(theme::TEXT_TERTIARY)
            .child(SharedString::from(text))
            .into_any_element(),
    )
}

// ---------------------------------------------------------------------------
// Session badge — leading `session_id` slice + per-session status glyph
// ---------------------------------------------------------------------------

/// Renders the 8-char session-id badge (Badge widget) followed by an
/// optional `⟳ / ● / ⚠` glyph that mirrors the matching session's
/// `ClaudeStatusStore` entry. The glyph drops out when the session
/// isn't known to the store (hook + jsonl both silent) so a fresh
/// task that hasn't yet emitted any event reads as "no session
/// activity yet" rather than "idle".
fn session_badge(task: &Task, snap: &RightDockSnapshot) -> Option<AnyElement> {
    let sid = task.session_ids.first()?;
    let take = ux_strings::RIGHT_PANEL_TASK_SESSION_BADGE_LEN.min(sid.len());
    let prefix: String = sid.chars().take(take).collect();

    let glyph = snap
        .claude_status_per_session
        .get(sid)
        .map(|status| session_status_glyph(*status));

    let mut row = div()
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_TASK_SESSION_GAP))
        .child(Badge::new(prefix));
    if let Some((glyph_text, glyph_color)) = glyph {
        row = row.child(div().flex_none().text_color(glyph_color).child(glyph_text));
    }
    Some(row.into_any_element())
}

/// Maps the abstract `SessionStatus` enum to the trailing-glyph
/// `(text, color)` pair surfaced next to the session-id badge.
fn session_status_glyph(status: SessionStatus) -> (&'static str, Hsla) {
    match status {
        SessionStatus::Working | SessionStatus::ExecutingTool => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_WORKING,
            theme::TEXT_TERTIARY,
        ),
        SessionStatus::NeedsAttention => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_NEEDS_ATTENTION,
            theme::WARNING,
        ),
        // `Connecting` and `Idle` both read as "quiet" — a session
        // that hasn't produced any output yet looks the same as one
        // that finished its turn and is waiting for the next prompt.
        SessionStatus::Idle | SessionStatus::Connecting => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_IDLE,
            theme::TEXT_TERTIARY,
        ),
    }
}

// ---------------------------------------------------------------------------
// Failure indicator — `failures N/M` once a session crosses the
// soft display threshold (R-11 Phase 2 counter, R-23 visualization)
// ---------------------------------------------------------------------------

/// Renders a small `failures 3/5` chip when any session attached to
/// the task has accumulated at least
/// [`ux_strings::RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD`]
/// tool-use failures. The denominator is the hard cap
/// [`TASK_TOOL_USE_FAILURE_THRESHOLD`] beyond which daruda auto-
/// escalates the row to `Error` — surfacing the ratio gives the user
/// a visual trend before the state flips.
///
/// Aggregates across every `task.session_ids` rather than the leading
/// one, because Resume / Retry can stack multiple sessions onto a
/// single Running task and the escalation check itself fires on the
/// per-session counter — taking the max keeps the row consistent
/// with the state machine that may flip it.
///
/// Drops out for terminal states (the row has already settled) and
/// when no session has crossed the soft threshold yet.
fn failure_indicator(task: &Task, snap: &RightDockSnapshot) -> Option<AnyElement> {
    if !matches!(task.state, TaskState::Running { .. }) {
        return None;
    }
    let count = task
        .session_ids
        .iter()
        .filter_map(|sid| snap.tool_use_failure_counts.get(sid).copied())
        .max()?;
    if count < ux_strings::RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD {
        return None;
    }
    let text = format!(
        "{}{}/{}",
        ux_strings::RIGHT_PANEL_TASK_FAILURES_LABEL,
        count,
        TASK_TOOL_USE_FAILURE_THRESHOLD,
    );
    Some(
        div()
            .flex_none()
            .text_size(px(theme::RIGHT_PANEL_TASK_FAILURE_FONT_SIZE))
            .text_color(theme::WARNING)
            .child(SharedString::from(text))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{matches_task, pulse_alpha};
    use crate::ui::theme;
    use chrono::{TimeZone, Utc};
    use daruda_store::tasks::{SubTask, Task};

    /// Use a space-free title so `sanitize_branch_name` accepts it
    /// verbatim — that keeps `branch_name` predictable for the
    /// substring assertion below. Titles with spaces drop into the
    /// `task-<ulid>` fallback, which would make the branch_name hit
    /// case fragile.
    fn fresh_task() -> Task {
        Task::new("fix-bug".into(), "prompt body".into(), None)
    }

    /// Title / prompt / notes / branch_name remain matched after the
    /// R-21 subtask addition — guard against accidental regression.
    #[test]
    fn matches_task_still_matches_core_metadata_fields() {
        let mut t = fresh_task();
        t.notes = "Investigate the auth flow".into();
        assert!(matches_task(&t, "fix"), "title hit");
        assert!(matches_task(&t, "prompt"), "prompt hit");
        assert!(matches_task(&t, "auth"), "notes hit");
        assert!(matches_task(&t, "fix-bug"), "branch_name hit");
        assert!(!matches_task(&t, "zzz"), "no match → false");
    }

    /// R-27 follow-up: subtask titles join the search corpus.
    #[test]
    fn matches_task_finds_subtask_titles() {
        let mut t = fresh_task();
        t.subtasks.push(SubTask::new("Inspect session.rs".into()));
        t.subtasks.push(SubTask::new("Add refresh logic".into()));
        assert!(matches_task(&t, "session"), "manual subtask title hit");
        assert!(matches_task(&t, "refresh"), "second subtask title hit");
    }

    /// Auto-injected subtasks (R-22 TodoWrite merge) carry the same
    /// title text as manual ones, so they must be searchable too.
    #[test]
    fn matches_task_finds_auto_subtasks() {
        let mut t = fresh_task();
        let mut auto = SubTask::new("Write integration tests".into());
        auto.source_session_id = Some("sess_abc".into());
        t.subtasks.push(auto);
        assert!(matches_task(&t, "integration"));
    }

    /// Query is already lowercased by `render`; the function assumes
    /// that and uses `to_ascii_lowercase` on the haystack. Mixed-case
    /// subtask titles still match a lowercase query.
    #[test]
    fn matches_task_subtask_search_is_case_insensitive() {
        let mut t = fresh_task();
        t.subtasks.push(SubTask::new("CamelCase Step".into()));
        assert!(matches_task(&t, "camelcase"));
        assert!(matches_task(&t, "step"));
    }

    /// Fence-post: at phase 0 the dot reads as fully lit.
    #[test]
    fn pulse_alpha_starts_at_max() {
        // Epoch is an exact multiple of every integer period (0 mod N
        // == 0), so phase = 0 there. Picks epoch over "1 × period"
        // because the latter only works if the period happens to be
        // a whole number of seconds.
        let now = Utc.timestamp_millis_opt(0).unwrap();
        let a = pulse_alpha(now);
        assert!(
            (a - theme::RIGHT_PANEL_TASK_PULSE_MAX_ALPHA).abs() < 1e-3,
            "expected ≈ max at phase 0, got {a}",
        );
    }

    /// Midpoint of the period must bottom out at the minimum alpha.
    #[test]
    fn pulse_alpha_midpoint_hits_min() {
        let period_ms = (theme::RIGHT_PANEL_TASK_PULSE_PERIOD_SEC * 1000.0) as i64;
        // `(0 + period/2) ms` past epoch → phase = 0.5.
        let now = Utc.timestamp_millis_opt(period_ms / 2).unwrap();
        let a = pulse_alpha(now);
        assert!(
            (a - theme::RIGHT_PANEL_TASK_PULSE_MIN_ALPHA).abs() < 1e-3,
            "expected ≈ min at phase 0.5, got {a}",
        );
    }

    /// Stays inside the documented bounds at present-day timestamps —
    /// the regression this guards against is the f32-precision bug
    /// where `timestamp() as f32` snapped sub-second offsets to zero,
    /// freezing alpha at `max` for the entire wall-clock period.
    #[test]
    fn pulse_alpha_stays_in_range_on_recent_timestamps() {
        // `2026-05-11T12:00:00Z` — far past the f32 precision wall.
        let base = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        let mut seen_distinct = std::collections::HashSet::new();
        for offset_ms in (0..1500).step_by(50) {
            let now = base + chrono::Duration::milliseconds(offset_ms);
            let a = pulse_alpha(now);
            let range = (theme::RIGHT_PANEL_TASK_PULSE_MIN_ALPHA - 1e-6)
                ..=(theme::RIGHT_PANEL_TASK_PULSE_MAX_ALPHA + 1e-6);
            assert!(
                range.contains(&a),
                "alpha {a} escaped [{}, {}] at offset_ms={offset_ms}",
                theme::RIGHT_PANEL_TASK_PULSE_MIN_ALPHA,
                theme::RIGHT_PANEL_TASK_PULSE_MAX_ALPHA,
            );
            seen_distinct.insert((a * 1000.0) as i32);
        }
        // If the f32 precision bug returned, every offset would collapse
        // onto a single alpha — assert we see plenty of distinct values
        // across the 1.5 s period.
        assert!(
            seen_distinct.len() > 10,
            "pulse looks frozen — only {} distinct alpha values across the period",
            seen_distinct.len()
        );
    }
}
