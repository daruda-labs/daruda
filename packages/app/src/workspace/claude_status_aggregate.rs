//! Agent status pane aggregation.
//!
//! `aggregate_over_panes` is the single source of truth for the left-dock
//! leading indicator's aggregate, shared with the render snapshot
//! (`render::snapshots`'s `agent_status_per_lane` /
//! `agent_per_session_per_lane`). The debug-only probe / logger sit on top
//! of it so the on-disk NDJSON log can be diffed against the rendered
//! indicator when verifying status transitions.

use std::collections::HashMap;

use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_host::lane_status;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_agent::SessionStatus;
use daruda_agent::pty_tracker::PtyBinding;

// `Path` and `Source` are only named by the debug-only transition logger
// (and `Source` also by the always-compiled test fixtures), so gate the
// imports to those profiles to avoid an unused-import warning in release
// non-test builds.
#[cfg(any(debug_assertions, test))]
use daruda_agent::hooks::status_file::Source;
#[cfg(debug_assertions)]
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
#[cfg(debug_assertions)]
use daruda_store::observability::log_writer::LogWriter;
#[cfg(debug_assertions)]
use daruda_store::observability::system_info::redact_home;
#[cfg(debug_assertions)]
use std::path::Path;

/// Per-lane leading indicator — the lane's collapsed session status.
pub(in crate::workspace) type LaneStatusMap =
    HashMap<daruda_store::project::LaneRef, SessionStatus>;

/// Per-lane `(session_id, status)` sub-rows in pane order — only lanes
/// with ≥ 2 sessions appear.
pub(in crate::workspace) type LaneSessionsMap =
    HashMap<daruda_store::project::LaneRef, Vec<(String, SessionStatus)>>;

/// Aggregate Claude status over panes, attributing each pane's session to
/// the lane that owns the pane (not the session's cwd).
///
/// `pane_lane` lists every pane in layout order paired with its owning
/// lane. For each pane: a missing binding means the pane has no Claude
/// session; a `session_id` absent from `store` is omitted entirely (it is
/// not surfaced as `Connecting`); a session already collected for the lane
/// (one session bound to two panes, e.g. `claude --resume` of a running
/// session) counts once at its first pane's position.
///
/// `acp_statuses` carries agent chat pane statuses (pane_id + status),
/// contributed alongside PTY-bound sessions so the lane indicator
/// reflects ACP sessions as well. Each entry uses a synthetic session id
/// (`"acp:{pane_id}"`) to stay distinguishable in the sub-row badges.
///
/// Returns:
/// - per-lane leading indicator: [`SessionStatus::aggregate`] over the
///   lane's sessions (only lanes with ≥ 1 session appear). Equal-priority
///   ties (`Working` vs `ExecutingTool`) resolve to the later session in
///   pane order.
/// - per-lane sub-rows: `(session_id, status)` in pane order, only for
///   lanes with ≥ 2 sessions (single-session lanes are fully described by
///   the leading indicator). Ids are cloned only for these lanes.
pub(in crate::workspace) fn aggregate_over_panes(
    pane_lane: &[(PaneId, daruda_store::project::LaneRef)],
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_agent::ClaudeStatusStore,
    acp_statuses: &[(PaneId, SessionStatus)],
) -> (LaneStatusMap, LaneSessionsMap) {
    let mut per_lane: HashMap<daruda_store::project::LaneRef, Vec<(String, SessionStatus)>> =
        HashMap::new();

    // Build a quick lookup for ACP pane statuses.
    let acp_map: HashMap<PaneId, SessionStatus> = acp_statuses.iter().copied().collect();

    for (pane_id, lane_ref) in pane_lane {
        let Some((session_id, status)) = pane_session(*pane_id, bindings, store, &acp_map) else {
            continue;
        };
        let sessions = per_lane.entry(*lane_ref).or_default();
        // One session, one entry — the same session bound to a second
        // pane (e.g. `claude --resume` of a running session) keeps its
        // first-pane position and must not inflate the sub-row count.
        if !sessions.iter().any(|(sid, _)| *sid == session_id) {
            sessions.push((session_id, status));
        }
    }

    let per_lane_status = per_lane
        .iter()
        .filter_map(|(lane_ref, sessions)| {
            SessionStatus::aggregate(sessions.iter().map(|(_, s)| *s)).map(|s| (*lane_ref, s))
        })
        .collect();

    let per_lane_sessions = per_lane
        .into_iter()
        .filter(|(_, sessions)| sessions.len() >= 2)
        .collect();

    (per_lane_status, per_lane_sessions)
}

/// One pane's session, `(session id, status)`: its PTY-bound Claude session
/// first, else its agent chat. The one definition the lane and tab
/// indicators share.
fn pane_session(
    pane_id: PaneId,
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_agent::ClaudeStatusStore,
    acp: &HashMap<PaneId, SessionStatus>,
) -> Option<(String, SessionStatus)> {
    if let Some(binding) = bindings.get(&pane_id)
        && let Some(file) = store.get(&binding.session_id)
    {
        return Some((binding.session_id.clone(), file.status));
    }
    acp.get(&pane_id)
        .map(|&status| (format!("acp:{pane_id}"), status))
}

/// Every distinct session among `pane_ids`, `(session id, status)`, in pane
/// order. A session bound to two panes counts once, as in
/// [`aggregate_over_panes`], but window-wide rather than per lane.
pub(in crate::workspace) fn distinct_sessions(
    pane_ids: impl IntoIterator<Item = PaneId>,
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_agent::ClaudeStatusStore,
    acp_statuses: &[(PaneId, SessionStatus)],
) -> Vec<(String, SessionStatus)> {
    let acp: HashMap<PaneId, SessionStatus> = acp_statuses.iter().copied().collect();
    let mut sessions: Vec<(String, SessionStatus)> = Vec::new();
    for pane_id in pane_ids {
        if let Some((id, status)) = pane_session(pane_id, bindings, store, &acp)
            && !sessions.iter().any(|(seen, _)| *seen == id)
        {
            sessions.push((id, status));
        }
    }
    sessions
}

/// The status a tab shows: the most urgent session among its panes.
pub(in crate::workspace) fn tab_session_status(
    pane_ids: &[PaneId],
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_agent::ClaudeStatusStore,
    acp_statuses: &[(PaneId, SessionStatus)],
) -> Option<SessionStatus> {
    let acp: HashMap<PaneId, SessionStatus> = acp_statuses.iter().copied().collect();
    SessionStatus::aggregate(
        pane_ids
            .iter()
            .filter_map(|id| pane_session(*id, bindings, store, &acp).map(|(_, s)| s)),
    )
}

/// `true` when any pane's bound session (PTY or ACP) is in a status whose
/// badge moves (see [`SessionStatus::animates`]). The status-pulse gate reads this instead of the per-lane
/// aggregate: the aggregate's max-priority collapse would hide a
/// `Connecting` session (priority 0) behind an `Idle` sibling (priority 1)
/// even though its sub-row badge animates. Short-circuits without
/// allocating, so it is cheap on the pulse tick.
pub(in crate::workspace) fn any_pane_session_animating(
    pane_lane: &[(PaneId, daruda_store::project::LaneRef)],
    bindings: &HashMap<PaneId, PtyBinding>,
    store: &daruda_agent::ClaudeStatusStore,
    acp_statuses: &[(PaneId, SessionStatus)],
    icon_theme: &daruda_config::StatusIconTheme,
    catalog: &crate::status_icon_themes::Catalog,
) -> bool {
    let animates = |status: SessionStatus| {
        catalog.animates(
            icon_theme,
            crate::ui::agent_status_badge::state_for_status(status),
        )
    };
    pane_lane.iter().any(|(pane_id, _)| {
        // PTY-bound session.
        let pty_animating = bindings
            .get(pane_id)
            .and_then(|binding| store.get(&binding.session_id))
            .is_some_and(|file| animates(file.status));
        if pty_animating {
            return true;
        }
        // ACP (agent chat) session.
        acp_statuses
            .iter()
            .any(|(pid, status)| pid == pane_id && animates(*status))
    })
}

impl Workspace {
    /// Every pane in the workspace paired with its owning lane, drawn
    /// from the single `runtimes` map. Layout order holds within a lane;
    /// cross-lane order is unspecified (HashMap iteration), so consumers
    /// must group per lane. Feeds [`aggregate_over_panes`] so status
    /// attribution follows pane membership rather than session cwd —
    /// and covers parked lanes, so a backgrounded lane's agent-chat
    /// indicator keeps updating after a lane switch.
    /// Not orchestrator-filtered: while its tab is visible its pane maps to
    /// the host worktree here. Safe only because every consumer joins this
    /// against a filtered set (`agent_chat_statuses`) or one it is never in
    /// (`pty_claude_bindings`) — a new consumer must filter or join likewise.
    pub(in crate::workspace) fn pane_lane_index(
        &self,
    ) -> Vec<(PaneId, daruda_store::project::LaneRef)> {
        let mut index = Vec::new();
        for (lane_ref, runtime) in &self.main_area.runtimes {
            for tab in &runtime.tabs {
                for pane_id in tab.layout.pane_ids() {
                    index.push((pane_id, *lane_ref));
                }
            }
        }
        index
    }

    /// Every agent session in this window with its status — the same sources
    /// as the lane indicator, so keep-awake and the badges never disagree.
    pub(crate) fn session_statuses(&self, cx: &gpui::App) -> Vec<(String, SessionStatus)> {
        distinct_sessions(
            self.pane_lane_index()
                .into_iter()
                .map(|(pane_id, _)| pane_id),
            &self.claude_sessions.pty_bindings,
            &self.claude_sessions.status,
            &self.agent_chat_statuses(cx),
        )
    }

    /// Dock-only tool detail; busy/completion consumers keep the activity status.
    pub(in crate::workspace) fn agent_chat_badge_statuses(
        &self,
        cx: &gpui::App,
    ) -> Vec<(PaneId, SessionStatus)> {
        if crate::ui::agent_status_badge::icon_theme(cx) == daruda_config::StatusIconTheme::Dots {
            return self.agent_chat_statuses(cx);
        }
        self.lane_agent_chats()
            .filter_map(|(id, view)| {
                let view = view.read(cx);
                let status = lane_status::session_status(view)?;
                let status = if status == SessionStatus::Working && view.has_running_tool() {
                    SessionStatus::ExecutingTool
                } else {
                    status
                };
                Some((id, status))
            })
            .collect()
    }

    /// `(pane_id, status)` for every AgentChat (ACP) pane across **all**
    /// lanes — not just the active one — so a parked lane's agent session
    /// still feeds the left-dock leading indicator and the status-pulse
    /// gate after a lane switch. Single source for both
    /// [`Workspace::prepare_left_dock_snapshot`] and
    /// [`Workspace::has_animating_claude_status`]; keeping it in one place
    /// means the "scan all lanes" rule can't drift between the two.
    ///
    /// Reading each view registers a GPUI entity dependency on the caller,
    /// so a view's `cx.notify()` dirties whatever observed it (the render
    /// snapshot) — the reactive half of the indicator update.
    pub(in crate::workspace) fn agent_chat_statuses(
        &self,
        cx: &gpui::App,
    ) -> Vec<(PaneId, SessionStatus)> {
        self.lane_agent_chats()
            .filter_map(|(id, view)| {
                let status = lane_status::session_status(view.read(cx))?;
                Some((id, status))
            })
            .collect()
    }
}

/// Pre-mutation snapshot of one lane's status, captured so a transition
/// can be detected after the store mutates.
#[cfg(debug_assertions)]
pub(in crate::workspace) struct LaneStatusProbe {
    /// The mutated session's own stored status (if present).
    session: Option<SessionStatus>,
    /// The lane's displayed leading-indicator aggregate.
    aggregate: Option<SessionStatus>,
    /// The pane the session is bound to, if any — captured here so the
    /// transition logger does not re-resolve it.
    pane_id: Option<PaneId>,
    /// The lane that owns the session's pane, if resolvable.
    lane_ref: Option<daruda_store::project::LaneRef>,
}

#[cfg(debug_assertions)]
impl Workspace {
    /// Resolve a session to the pane it is bound to and that pane's lane.
    ///
    /// Scan the bindings for the pane bound to `session_id`, then locate
    /// that pane in `index` (the caller's [`Workspace::pane_lane_index`],
    /// passed in so a single probe builds it once). The reverse scan is
    /// `O(panes)` and debug-only / low-frequency, so it is not indexed.
    /// Either component is `None` when no live binding or pane mapping
    /// exists for the session.
    fn session_pane_lane(
        &self,
        session_id: &str,
        index: &[(PaneId, daruda_store::project::LaneRef)],
    ) -> (Option<PaneId>, Option<daruda_store::project::LaneRef>) {
        let pane_id = self
            .claude_sessions
            .pty_bindings
            .iter()
            .find(|(_, binding)| binding.session_id == session_id)
            .map(|(pane_id, _)| *pane_id);
        let lane_ref =
            pane_id.and_then(|pane_id| index.iter().find(|(p, _)| *p == pane_id).map(|(_, l)| *l));
        (pane_id, lane_ref)
    }

    /// Capture the session's stored status and its lane aggregate.
    ///
    /// The lane is resolved from the session's pane via
    /// [`Workspace::session_pane_lane`].
    pub(in crate::workspace) fn probe_lane_status(&self, session_id: &str) -> LaneStatusProbe {
        let index = self.pane_lane_index();
        let (pane_id, lane_ref) = self.session_pane_lane(session_id, &index);
        let aggregate = lane_ref.and_then(|lane_ref| {
            // ACP (agent chat) statuses are not included in the probe
            // because this path only triggers for PTY-bound session
            // transitions (hook / JSONL watcher).
            let (per_lane, _) = aggregate_over_panes(
                &index,
                &self.claude_sessions.pty_bindings,
                &self.claude_sessions.status,
                &[],
            );
            // `None` when no pane in the lane has a store entry yet
            // (e.g. a brand-new session before its first hook write) —
            // logged as "(none)", matching the rendered indicator.
            per_lane.get(&lane_ref).copied()
        });
        LaneStatusProbe {
            session: self
                .claude_sessions
                .status
                .get(session_id)
                .map(|f| f.status),
            aggregate,
            pane_id,
            lane_ref,
        }
    }

    /// Emit one Info NDJSON line per level (session / lane indicator)
    /// that changed since `before`. Log-only — never a toast — so the
    /// frequent tool-call transitions don't flood the UI; verify against
    /// the rendered indicator via the on-disk log.
    pub(in crate::workspace) fn log_lane_status_change(
        &self,
        before: LaneStatusProbe,
        session_id: &str,
        cwd: &Path,
        last_event: &str,
        source: Source,
    ) {
        let after = self.probe_lane_status(session_id);
        // Skip sessions not bound to any pane in this workspace (claude
        // running outside daruda, or another instance's session). They
        // never drive a lane indicator here, so logging their transitions
        // only floods the file with "(unbound)" noise. A `Some` lane_ref
        // whose project/lane was later removed still logs below (a real
        // ownership change worth recording).
        if after.lane_ref.is_none() {
            return;
        }
        // Resolve human-readable project + lane labels for the log context
        // from the pane/lane the probe already resolved. Falls back to
        // "(unbound)" when the referenced project / lane no longer exists.
        // A real project name paired with an "(unbound)" lane means the
        // project exists but the lane was removed — intentional, not a
        // resolution failure.
        let project = after
            .lane_ref
            .and_then(|lr| self.project_for(lr.project))
            .map(|p| p.name.as_str())
            .unwrap_or("(unbound)");
        let lane = after
            .lane_ref
            .and_then(|lr| self.lane_for(lr))
            .map(|l| l.display_name())
            .unwrap_or_else(|| "(unbound)".to_string());
        if before.session != after.session {
            LogWriter::log(session_transition_report(
                before.session,
                after.session,
                session_id,
                cwd,
                last_event,
                source,
                after.pane_id,
                project,
                &lane,
            ));
        }
        if before.aggregate != after.aggregate {
            LogWriter::log(aggregate_transition_report(
                before.aggregate,
                after.aggregate,
                cwd,
                project,
                &lane,
            ));
        }
    }
}

#[cfg(debug_assertions)]
fn status_label(status: Option<SessionStatus>) -> &'static str {
    match status {
        Some(SessionStatus::Working) => "Working",
        Some(SessionStatus::ExecutingTool) => "ExecutingTool",
        Some(SessionStatus::NeedsAttention) => "NeedsAttention",
        Some(SessionStatus::Idle) => "Idle",
        Some(SessionStatus::Connecting) => "Connecting",
        Some(SessionStatus::Failed) => "Failed",
        None => "(none)",
    }
}

#[cfg(debug_assertions)]
fn source_label(source: Source) -> &'static str {
    match source {
        Source::Hook => "hook",
        Source::Jsonl => "jsonl",
    }
}

#[cfg(debug_assertions)]
#[allow(clippy::too_many_arguments)]
fn session_transition_report(
    prev: Option<SessionStatus>,
    next: Option<SessionStatus>,
    session_id: &str,
    cwd: &Path,
    last_event: &str,
    source: Source,
    pane: Option<PaneId>,
    project: &str,
    lane: &str,
) -> ErrorReport {
    let mut report = ErrorReport::new("Claude session status changed")
        .severity(ErrorSeverity::Info)
        .message(format!("{} -> {}", status_label(prev), status_label(next)))
        .at(file!(), line!())
        .with_context("session", session_id)
        .with_context("cwd", redact_home(cwd))
        .with_context("event", last_event)
        .with_context("source", source_label(source))
        .with_context("project", project)
        .with_context("lane", lane);
    // Pane id is only meaningful while a live binding exists; omit the
    // key entirely otherwise rather than logging a placeholder.
    if let Some(pane) = pane {
        report = report.with_context("pane", pane.to_string());
    }
    report.build()
}

// `cwd` here is the triggering session's own working directory, not the
// owning lane's path — the two can differ now that attribution is by pane.
// The `project` / `lane` keys carry the lane this indicator belongs to.
#[cfg(debug_assertions)]
fn aggregate_transition_report(
    prev: Option<SessionStatus>,
    next: Option<SessionStatus>,
    cwd: &Path,
    project: &str,
    lane: &str,
) -> ErrorReport {
    ErrorReport::new("Claude lane indicator changed")
        .severity(ErrorSeverity::Info)
        .message(format!("{} -> {}", status_label(prev), status_label(next)))
        .at(file!(), line!())
        .with_context("cwd", redact_home(cwd))
        .with_context("project", project)
        .with_context("lane", lane)
        .build()
}

#[cfg(test)]
mod tests;
