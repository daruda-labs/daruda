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
        briefing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let pane_id =
            self.insert_orchestrator_chat_pane(agent_id, cwd, account, briefing, window, cx)?;
        // Last, because revealing focuses the pane and focusing is what starts
        // the session — everything the session depends on has to be true
        // before this line.
        self.reveal_new_agent_chat_pane(pane_id, window, cx);
        Some(pane_id)
    }

    /// Everything [`Self::seed_orchestrator_chat_pane`] does except revealing.
    ///
    /// Split for the same reason `control_insert_chat` is: revealing starts a
    /// real ACP adapter, whose task outlives a test and then trips gpui's
    /// determinism assert in whichever test runs next — so a fixture can stand
    /// an orchestrator up without a session.
    fn insert_orchestrator_chat_pane(
        &mut self,
        agent_id: String,
        cwd: PathBuf,
        account: AccountSelection,
        briefing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        self.empty_active_lane_runtime(window, cx);
        // No lane, so the lane-derived cwd triple would be all `None` and the
        // pane would park in `Error`; the orchestrator supplies its own.
        let pane_id = self.insert_agent_chat_pane(agent_id, (Some(cwd), None, None), window, cx)?;
        // Both before any reveal: the account decides which config dir the
        // session runs under, and the briefing has to be armed before the
        // first prompt can reach the wire.
        self.set_agent_chat_account(pane_id, account);
        if let Some(briefing) = briefing
            && let Some(view) = self.agent_chat_view(pane_id).cloned()
        {
            view.update(cx, |v, _| v.set_briefing(briefing));
        }
        Some(pane_id)
    }

    /// [`Self::insert_orchestrator_chat_pane`] for a fixture that wants an
    /// orchestrator window without a live session.
    #[cfg(test)]
    pub(crate) fn seed_orchestrator_chat_pane_unrevealed_for_test(
        &mut self,
        agent_id: String,
        cwd: PathBuf,
        account: AccountSelection,
        briefing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        self.insert_orchestrator_chat_pane(agent_id, cwd, account, briefing, window, cx)
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
                    unique_dir("state"),
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

    /// A directory no other test in this process shares. Parallel tests write
    /// real files here, so a fixed name would let them collide.
    fn unique_dir(kind: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "daruda_orchestrator_test_{}_{id}_{kind}",
            std::process::id()
        ))
    }

    fn seed(
        fixture: &(
            gpui::WindowHandle<gpui_component::Root>,
            gpui::Entity<Workspace>,
        ),
        account: AccountSelection,
        cx: &mut TestAppContext,
    ) -> Option<PaneId> {
        seed_at(fixture, account, None, cx).map(|(pane, _)| pane)
    }

    /// Same, also reporting the directory the pane was rooted at — unique per
    /// call, so the assertion cannot pass against another test's leftovers.
    fn seed_at(
        fixture: &(
            gpui::WindowHandle<gpui_component::Root>,
            gpui::Entity<Workspace>,
        ),
        account: AccountSelection,
        briefing: Option<String>,
        cx: &mut TestAppContext,
    ) -> Option<(PaneId, PathBuf)> {
        // These are the tests that exercise the *revealing* seed, so a real
        // ACP adapter really does start and wake this app from its own thread.
        // gpui's sanctioned opt-out for that, rather than a determinism assert
        // this test cannot honour — fixtures that only need a pane use
        // `seed_orchestrator_chat_pane_unrevealed_for_test`.
        cx.executor().allow_parking();
        let cwd = unique_dir("cwd");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let agent = daruda_config::Config::default().resolved_agents()[0]
            .id
            .clone();
        let pane = cx
            .update_window(fixture.0.into(), |_, window, cx| {
                fixture.1.update(cx, |ws, cx| {
                    ws.seed_orchestrator_chat_pane(
                        agent,
                        cwd.clone(),
                        account,
                        briefing.clone(),
                        window,
                        cx,
                    )
                })
            })
            .expect("window is live")?;
        Some((pane, cwd))
    }

    /// Production registers the orchestrator *before* seeding it
    /// (`orchestrator::window::seed`), and the seed's reveal connects from
    /// inside `workspace.update` — so the connect path asks "am I the
    /// orchestrator?" about the entity it is already holding. Reading that
    /// entity back through the registry's handle is a double-lease panic; this
    /// pins the order that finds it.
    #[gpui::test]
    fn seeding_an_already_registered_orchestrator_does_not_double_lease(cx: &mut TestAppContext) {
        let fixture = projectless_window(cx);
        cx.update(|cx| {
            crate::window_registry::WindowRegistry::register_orchestrator(
                fixture.0.into(),
                fixture.1.downgrade(),
                cx,
            );
        });
        assert!(
            seed(&fixture, AccountSelection::SystemDefault, cx).is_some(),
            "the seed completes rather than panicking"
        );
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
        let (pane, expected) =
            seed_at(&fixture, AccountSelection::SystemDefault, None, cx).expect("seeded");
        fixture.1.read_with(cx, |ws, _| {
            assert_eq!(ws.pane_local_cwd_for_test(pane), Some(expected));
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
