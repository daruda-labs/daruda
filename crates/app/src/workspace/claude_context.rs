//! Claude Code integration state owned by [`super::Workspace`].
//!
//! Groups the 18 fields that together drive the right-panel Usage tab,
//! the per-worktree session-status indicator, the JSONL fallback
//! watcher, the PTY tracker, and the plan-limits / service-status
//! polls. Workspace owns this struct directly (not as an `Entity`) so
//! the existing access patterns (`self.claude.usage`, etc.) compile
//! without subscription / re-render plumbing changes — the goal at
//! this stage is **field grouping**, not actor isolation. A future
//! refactor can promote `ClaudeContext` to its own GPUI Entity once
//! the call graph is mapped.
//!
//! The `_*` prefixed fields are RAII guards: dropping the struct
//! cancels the matching pump tasks / shuts down the JSONL fallback
//! watcher.

use std::collections::HashMap;
use std::sync::mpsc;

use gpui::{Entity, Subscription, Task};

use crate::hooks::pty_tracker::{PtyBinding, PtyTracker};
use crate::ui::select::SelectState;
use crate::workspace::layout::PaneId;

pub(in crate::workspace) struct ClaudeContext {
    /// Token-usage accumulator for the right dock's Usage tab. Folded
    /// per-session by `apply_claude_jsonl_event` whenever the JSONL
    /// watcher surfaces new assistant entries with a `usage` block.
    /// Not persisted — the watcher's first fire after each launch
    /// re-emits the entire session history, providing automatic cold
    /// restore.
    pub(in crate::workspace) usage: daruda_claude::usage::UsageState,

    /// Per-million-token pricing applied when rendering the Usage
    /// tab's cost columns. Derived from `[usage.pricing]` at construct
    /// time and refreshed by `apply_config` so live config edits flow
    /// to the next frame without re-reading TOML in the render path.
    pub(in crate::workspace) usage_pricing: daruda_claude::usage::UsagePricing,

    /// Background-poll cadences for the OAuth `/api/oauth/usage` and
    /// public `status.claude.com` endpoints. Stored as a snapshot of
    /// `[usage.poll]` so `limits_pump` can read it under
    /// `read_with` without locking the full `Config`.
    pub(in crate::workspace) usage_poll: daruda_config::PollConfig,

    /// Latest plan-rate response (5-hour + 7-day windows). Default-
    /// constructed before the first successful fetch — gauges then
    /// render in their placeholder state. `set_plan_limits` updates
    /// this and bumps `cx.notify()`.
    pub(in crate::workspace) plan_limits: daruda_claude::PlanLimits,

    /// Latest service-status indicator from `status.claude.com`.
    /// Default-constructed (`Unknown`) before the first fetch lands;
    /// `set_service_status` updates and bumps `cx.notify()`.
    pub(in crate::workspace) service_status: daruda_claude::ServiceStatus,

    /// Time window applied when rendering the Usage tab — sessions
    /// older than the window are hidden and the Total summary
    /// aggregates only sessions inside it. Persisted via
    /// `ProjectState::active_usage_window` (default = `Last7d`).
    pub(in crate::workspace) usage_window: daruda_store::project::UsageWindow,

    /// Picker rendered at the top of the Usage tab. Workspace owns
    /// the entity so selection survives dock teardown and
    /// `restore_state` can resync the picker after the saved
    /// `usage_window` is reapplied.
    pub(in crate::workspace) usage_select: Entity<SelectState>,

    /// Keeps the `usage_select` subscription alive for the lifetime
    /// of the Workspace; dropping `Self` cancels the closure.
    #[allow(dead_code)]
    pub(in crate::workspace) _usage_select_subscription: Subscription,

    /// Claude Code session-status mirror — driven by the hook channel
    /// (and Phase B jsonl fallback). Read by the sidebar to render the
    /// per-worktree Working/NeedsAttention/Idle/Connecting indicator.
    pub(in crate::workspace) claude_status: daruda_claude::ClaudeStatusStore,

    /// Whether the Claude status feature is enabled in `[claude_status]`
    /// config. False suppresses both the indicator and the install banner.
    pub(in crate::workspace) claude_status_enabled: bool,

    /// Whether daruda's hook entries are present in
    /// `~/.claude/settings.json`. Cached: refreshed at startup and after
    /// each install/uninstall action; on disk changes by the user
    /// directly editing settings.json the cache will lag until daruda
    /// restarts (acceptable — Phase C-1 adds a settings.json watch).
    pub(in crate::workspace) claude_hooks_installed: bool,

    /// PTY → claude session tracker. Polls sysinfo every 3 s to find
    /// `claude` descendants of each registered Pane's shell PID and
    /// resolves their session_id via `~/.claude/sessions/<pid>.json`.
    pub(in crate::workspace) pty_tracker: PtyTracker,

    /// Last reported `(PaneId → claude session)` bindings from the
    /// tracker. Used by sub-row badges to highlight the focused tab's
    /// session and by `apply_pty_tracker_event` to detect dead sessions.
    pub(in crate::workspace) pty_claude_bindings: HashMap<PaneId, PtyBinding>,

    /// Keeps the tracker event-pump task alive for the Workspace
    /// lifetime; dropped when Workspace is dropped, which lets the
    /// tracker thread exit cleanly via the receiver-disconnect probe.
    #[allow(dead_code)]
    pub(in crate::workspace) _pty_event_pump: Task<()>,

    /// JSONL fallback watcher's shutdown half — engaged only when
    /// the user has not installed the hook integration. The watcher
    /// thread reads `~/.claude/projects/<encoded(cwd)>/` per worktree
    /// and feeds `JsonlEvent`s through the same store as hooks (with
    /// `Source::Jsonl`). `None` when hooks are installed (or the
    /// claude_status feature is disabled). Dropping this sender
    /// shuts the watcher thread down via its shutdown channel;
    /// `refresh_jsonl_watcher` reassigns this when install state or
    /// the worktree set changes.
    pub(in crate::workspace) _jsonl_watcher_shutdown: Option<mpsc::Sender<()>>,
    pub(in crate::workspace) _jsonl_event_pump: Option<Task<()>>,

    /// Running count of `PostToolUseFailure` hook events per Claude
    /// session. When a session crosses
    /// `daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD` the owning task
    /// is escalated to `Error { message: "tool_use_failure xN" }`
    /// (R-15 Phase 2). Cleared whenever a session is removed or
    /// transitioned out of `Running`.
    pub(in crate::workspace) tool_use_failure_counts: HashMap<String, u32>,

    /// Two background-poll tasks for `[usage.poll]` —
    /// `/api/oauth/usage` (plan-rate windows) and
    /// `status.claude.com` (service status). Each task re-reads the
    /// workspace's poll cadence every tick so live config edits flow
    /// without restart. Dropping the tasks (when Workspace is
    /// dropped) cancels the loops; held in a tuple so a single field
    /// owns both.
    #[allow(dead_code)]
    pub(in crate::workspace) _limits_pumps: (Task<()>, Task<()>),
}
