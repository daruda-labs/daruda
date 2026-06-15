//! Skills watcher → GPUI bridge.
//!
//! Mirrors `jsonl_pump`: every 100 ms tick the pump task drains every
//! [`SkillsEvent`] queued by the watcher and dispatches a per-scope
//! reload through [`Workspace::apply_skills_event`]. Dropping the
//! returned `Task<()>` (held as `_skills_event_pump` on `Workspace`)
//! cancels the loop, the receiver disconnects, and the watcher
//! threads exit.
//!
//! Mutations land in the app-wide `SkillsState` Global; other
//! Workspaces re-render via their `observe_global::<SkillsState>`
//! subscription registered in `Workspace::new_with_project`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui::{BorrowAppContext, Context, Task};

use crate::agent::skills::{SkillScope, SkillsState, scan};
use crate::hooks::skills_watcher::{self, SkillsEvent};
use crate::workspace::Workspace;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(in crate::workspace) fn spawn(
    events: Receiver<SkillsEvent>,
    cx: &mut Context<Workspace>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        'outer: loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            loop {
                let ev = match events.try_recv() {
                    Ok(e) => e,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'outer,
                };
                if this
                    .update(cx, |ws, cx| ws.apply_skills_event(ev, cx))
                    .is_err()
                {
                    break 'outer;
                }
            }
        }
    })
}

impl Workspace {
    /// Apply one debounced [`SkillsEvent`] from the watcher.
    ///
    /// The event carries only the scope; the lane path is taken
    /// from *this* workspace's active lane at fire time so the
    /// pump never scans against a stale `project_root`. The result
    /// lands in the `SkillsState` Global, where every other open
    /// Workspace picks it up through `observe_global`.
    pub(in crate::workspace) fn apply_skills_event(
        &mut self,
        event: SkillsEvent,
        cx: &mut Context<Self>,
    ) {
        let SkillsEvent::Reloaded(scope) = event;
        let lane = self.active_lane_root();
        let personal = scan::skills_personal_dir();
        cx.update_global::<SkillsState, _>(|state, _| {
            state.reload_scope(scope, lane.as_deref(), &personal);
            if scope == SkillScope::Personal {
                // ~/.claude/skills also sources @skills-dir plugins → refresh Plugin too.
                state.reload_scope(SkillScope::Plugin, None, &personal);
            }
        });
        cx.notify();
    }

    /// Project-skills root for the active lane. `None` when the
    /// workspace has no active lane (welcome window).
    pub(in crate::workspace) fn active_lane_root(&self) -> Option<PathBuf> {
        self.active_lane().map(|wt| wt.path.clone())
    }

    /// (Re)spawn the Skills watcher with the current lane's project
    /// root. Call from initial construction, lane create / remove,
    /// and lane activation. Synchronously refreshes the Global so
    /// the panel reflects the new lane before the watcher's first
    /// debounced fire arrives.
    pub fn refresh_skills_watcher(&mut self, cx: &mut Context<Self>) {
        // Drop the previous watcher + pump first so the old FSEvent
        // subscription unregisters before we attach a new one.
        self._skills_watcher = None;
        self._skills_event_pump = None;

        let project_root = self.active_lane_root();
        let personal = scan::skills_personal_dir();

        cx.update_global::<SkillsState, _>(|state, _| {
            state.reload_scope(SkillScope::Personal, None, &personal);
            state.reload_scope(SkillScope::Plugin, None, &personal);
            state.reload_scope(SkillScope::Project, project_root.as_deref(), &personal);
        });
        cx.notify();

        let project_skills_dir = project_root.as_deref().map(scan::skills_project_dir);
        // Use the `~/.claude/plugins` root so the watcher catches
        // changes under both `cache/` (installed) and `marketplaces/`
        // (registered-but-not-installed) without needing two
        // subscriptions. Path matching in the callback narrows down
        // to the skills subtree.
        let plugin_root = crate::agent::skills::plugins::plugins_root();
        let (events, handle) = skills_watcher::spawn(project_skills_dir, personal, plugin_root);
        let pump = spawn(events, cx);
        self._skills_watcher = Some(handle);
        self._skills_event_pump = Some(pump);
    }
}
