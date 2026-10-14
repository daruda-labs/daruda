//! `Workspace` construction: the public constructors, the test-only
//! lightweight variants, and the shared `new_with_project_impl` body.

use std::collections::{HashMap, HashSet};

use gpui::{AppContext, Context, Window};

use super::{
    ConfigMirrors, Workspace, account_login_ops, accounts_global, agent_usage,
    agent_vocabulary_global, claude_session_ops, command, config_ops, flow_context, flow_paths,
    layout, left_dock, lifetimes, main_area, orchestrator_ops, pages, right_dock, sync,
    toast_layer, unseen_outcomes, window_runtime,
};

impl Workspace {
    #[allow(dead_code)]
    pub fn new(
        config: &daruda_config::Config,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_project(config, None, data_dir, window, cx)
    }

    pub fn new_with_project(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_project_impl(config, project, data_dir, window, cx, false)
    }

    /// Test-only constructor that skips the heavy runtime systems:
    /// initial tab + PTY spawn, JSONL/skills/MCP filesystem watchers,
    /// `tasks_global::load_from_dir`, `persist_state`, macro-shortcut
    /// registration, and `WindowRegistry` register. Tests that exercise
    /// any of those must opt in by calling the matching `refresh_*` /
    /// `add_tab` / `persist_state` method after construction.
    ///
    /// Rationale: 8-thread parallelism × (3 notify FS watchers + PTY
    /// shell fork/exec + sync disk I/O) is what makes the suite take
    /// ~60 s instead of <30 s.
    #[cfg(test)]
    pub fn new_with_project_for_test(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_project_impl(config, project, data_dir, window, cx, true)
    }

    /// Variant of [`Self::new_with_project_for_test`] that additionally
    /// runs the two pieces of heavy init that persistence + lane
    /// tests assume: opening the initial tab and writing the first
    /// `persist_state` snapshot. Other heavy work (FS watchers, macro
    /// shortcut registration, `WindowRegistry`) is still skipped.
    #[cfg(test)]
    pub fn new_with_project_for_test_full(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut ws = Self::new_with_project_impl(config, project, data_dir, window, cx, true);
        ws.add_tab(window, cx);
        // `persist_state` snapshots `WindowRuntime::cached_bounds`; production
        // captures them between `install_window_close_hook` and the
        // first persist. Mirror that here so the saved state has real
        // geometry instead of `None`.
        ws.capture_window_bounds(window, cx);
        ws.persist_state(cx); // lint-reentrant-reads: test-only constructor, no dock entity is in EntityState::Mut
        ws
    }

    fn new_with_project_impl(
        config: &daruda_config::Config,
        project: Option<daruda_store::project::Project>,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
        for_test: bool,
    ) -> Self {
        // Layer the project-local override (currently `[shell]` only)
        // on top of the user-global config so the workspace boots with
        // the right shell program for its project.
        let effective = config_ops::effective_config_for(project.as_ref(), config, &data_dir);
        let config = &effective;

        // Ensure app-wide Globals exist before any constructor code
        // pokes them. Production paths register these in `main.rs`;
        // test fixtures that build a Workspace directly (without
        // `init_gpui_component`) still need them for
        // `refresh_skills_watcher` / `refresh_mcp_watcher`. The
        // `Theme` Global is also needed at first paint by the dock's
        // TabBar and by `theme::current(cx)` reads inside any
        // wrapped-widget render path.
        crate::ui::theme::init_if_missing(cx);
        crate::agent::skills::global::init(cx);
        crate::agent::mcp::global::init(cx);
        crate::ui::theme::DarudaTheme::init(cx);
        crate::settings_store::SettingsStore::init(cx);

        let ws_weak = cx.entity().downgrade();
        let toast_layer = cx.new(|_| toast_layer::ToastLayer::new(ws_weak.clone()));

        // PTY tracker — single thread polls sysinfo every 3 s for
        // claude descendants of registered panes. The tracker handle
        // is shared with the Workspace (for register/unregister) and
        // the receiver feeds an event-pump task that updates the
        // bindings map. See `workspace/sync/pty.rs`.
        let (pty_tracker, pty_rx) = daruda_agent::pty_tracker::PtyTracker::spawn();
        let pty_event_pump = sync::pty::spawn(pty_rx, cx);

        // Install the app-wide accounts Global from disk if this is the
        // first window (idempotent), then take the mirror from the Global —
        // a later window reads whatever the first installed, since one
        // process = one profile = one `data_dir`.
        accounts_global::install_if_absent(
            cx,
            daruda_store::accounts::load_accounts_in(&data_dir).unwrap_or_default(),
        );
        let accounts = accounts_global::snapshot(cx);
        agent_vocabulary_global::install_path(cx, &data_dir);
        let agent_vocabulary = agent_vocabulary_global::snapshot(cx, &data_dir);
        // Sweep any per-account config dir left behind by a login that
        // was cancelled or crashed before being promoted to a
        // `ManagedAccount` — a shallow, one-shot readdir under
        // `claude-accounts/`, cheap enough to run inline here.
        //
        // `grace = account_login_ops::LOGIN_TIMEOUT`: multi-window is first-class
        // (see `WindowRegistry::for_each_workspace`), so a *different*
        // window's login can still be in flight — its config dir exists but
        // hasn't been promoted to `known_account_ids` yet — while this
        // window's constructor runs this sweep. Sparing anything younger
        // than the login timeout avoids deleting that other window's
        // live login dir out from under it; a genuine orphan (crash, or a
        // timed-out login whose cleanup never ran) is always older than
        // this, since a login can't still be running past its own timeout.
        let known_account_ids: Vec<daruda_store::accounts::AccountId> =
            accounts.accounts.iter().map(|account| account.id).collect();
        daruda_agent::accounts::sweep_orphan_dirs(
            &data_dir,
            &known_account_ids,
            account_login_ops::LOGIN_TIMEOUT,
        );

        // An OS flip while no window was open reached no observer; this
        // window's own appearance says where the OS is now — told before the
        // mirrors below read what it paints. Cached on the window, so no
        // platform call.
        crate::ui::theme::note_system_appearance(window.appearance(), cx);
        let mut ws = Self {
            uuid: daruda_store::project::WorkspaceUuid::new(),
            main_area: main_area::MainAreaContext::default(),
            orchestrator: orchestrator_ops::OrchestratorHost::default(),
            next_id: 0,
            window_runtime: window_runtime::WindowRuntime::new(window, cx),
            docks: layout::docks::Docks::new(&ws_weak, config, cx),
            workspace_page: None,
            pages: pages::detail::PageDetails::default(),
            unseen_outcomes: unseen_outcomes::UnseenOutcomes::default(),
            agent_usage: agent_usage::AgentUsageState {
                _limits_pumps: sync::limits::spawn(cx),
                ..Default::default()
            },
            claude_sessions: claude_session_ops::ClaudeSessionState {
                status: claude_session_ops::initial_claude_status(config, cx),
                hooks_installed: daruda_agent::hooks::installer::InstallerPaths::from_env()
                    .map(|p| daruda_agent::hooks::installer::is_installed(&p))
                    .unwrap_or(false),
                pty_tracker,
                pty_bindings: HashMap::new(),
                _pty_event_pump: pty_event_pump,
                _jsonl_watcher: None,
                _jsonl_event_pump: None,
                tool_use_failure_counts: HashMap::new(),
                last_pushed_notification: HashMap::new(),
                open_turns: Default::default(),
            },
            // Bootstrap projects for this workspace. When the caller
            // supplies a project root, wrap it in a single runtime
            // `Project` (id `0`) carrying a pure placeholder lane —
            // git discovery is deferred to
            // `reconcile_bootstrapped_lanes` below so window creation
            // never blocks on git CLI. Otherwise the workspace starts
            // with no projects (Landing path).
            projects: project
                .as_ref()
                .map(|p| {
                    crate::project::Projects::from(vec![
                        crate::project::Project::bootstrap_placeholder(0, p.root.clone()),
                    ])
                })
                .unwrap_or_default(),
            active: daruda_store::project::LaneRef::default(),
            groups: Vec::new(),
            next_project_id: if project.is_some() { 1 } else { 0 },
            next_group_id: 0,
            window_open_policy: daruda_store::project::WindowOpenPolicy::default(),
            settings: None,
            overlays: command::overlays::CommandOverlays::default(),
            flows: flow_context::FlowContext::new(window, cx),
            lane_scoped: HashMap::new(),
            file_tree: left_dock::file_tree_context::FileTreeContext {
                files_watcher_poll: None,
                files_panel_focus: cx.focus_handle(),
                files_selection: None,
                files_scroll_handle: gpui::UniformListScrollHandle::new(),
            },
            mirrors: ConfigMirrors::from_config(
                config,
                crate::ui::theme::painted_ui_preset(&config.theme.ui_preset, cx),
            ),
            lanes_scroll_handle: gpui::ScrollHandle::new(),
            last_error: None,
            #[cfg(test)]
            error_history: Vec::new(),
            toast_layer,
            cached_project_config: None,
            last_agent_id: None,
            agent_pulse_prev: Vec::new(),
            git: left_dock::git_ops::context::GitContext::new(&ws_weak, window, cx),
            left_dock_preview: None,
            panels: main_area::bottom_dock::macro_ops::load_or_seed_panels(&data_dir),
            agent_vocabulary,
            accounts,
            login: account_login_ops::LoginState::default(),
            // Managed accounts live in the app-wide `AccountsGlobal`; this
            // subscription refreshes the `accounts` read-cache from it and
            // repaints whenever any window mutates it (single, symmetric
            // cross-window propagation path — see `accounts_global`).
            right_views: right_dock::context::RightDockViews::new(window, cx),
            input_dock: main_area::bottom_dock::input_dock::InputDock::new(
                &ws_weak, config, window, cx,
            ),
            persistence_suspended: false,
            pending_lane_creates: HashSet::new(),
            // The shared root in production; under the test's own data
            // directory otherwise, so a suite takes no lock the developer's
            // app could see and leaves nothing in their config directory.
            lock_root: if for_test {
                data_dir.join(flow_paths::LOCKS_DIR)
            } else {
                daruda_store::persistence::flow_lock_root()
            },
            workspace_store: crate::workspace_storage::for_root(&data_dir, cx),
            data_dir,

            // Re-resolve the user layer with this workspace's project
            // overlay and reapply whenever `SettingsStore` changes —
            // both the FS watch tick and the Settings-window save
            // fold into the same Global mutation and fanout here.
            // Observe the Updater entity: it self-notifies on every status
            // transition, so the handler filters for `Available` and toasts
            // once per new version.
            last_update_toast_version: None,
            ports: sync::ports::PortsState::default(),
            _observers: lifetimes::GlobalObservers::new(
                cx.observe_global::<accounts_global::AccountsGlobal>(|ws, cx| {
                    ws.accounts = accounts_global::snapshot(cx);
                    ws.reconcile_account_pins(cx);
                    cx.notify();
                }),
                cx.observe_global::<agent_vocabulary_global::AgentVocabularyGlobal>(|ws, cx| {
                    ws.agent_vocabulary = agent_vocabulary_global::snapshot(cx, &ws.data_dir);
                    cx.notify();
                }),
                cx.observe_global::<crate::agent::mcp::McpState>(|_ws, cx| {
                    // Right dock re-stages + diffs on this notify.
                    cx.notify();
                }),
                cx.observe_global::<crate::settings_store::SettingsStore>(|ws, cx| {
                    ws.apply_store_config(cx);
                }),
                cx.observe_global::<crate::agent::skills::global::GlobalSkills>(|_ws, cx| {
                    // Right dock re-stages + diffs on this notify.
                    cx.notify();
                }),
                cx.observe_global::<crate::agent::tasks_global::GlobalTasks>(|ws, cx| {
                    ws.ensure_task_live_tick(cx);
                    cx.notify();
                }),
                crate::update::Updater::get(cx).map(|e| {
                    cx.observe(&e, |this: &mut Workspace, updater, cx| {
                        this.on_updater_status_changed(&updater, cx);
                    })
                }),
            ),
            pumps: lifetimes::Pumps::new(sync::ports::spawn(cx)),
        };
        // Invariant seed: the active lane's runtime must always exist in
        // `runtimes` so `active_runtime()` (read unconditionally by
        // `render`) never panics — including the Landing state, where
        // `active` is `LaneRef::default()` and no project is open. Seed
        // it for whatever `active` is, unconditionally: production then
        // populates the first project's lane via `add_tab` below (the only
        // auto-seed left), and an empty workspace leaves it empty (render
        // shows Landing). Lanes activated later are never auto-seeded —
        // they render the empty-state until the user opens content.
        ws.main_area.runtimes.entry(ws.active).or_default();
        // Above the test short-circuit: a subscription spawns nothing, and
        // the unseen-outcome tests drive exactly these two triggers.
        Self::observe_outcome_visibility(window, cx);
        // Test-only short-circuit: every line below this point spawns a
        // background thread (PTY, FS watchers) or performs sync disk I/O
        // (persist_state, tasks_global::load_from_dir). Tests that need
        // any of these must invoke them explicitly after construction;
        // skipping them by default is what brings the suite under 30 s.
        if for_test {
            // Render snapshots read `GlobalTasks`; registering an empty
            // one here costs nothing and prevents
            // "no state of type GlobalTasks exists" panics on the first
            // `cx.notify()` triggered by any test mutation.
            crate::agent::tasks_global::init(cx);
            return ws;
        }
        // Seed the first tab only when there is a project to root it at.
        // With no project there is no working directory, so the shell would
        // land in `$HOME` — somewhere nobody asked for, and it would cover
        // the Landing view on every project-less launch.
        //
        // This governs the *automatic* seed only. `add_tab` still honours an
        // explicit `+` / Cmd+T in an empty workspace (see its own comment):
        // there the user did ask, and `$HOME` is the documented fallback.
        if !ws.projects.is_empty() {
            ws.add_tab(window, cx);
        }
        // After tabs/lanes are seeded, decide whether the JSONL
        // fallback watcher should run for this Workspace and start it.
        ws.refresh_jsonl_watcher(cx);
        // Skills watcher: subscribes to project + personal skill dirs
        // and re-scans on every filesystem change.
        ws.refresh_skills_watcher(cx);
        // MCP watcher: subscribes to ~/.claude/settings.json and the
        // active lane's .mcp.json, re-loading the matching scope on
        // every external edit.
        ws.refresh_mcp_watcher(window, cx);
        // Flow watcher: subscribes to the active lane's flow directories so a
        // graph and the Flows page follow an edit made outside the app.
        ws.respawn_flow_watcher(cx);
        // Load the right-panel Tasks tab from this workspace's
        // `data_dir`. Production paths all share the default
        // `~/.config/daruda/`; tests inject a fresh per-test dir.
        crate::agent::tasks_global::load_from_dir(cx, &ws.data_dir);
        ws.restore_task_cli_tracking(cx);
        // Kick off the pulse / duration tick if `load_from_dir`
        // restored any `Running` task — the `GlobalTasks` observe
        // subscription wouldn't fire here because the set_global call
        // pre-dates the subscription registration above.
        ws.ensure_task_live_tick(cx);
        // Background-detect `default_branch` for the bootstrapped project
        // (if any). `Project::bootstrap` sets it to `None` to avoid
        // blocking the UI thread during window creation; this fires
        // asynchronously and persists the result when it returns.
        ws.reconcile_project_default_branches(cx);
        // Upgrade the construction placeholder lane to the
        // git-discovered list off the UI thread (see
        // `Project::bootstrap_placeholder`).
        ws.reconcile_bootstrapped_lanes(cx);

        // `window.appearance()` is the value the platform just handed over;
        // asking the platform again from here panics on Linux.
        cx.observe_window_appearance(window, |this: &mut Workspace, window, cx| {
            this.on_system_appearance_changed(window.appearance(), cx);
        })
        .detach();

        crate::platform::window_controls::compact(window);
        cx.observe_window_bounds(window, |this: &mut Workspace, window, cx| {
            crate::platform::window_controls::compact(window);
            this.capture_window_bounds(window, cx);
            this.resize_all_tabs(window, cx);
        })
        .detach();

        // Intercept `Cmd+Q` and red-cross close attempts so unsaved edits
        // (file panes, the Task editor) don't silently disappear. The callback returns
        // `false` to veto the close, spawns the async batch prompt, and
        // then re-issues `window.remove_window()` once the user picks
        // Save all / Discard all.
        Self::install_window_close_hook(ws_weak.clone(), window, cx);
        // Capture initial bounds so first save carries real geometry.
        ws.capture_window_bounds(window, cx);

        // Persist initial state so the project is restorable on next launch.
        ws.persist_state(cx); // lint-reentrant-reads: no dock entity is in EntityState::Mut during construction.

        // Register every macro shortcut from the loaded panels state.
        // Uses the App context (cx derefs to App) — global bindings,
        // last-wins, so two windows registering the same shortcut is
        // harmless.
        main_area::bottom_dock::macro_ops::register_macro_shortcuts(&ws.panels, cx);

        // Register this Workspace in the WindowRegistry so broadcasts
        // and window-lifecycle helpers can find it without iterating
        // all windows. The weak is captured at construction time rather
        // than inside the on_release closure: by the time on_release
        // fires, cx.entity() refers to an entity in the process of being
        // dropped, so pre-building the weak here is the safe pattern
        // (mirrors Zed's WorkspaceStore deregistration).
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        crate::window_registry::WindowRegistry::register(window_handle, weak.clone(), cx);
        // A restored workspace can open with unread worktrees and no status
        // change yet to recount them, and agent sessions keep-awake counts.
        Self::refresh_dock_badge(cx);
        crate::keep_awake::refresh(cx);
        cx.on_release(move |ws: &mut Workspace, cx: &mut gpui::App| {
            // Before deregistering: a login this window owned can no longer be
            // finished by anyone, and the process-wide slot it holds would
            // block every other window until the login timed out.
            ws.release_pending_login_on_close(cx);
            crate::window_registry::WindowRegistry::deregister(&weak, cx);
            // This window's worktrees and sessions leave both counts.
            Workspace::refresh_dock_badge(cx);
            crate::keep_awake::refresh(cx);
        })
        .detach();

        ws
    }
}
