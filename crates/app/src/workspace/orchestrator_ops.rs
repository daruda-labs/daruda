//! `impl Workspace` for the orchestrator's window.
//!
//! One operation: replace whatever the constructor's `add_tab` opened with a
//! single agent-chat pane rooted at the orchestrator's own directory. It lives
//! here rather than in `orchestrator/window.rs` because the body is a Model
//! change, and those belong to a `*_ops.rs`.

use gpui::{Context, Window};

use std::path::PathBuf;

use daruda_store::accounts::AccountSelection;

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Leave this window holding exactly one agent-chat pane, under
    /// `agent_id`, rooted at `cwd`, pinned to `account`.
    ///
    /// `Workspace::new` has already opened a terminal tab by the time this
    /// runs (it calls `add_tab`), so the first step is to empty the runtime
    /// rather than to add anything.
    ///
    /// `None` when the pane could not be inserted.
    pub(crate) fn seed_orchestrator_chat_pane(
        &mut self,
        agent_id: String,
        cwd: PathBuf,
        account: AccountSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        self.empty_active_lane_runtime(window, cx);
        // No lane, so the lane-derived cwd triple would be all `None` and the
        // pane would park in `Error`; the orchestrator supplies its own.
        let pane_id = self.insert_agent_chat_pane(agent_id, (Some(cwd), None, None), window, cx)?;
        // Between insert and reveal on purpose: revealing focuses the pane,
        // which is what starts the session, and the account decides which
        // config dir that session runs under.
        self.set_agent_chat_account(pane_id, account);
        self.reveal_new_agent_chat_pane(pane_id, window, cx);
        Some(pane_id)
    }

    /// Overwrite a freshly inserted chat pane's account. Narrow on purpose:
    /// switching a *live* pane's account is `account_ops`' job, which has a
    /// whole state machine for what happens to the session in flight.
    fn set_agent_chat_account(&mut self, pane_id: PaneId, account: AccountSelection) {
        if let Some(content) = self
            .active_runtime_mut()
            .panes
            .iter_mut()
            .find(|p| p.id == pane_id)
            .and_then(crate::workspace::main_area::pane::Pane::agent_chat_content_mut)
        {
            content.account = account;
        }
    }

    /// This window's agent-chat pane, addressed the way a control command
    /// addresses one. `None` before the seed runs, or if it refused.
    ///
    /// Return the first agent-chat pane in this window.
    pub(crate) fn orchestrator_chat_pane(&self) -> Option<crate::telegram::bridge::PaneRef> {
        let pane = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.agent_chat_view().is_some())?;
        Some(crate::telegram::bridge::PaneRef {
            workspace: self.uuid(),
            pane: pane.id,
        })
    }

    /// Every pane in this window, paired with whether it is an agent chat.
    #[cfg(test)]
    pub(crate) fn orchestrator_panes_for_test(&self) -> Vec<(PaneId, bool)> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| {
                rt.panes
                    .iter()
                    .map(|p| (p.id, p.agent_chat_view().is_some()))
            })
            .collect()
    }

    /// A pane's local working directory, for the assertion that the
    /// orchestrator's pane is rooted where `window::cwd` says.
    #[cfg(test)]
    pub(crate) fn pane_local_cwd_for_test(&self, pane_id: PaneId) -> Option<PathBuf> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.id == pane_id)
            .and_then(|p| p.cwd())
            .map(std::path::Path::to_path_buf)
    }

    #[cfg(test)]
    pub(crate) fn projects_is_empty_for_test(&self) -> bool {
        self.projects.is_empty()
    }

    /// `snapshot_for_disk` is `pub(in crate::workspace)`; this lets the
    /// orchestrator's own test assert the window never reaches the save path.
    #[cfg(test)]
    pub(crate) fn has_disk_snapshot_for_test(&self, cx: &gpui::App) -> bool {
        self.snapshot_for_disk(cx).is_some()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext};

    use daruda_store::accounts::{AccountId, AccountSelection};

    use super::*;
    use crate::workspace::Workspace;

    /// A project-less window in the shape `orchestrator::window::open` builds
    /// one: the constructor has already opened its terminal tab, so what the
    /// seed does to that tab is observable.
    ///
    /// Built with `add_window` + the test constructor, like every other
    /// agent-chat test — the production constructor starts background pumps
    /// that race the gpui test scheduler.
    fn projectless_window(
        cx: &mut TestAppContext,
    ) -> (
        gpui::WindowHandle<gpui_component::Root>,
        gpui::Entity<Workspace>,
    ) {
        crate::test_support::init_gpui_component(cx);
        let config = daruda_config::Config::default();
        let holder = std::cell::RefCell::new(None);
        let handle = cx.add_window(|window, cx| {
            let ws = cx.new(|cx| {
                Workspace::new_with_project_for_test_full(
                    &config,
                    None,
                    std::env::temp_dir().join(format!(
                        "daruda_orchestrator_test_{}_{}",
                        std::process::id(),
                        next_id()
                    )),
                    window,
                    cx,
                )
            });
            *holder.borrow_mut() = Some(ws.clone());
            gpui_component::Root::new(ws, window, cx)
        });
        (
            handle,
            holder.borrow().clone().expect("workspace constructed"),
        )
    }

    fn next_id() -> u64 {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn seed(
        fixture: &(
            gpui::WindowHandle<gpui_component::Root>,
            gpui::Entity<Workspace>,
        ),
        account: AccountSelection,
        cx: &mut TestAppContext,
    ) -> Option<PaneId> {
        let cwd = std::env::temp_dir().join("daruda_orchestrator_cwd");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let agent = daruda_config::Config::default().resolved_agents()[0]
            .id
            .clone();
        cx.update_window(fixture.0.into(), |_, window, cx| {
            fixture.1.update(cx, |ws, cx| {
                ws.seed_orchestrator_chat_pane(agent, cwd, account, window, cx)
            })
        })
        .expect("window is live")
    }

    /// The constructor's `add_tab` leaves a terminal pane; the seed must
    /// replace it rather than open a chat pane beside it.
    #[gpui::test]
    fn seeding_leaves_exactly_one_chat_pane(cx: &mut TestAppContext) {
        let fixture = projectless_window(cx);
        fixture.1.read_with(cx, |ws, _| {
            let before = ws.orchestrator_panes_for_test();
            assert_eq!(before.len(), 1, "the constructor opened a tab");
            assert!(!before[0].1, "and it is a terminal, not a chat");
        });

        let pane = seed(&fixture, AccountSelection::SystemDefault, cx).expect("seeded");
        fixture.1.read_with(cx, |ws, _| {
            let after = ws.orchestrator_panes_for_test();
            assert_eq!(after.len(), 1, "no terminal tab alongside: {after:?}");
            assert_eq!(after[0], (pane, true), "the one pane is the chat pane");
        });
    }

    #[gpui::test]
    fn the_seeded_pane_is_rooted_at_the_directory_it_was_given(cx: &mut TestAppContext) {
        let fixture = projectless_window(cx);
        let pane = seed(&fixture, AccountSelection::SystemDefault, cx).expect("seeded");
        fixture.1.read_with(cx, |ws, _| {
            assert_eq!(
                ws.pane_local_cwd_for_test(pane),
                Some(std::env::temp_dir().join("daruda_orchestrator_cwd")),
            );
        });
    }

    /// The account has to be on the pane before the reveal that starts the
    /// session, since it decides which config dir that session runs under.
    #[gpui::test]
    fn the_seeded_pane_carries_the_configured_account(cx: &mut TestAppContext) {
        let id = AccountId::new();
        let fixture = projectless_window(cx);
        let pane = seed(&fixture, AccountSelection::Managed(id), cx).expect("seeded");
        fixture.1.read_with(cx, |ws, _| {
            assert_eq!(
                ws.agent_chat_account_selection(pane),
                AccountSelection::Managed(id)
            );
        });
    }

    /// No project, so `snapshot_for_disk` short-circuits and the window never
    /// reaches the save path — it cannot be restored on next launch.
    #[gpui::test]
    fn the_window_is_not_persisted(cx: &mut TestAppContext) {
        let fixture = projectless_window(cx);
        seed(&fixture, AccountSelection::SystemDefault, cx).expect("seeded");
        fixture.1.read_with(cx, |ws, cx| {
            assert!(ws.projects_is_empty_for_test());
            assert!(!ws.has_disk_snapshot_for_test(cx));
        });
    }
}
