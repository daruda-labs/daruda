//! Project-scoped task list. Header controls own scope and status;
//! per-row status menus retain task lifecycle actions.

mod browser;
mod controls;
mod grouping;
mod list;
mod rows;
#[cfg(feature = "screenshot")]
mod screenshot;
mod state;

pub(in crate::workspace) use browser::{TaskBrowser, TaskBrowserSnapshot};
pub(in crate::workspace) use controls::header;
pub(in crate::workspace) use grouping::{TaskGroupKey, TaskGrouping, TaskGroups};

use crate::ui::theme;
use chrono::{DateTime, Utc};
use daruda_agent::SessionStatus;
use daruda_store::tasks::{SessionEndReason, TASK_TOOL_USE_FAILURE_THRESHOLD, Task, TaskState};
use daruda_terminal::ux::strings as ux_strings;
use gpui::{AnyElement, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::super::layout::RightDockSnapshot;
use super::status_pill;
use crate::surface::strings;
use crate::ui::Badge;

pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &gpui::App) -> AnyElement {
    let list = list::TaskList::project(
        &snap.tasks,
        snap.task_browser.state.scope,
        snap.task_projects.active,
        snap.task_browser.state.filter,
        &snap.task_browser.query,
    );
    let mut body = crate::workspace::right_dock::right_panel_body()
        .child(controls::status_tabs(snap, &list))
        .child(
            crate::ui::list_page::toolbar()
                .child(div().flex_1().min_w_0().child(search_row(snap, cx)))
                .child(controls::grouping_picker(snap)),
        )
        .child(controls::results(snap, &list, cx));

    if list.visible.is_empty() {
        body = body.child(controls::empty_state(snap, list.scoped_count, cx));
    } else {
        body = body.child(rows::table(snap, &list, cx));
    }

    body.into_any_element()
}

/// Search task metadata and subtasks, excluding attached session IDs.
pub(super) fn matches_task(t: &Task, query_lower: &str) -> bool {
    t.title.to_ascii_lowercase().contains(query_lower)
        || t.id.to_ascii_lowercase().contains(query_lower)
        || t.prompt.to_ascii_lowercase().contains(query_lower)
        || t.notes.to_ascii_lowercase().contains(query_lower)
        || t.branch_name.to_ascii_lowercase().contains(query_lower)
        || t.subtasks
            .iter()
            .any(|s| s.title.to_ascii_lowercase().contains(query_lower))
}

fn search_row(snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    let has_query = !snap.task_browser.query.trim().is_empty();
    let workspace = snap.workspace.clone();
    crate::ui::list_page::search(
        "task-search-clear",
        strings::common::search_clear().into(),
        &snap.task_browser.search,
        has_query,
        move |window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.clear_task_search(window, cx));
            }
        },
        cx,
    )
}

// ---------------------------------------------------------------------------
// Per-task row
// ---------------------------------------------------------------------------

/// Progress is metadata, omitted by the row when no subtasks exist.
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

/// Leading state-indicator cell. `Running` rows paint a filled circle
/// whose alpha pulses (see `pulse_alpha`), driven by the workspace's
/// `spawn_task_live_tick` redraws; every other state renders a static
/// glyph in its state color.
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

/// Triangular pulse wave bounded by the pulse min/max alpha over the
/// pulse period. Phase derives from `now`'s sub-period offset, so every
/// `Running` row breathes in lockstep with no per-row state.
///
/// The modulo runs in integer-millisecond space: `timestamp() as f32`
/// spends f32's mantissa on the seconds-since-epoch magnitude, dropping
/// sub-second resolution below the noise floor so the pulse would freeze
/// at a single alpha.
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

fn state_indicator(state: &TaskState, cx: &gpui::App) -> (&'static str, Hsla) {
    let t = theme::current(cx);
    match state {
        TaskState::Backlog => (ux_strings::AGENT_TASK_QUEUED, t.text_subtle),
        TaskState::Running { .. } => (
            ux_strings::AGENT_TASK_RUNNING,
            t.right_panel_task_running_color,
        ),
        TaskState::Done { .. } => (ux_strings::AGENT_TASK_DONE, t.text_muted),
        TaskState::Error { .. } => (ux_strings::AGENT_TASK_ERROR, theme::ERROR),
        TaskState::Cancelled { .. } => (ux_strings::AGENT_TASK_CANCELLED, t.text_subtle),
    }
}

fn state_label(state: &TaskState) -> SharedString {
    match state {
        TaskState::Backlog => SharedString::from(strings::terminal::task_backlog()),
        TaskState::Running { .. } => SharedString::from(strings::terminal::task_running()),
        TaskState::Done { end_reason, .. } => SharedString::from(format!(
            "{} ({})",
            strings::terminal::task_done_prefix(),
            done_flavour_label(*end_reason),
        )),
        TaskState::Error { message, .. } => SharedString::from(format!(
            "{}: {}",
            strings::terminal::task_error_prefix(),
            message,
        )),
        TaskState::Cancelled { .. } => SharedString::from(strings::terminal::task_cancelled()),
    }
}

fn done_flavour_label(reason: SessionEndReason) -> String {
    match reason {
        SessionEndReason::Stop => strings::task::done_flavour_stop(),
        SessionEndReason::PromptInputExit => strings::task::done_flavour_prompt_input_exit(),
        SessionEndReason::Logout => strings::task::done_flavour_logout(),
        SessionEndReason::Other => strings::task::done_flavour_other(),
        // `Error` belongs to the `Error` state, not `Done`; reaching
        // here means a migrated row — fall back to "Other".
        SessionEndReason::Error => strings::task::done_flavour_other(),
    }
}

// ---------------------------------------------------------------------------
// Duration cell — `now - created_at` or `finished_at - created_at`
// ---------------------------------------------------------------------------

/// Inline span showing the task's run duration — live for `Running`
/// rows (re-rendered by `spawn_task_live_tick`), frozen at
/// `finished_at - created_at` for terminal states, dropped for
/// `Backlog`. Formatting reuses `format_duration_compact`.
fn duration_cell(
    task: &Task,
    snap: &RightDockSnapshot,
    t: &crate::ui::theme::DarudaTheme,
) -> Option<AnyElement> {
    let end = match &task.state {
        TaskState::Backlog => return None,
        TaskState::Running { .. } => *snap.now,
        TaskState::Done { .. } | TaskState::Error { .. } | TaskState::Cancelled { .. } => {
            task.finished_at.unwrap_or(*snap.now)
        }
    };
    let elapsed = (end - task.created_at).to_std().ok()?;
    if elapsed.as_secs() == 0 && !matches!(task.state, TaskState::Running { .. }) {
        // A near-instant terminal transition (cancel-before-start) is
        // not worth a "0s" badge — drop the cell entirely so the row
        // stays clean.
        return None;
    }
    let text = crate::surface::strings::notification::format_duration_compact(elapsed);
    Some(
        div()
            .flex_none()
            .text_size(px(theme::RIGHT_PANEL_TASK_DURATION_FONT_SIZE))
            .text_color(t.text_muted)
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
fn session_badge(task: &Task, snap: &RightDockSnapshot, cx: &gpui::App) -> Option<AnyElement> {
    let sid = task.session_ids.first()?;
    let take = ux_strings::RIGHT_PANEL_TASK_SESSION_BADGE_LEN.min(sid.len());
    let prefix: String = sid.chars().take(take).collect();

    let glyph = snap
        .claude_status_per_session
        .get(sid)
        .map(|status| session_status_glyph(*status, cx));

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
fn session_status_glyph(status: SessionStatus, cx: &gpui::App) -> (&'static str, Hsla) {
    match status {
        SessionStatus::Working | SessionStatus::ExecutingTool => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_WORKING,
            theme::current(cx).text_muted,
        ),
        SessionStatus::NeedsAttention => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_NEEDS_ATTENTION,
            theme::WARNING,
        ),
        SessionStatus::Failed => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_FAILED,
            theme::current(cx).status_failed_dark,
        ),
        // `Connecting` and `Idle` both read as "quiet" — a session
        // that hasn't produced any output yet looks the same as one
        // that finished its turn and is waiting for the next prompt.
        SessionStatus::Idle | SessionStatus::Connecting => (
            ux_strings::RIGHT_PANEL_TASK_SESSION_STATUS_IDLE,
            theme::current(cx).text_muted,
        ),
    }
}

// ---------------------------------------------------------------------------
// Failure indicator — `failures N/M` once a session crosses the
// soft display threshold
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
        strings::terminal::task_failures_prefix(),
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

    /// The branch is set apart from every other field, so a branch hit
    /// cannot pass by matching the title instead.
    fn fresh_task() -> Task {
        let mut task = Task::new(
            daruda_store::project::ProjectUuid::default(),
            "fix-bug".into(),
            "prompt body".into(),
            None,
        );
        task.branch_name = "feat-login".into();
        task
    }

    /// Title / prompt / notes / branch_name stay matched even once
    /// subtasks are present — guard against accidental regression.
    #[test]
    fn matches_task_still_matches_core_metadata_fields() {
        let mut t = fresh_task();
        t.notes = "Investigate the auth flow".into();
        assert!(matches_task(&t, "fix"), "title hit");
        assert!(matches_task(&t, "prompt"), "prompt hit");
        assert!(matches_task(&t, "auth"), "notes hit");
        assert!(matches_task(&t, "feat-login"), "branch_name hit");
        assert!(!matches_task(&t, "zzz"), "no match → false");
    }

    /// Subtask titles join the search corpus.
    #[test]
    fn matches_task_finds_subtask_titles() {
        let mut t = fresh_task();
        t.subtasks.push(SubTask::new("Inspect session.rs".into()));
        t.subtasks.push(SubTask::new("Add refresh logic".into()));
        assert!(matches_task(&t, "session"), "manual subtask title hit");
        assert!(matches_task(&t, "refresh"), "second subtask title hit");
    }

    /// Auto-injected subtasks (from the TodoWrite hook merge) carry the
    /// same title text as manual ones, so they must be searchable too.
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
