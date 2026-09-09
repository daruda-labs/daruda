//! Shared test helpers for `daruda` integration / unit tests.
//!
//! Tests that mount a window which renders any `gpui_component` widget must
//! initialise the upstream theme + global state — otherwise rendering panics
//! at `gpui_component::theme::ActiveTheme::theme(cx)` because `Theme` is a
//! global that lives on `App`. Production code calls
//! `gpui_component::init(&mut cx)` once in `main.rs`; tests don't reach that
//! path.
//!
//! Call [`init_gpui_component`] at the top of every `#[gpui::test]` that
//! constructs a `gpui_component::*` widget, modal, or workspace shell.
//! Idempotent — calling more than once is harmless.

#![cfg(test)]

use gpui::{AppContext as _, TestAppContext};

/// Initialise `gpui_component`'s theme + globals on a `TestAppContext`,
/// then overlay daruda's palette so tests render with the same colors
/// as production. Idempotent — calling more than once is harmless.
pub(crate) fn init_gpui_component(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        // DarudaTheme must be registered before apply_daruda_palette
        // because the palette reads `cx.global::<DarudaTheme>()` to
        // map slot values into `gpui_component::Theme`.
        crate::ui::theme::DarudaTheme::init(cx);
        crate::ui::theme::apply_daruda_palette(cx);
        // Register every app-wide Global the production `main.rs::app.run`
        // would set up — Workspace constructors poke them defensively
        // too, but tests that build only a sub-entity (no Workspace)
        // still need these.
        crate::agent::skills::global::init(cx);
        crate::agent::mcp::global::init(cx);
        crate::agent::tasks_global::init(cx);
    });
}

/// A window built for the external control surface: the workspace, its window
/// handle, and — for the variants that open one — an agent-chat pane.
///
/// `pane` is an `Option` rather than a `0` sentinel because pane ids start at
/// `0`, so the sentinel would also be a legal id.
pub(crate) struct ControlFixture {
    pub window: gpui::WindowHandle<gpui_component::Root>,
    pub workspace: gpui::Entity<crate::workspace::Workspace>,
    pub pane: Option<u64>,
    /// Kept alive for the fixture's lifetime: the project root the workspace
    /// resolves lanes and `.daruda/flows` against. Dropping it deletes the
    /// directory out from under the running test.
    _root: tempfile::TempDir,
}

impl ControlFixture {
    /// The agent-chat pane this fixture opened. Panics on the variant that
    /// opens none, which is the point of the `Option`.
    pub(crate) fn pane(&self) -> u64 {
        self.pane.expect("this fixture opened an agent chat pane")
    }
}

/// A registered workspace window holding one agent-chat pane.
///
/// Registered in `WindowRegistry` because the control dispatcher enumerates
/// through it — the `for_test` constructors deliberately skip registration, so
/// a fixture that wants to be *found* has to opt back in (the same thing
/// `workspace::tests::files` does).
pub(crate) fn workspace_with_agent_chat(cx: &mut TestAppContext) -> ControlFixture {
    let fixture = workspace_for_control(cx);
    let pane = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture
                .workspace
                .update(cx, |ws, cx| ws.open_agent_chat_pane_for_test(window, cx))
        })
        .expect("window is live");
    ControlFixture {
        pane: Some(pane),
        ..fixture
    }
}

/// The same window with no agent-chat pane in it.
///
/// Project-backed on purpose: a workspace with no project has no lane, so
/// `active_lane_root` is `None` and anything lane-scoped (a flow, a second
/// lane) is unreachable — which is not the shape the control surface runs in.
pub(crate) fn workspace_for_control(cx: &mut TestAppContext) -> ControlFixture {
    init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let root = tempfile::tempdir().expect("tempdir");
    let project = daruda_store::project::Project::from_path(root.path());
    let holder = std::cell::RefCell::new(None);
    let window = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            crate::workspace::Workspace::new_with_project_for_test_full(
                &config,
                Some(project.clone()),
                control_test_data_dir(),
                window,
                cx,
            )
        });
        *holder.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = holder.borrow().clone().expect("workspace constructed");
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(window.into(), workspace.downgrade(), cx);
    });
    ControlFixture {
        window,
        workspace,
        pane: None,
        _root: root,
    }
}

/// Stand up a seeded, registered orchestrator without `orchestrator::window::open`.
pub(crate) fn register_test_orchestrator(
    cx: &mut TestAppContext,
) -> crate::telegram::bridge::PaneRef {
    crate::test_support::init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let holder = std::cell::RefCell::new(None);
    let window = cx.add_window(|window, cx| {
        let ws = cx.new(|cx| {
            crate::workspace::Workspace::new_with_project_for_test_full(
                &config,
                None,
                control_test_data_dir(),
                window,
                cx,
            )
        });
        *holder.borrow_mut() = Some(ws.clone());
        gpui_component::Root::new(ws, window, cx)
    });
    let ws = holder.borrow().clone().expect("workspace constructed");
    // Unique per fixture: parallel tests must not share a state dir or a
    // working directory (`control_test_data_dir` keys on pid + a counter).
    let cwd = control_test_data_dir().join("orchestrator-cwd");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let agent = config.resolved_agents()[0].id.clone();
    cx.update_window(window.into(), |_, win, cx| {
        ws.update(cx, |ws, cx| {
            ws.seed_orchestrator_chat_pane_unrevealed_for_test(
                agent,
                cwd,
                daruda_store::accounts::AccountSelection::SystemDefault,
                None,
                win,
                cx,
            )
        })
    })
    .expect("window is live")
    .expect("seeded");
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register_orchestrator(
            window.into(),
            ws.downgrade(),
            cx,
        );
        crate::orchestrator::pane(cx).expect("the orchestrator reports its pane")
    })
}

/// A unique temp directory per fixture so parallel tests never share
/// persistence state.
///
/// Same shape as `workspace::tests::fresh_test_data_dir`, kept separate
/// because that one is private to the `workspace::tests` tree and this is
/// reached from three module trees outside it.
fn control_test_data_dir() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("daruda_control_test_{pid}_{id}"))
}
