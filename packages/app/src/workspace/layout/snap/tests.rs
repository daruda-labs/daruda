use super::*;

use std::sync::Arc;

use gpui::{AppContext as _, ScrollHandle, TestAppContext, Window};

/// Build a minimal but real `LeftDockSnapshot` inside a live window so
/// the excluded GPUI handle fields (`FocusHandle` / `ScrollHandle` /
/// `Entity<InputPanel>` / `WeakEntity<Workspace>`) are valid instances.
/// The data fields default to empty/false so individual tests mutate
/// only the field under examination.
fn fixture(window: &mut Window, cx: &mut gpui::App) -> LeftDockSnapshot {
    let git_commit_input = cx.new(|cx| {
        crate::ui::InputPanel::new(crate::ui::InputPanelLayout::ActionsBelow, window, cx)
    });
    LeftDockSnapshot {
        metrics: Default::default(),
        expanded: true,
        open_tabs: Default::default(),
        named_sessions: Default::default(),
        left_dock_view: daruda_store::project::LeftDockView::default(),
        workspace_page: None,
        lanes: Vec::new(),
        projects: Vec::new(),
        groups: Vec::new(),
        active: daruda_store::project::LaneRef::default(),
        git_worktree_cache: std::collections::HashMap::new(),
        git_tracking_cache: std::collections::HashMap::new(),
        git_stage_in_flight: false,
        git_op_in_flight: false,
        git_collapsed_dirs: std::collections::HashSet::new(),
        git_against_base: None,
        git_against_base_collapsed: false,
        git_changes_cursor: None,
        git_changes_panel_focus: Handle(cx.focus_handle()),
        focused_file_selection: None,
        git_changes_scroll_handle: Handle(gpui::UniformListScrollHandle::new()),
        lanes_scroll_handle: Handle(ScrollHandle::new()),
        git_commit_input: Handle(git_commit_input),
        files_panel_focus: Handle(cx.focus_handle()),
        files_scroll_handle: Handle(UniformListScrollHandle::new()),
        files_icon_color_mode: daruda_config::IconColorMode::default(),
        cached_visible: ByPointer(Arc::new(Vec::new())),
        root_kind: None,
        agent_status_per_lane: std::collections::HashMap::new(),
        agent_outcome_per_lane: std::collections::HashMap::new(),
        agent_per_session_per_lane: std::collections::HashMap::new(),
        agent_active_session_id: None,
        workspace: Handle(WeakEntity::new_invalid()),
    }
}

/// The right dock's counterpart to [`fixture`]. It had none: every one of
/// its 21 comparisons was unverified, in the dock whose exclusion list also
/// mixed handles with a per-frame clock. Data fields start empty so a test
/// mutates only the field it is about.
fn right_fixture(window: &mut Window, cx: &mut gpui::App) -> RightDockSnapshot {
    let skill_search_input = cx.new(|cx| crate::ui::InputState::new(window, cx));
    let task_search_input = cx.new(|cx| crate::ui::InputState::new(window, cx));
    RightDockSnapshot {
        right_dock_view: daruda_store::project::RightDockView::default(),
        workspace: Handle(WeakEntity::new_invalid()),
        usage: Vec::new(),
        focused_agent_domain: crate::workspace::main_area::pane::AccountDomain::Any,
        usage_domain_override: None,
        stop_on_overage: false,
        activity: Vec::new(),
        recent_sessions: Vec::new(),
        usage_refresh_in_flight: false,
        skills: crate::agent::skills::SkillsSnapshot::default(),
        skill_search_input: Handle(skill_search_input),
        skill_search_query: String::new(),
        skill_plugin_expanded: std::collections::HashSet::new(),
        sections: Default::default(),
        tasks: daruda_store::tasks::TasksBook::default(),
        task_browser: crate::workspace::right_dock::tasks::TaskBrowserSnapshot {
            state: Default::default(),
            search: Handle(task_search_input),
            query: String::new(),
        },
        task_projects: TaskProjects::default(),
        task_agents: std::sync::Arc::from([]),
        claude_status_per_session: std::collections::HashMap::new(),
        tool_use_failure_counts: std::collections::HashMap::new(),
        now: PerFrame(chrono::Utc::now()),
        right_panel_scroll_handle: Handle(ScrollHandle::new()),
        mcp: crate::agent::mcp::McpSnapshot::default(),
        library: Default::default(),
    }
}

/// The contract the wrappers exist for: distinct handle instances and a
/// clock that advanced between frames must not report a change. `now` is
/// the only `PerFrame` field in the app, so this is its sole check —
/// without it the right dock would repaint on every single frame.
#[gpui::test]
fn right_identical_content_does_not_differ(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = right_fixture(window, cx);
        let mut b = right_fixture(window, cx);
        b.now = PerFrame(
            a.now
                .checked_add_signed(chrono::Duration::seconds(1))
                .unwrap(),
        );
        assert!(
            !a.content_differs(&b),
            "distinct handles and an advanced clock are not content"
        );
        gpui::Empty
    });
}

/// A `PerFrame` field is kept out of the diff so it cannot force a
/// repaint — which only works if staging still takes the fresh snapshot.
/// Gate the assignment on the diff as well and the right dock's clock
/// freezes at the last frame something else changed, so a Running task's
/// elapsed time stops advancing while the task-live tick repaints it four
/// times a second.
#[gpui::test]
fn staging_takes_the_fresh_snapshot_even_when_it_will_not_repaint(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let mut dock =
            super::super::Dock::new(super::super::DockPosition::Right, WeakEntity::new_invalid());

        let first = right_fixture(window, cx);
        assert!(
            dock.stage(DockSnapshot::Right(Box::new(first))),
            "the first snapshot always counts as a change"
        );

        let mut later = right_fixture(window, cx);
        let advanced = later
            .now
            .checked_add_signed(chrono::Duration::seconds(30))
            .expect("30s past the fixture clock");
        later.now = PerFrame(advanced);

        assert!(
            !dock.stage(DockSnapshot::Right(Box::new(later))),
            "only the clock moved, so the dock must not repaint"
        );
        let DockSnapshot::Right(staged) = &dock.snap else {
            panic!("the right snapshot is staged");
        };
        assert_eq!(
            *staged.now, advanced,
            "the refused repaint must still leave the fresh clock staged"
        );
        gpui::Empty
    });
}

/// And the other half: an unwrapped field really is compared.
#[gpui::test]
fn right_content_change_differs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = right_fixture(window, cx);
        let mut b = right_fixture(window, cx);
        b.task_browser.state.filter = daruda_store::tasks::TaskFilter::Running;
        assert!(
            a.content_differs(&b),
            "a changed Tasks filter must re-stage the panel"
        );
        let mut c = right_fixture(window, cx);
        c.task_browser.state.scope = daruda_store::tasks::TaskScope::AllProjects;
        assert!(
            a.content_differs(&c),
            "a changed Tasks scope must re-stage the panel"
        );
        let mut d = right_fixture(window, cx);
        d.task_browser
            .state
            .groups
            .set_mode(crate::workspace::right_dock::tasks::TaskGrouping::Status);
        assert!(
            a.content_differs(&d),
            "a changed Tasks grouping must re-stage the panel"
        );
        let mut e = right_fixture(window, cx);
        e.task_browser.query = "login".into();
        assert!(
            a.content_differs(&e),
            "a changed Tasks query must re-stage the panel"
        );
        gpui::Empty
    });
}

#[gpui::test]
fn identical_content_does_not_differ(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = fixture(window, cx);
        let b = fixture(window, cx);
        // Distinct handle instances (separate `cx.focus_handle()` /
        // `InputPanel` entities) must NOT trip the diff — only content
        // matters. `cached_visible` shares the same `Arc` only when one
        // is cloned from the other, so clone `b`'s into a copy to model
        // the "cache reused" path.
        let b = LeftDockSnapshot {
            cached_visible: ByPointer(a.cached_visible.clone()),
            ..b
        };
        assert!(
            !a.content_differs(&b),
            "identical content (modulo handles + shared cache Arc) must not differ"
        );
        gpui::Empty
    });
}

#[gpui::test]
fn claude_status_change_differs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = fixture(window, cx);
        let mut b = fixture(window, cx);
        b.cached_visible = ByPointer(a.cached_visible.clone());
        b.agent_status_per_lane.insert(
            daruda_store::project::LaneRef::default(),
            daruda_agent::SessionStatus::Working,
        );
        assert!(
            a.content_differs(&b),
            "a changed agent_status_per_lane must trip content_differs"
        );
        gpui::Empty
    });
}

#[gpui::test]
fn agent_outcome_change_differs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let before = fixture(window, cx);
        let mut after = fixture(window, cx);
        after.cached_visible = ByPointer(before.cached_visible.clone());
        after.agent_outcome_per_lane.insert(
            daruda_store::project::LaneRef::default(),
            daruda_agent::AgentOutcome::Completed,
        );
        assert!(before.content_differs(&after));
        assert!(after.content_differs(&before));
        gpui::Empty
    });
}

#[gpui::test]
fn git_field_change_differs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = fixture(window, cx);
        let mut b = fixture(window, cx);
        b.cached_visible = ByPointer(a.cached_visible.clone());
        b.git_op_in_flight = true;
        assert!(
            a.content_differs(&b),
            "a changed git field must trip content_differs"
        );
        gpui::Empty
    });
}

#[gpui::test]
fn lanes_change_differs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = fixture(window, cx);
        let mut b = fixture(window, cx);
        b.cached_visible = ByPointer(a.cached_visible.clone());
        b.lanes.push(crate::lane::Lane::default_for_project(
            0,
            std::path::PathBuf::from("/tmp/scratch"),
        ));
        assert!(
            a.content_differs(&b),
            "a changed lanes vec must trip content_differs"
        );
        gpui::Empty
    });
}

#[gpui::test]
fn distinct_cache_arc_differs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        // Two independently-allocated empty `Arc`s have equal content
        // but distinct pointers — `Arc::ptr_eq` (intentionally) reports
        // them as a difference, modeling a cache rebuild.
        let a = fixture(window, cx);
        let b = fixture(window, cx);
        assert!(
            a.content_differs(&b),
            "distinct cached_visible Arc must trip content_differs (ptr_eq)"
        );
        gpui::Empty
    });
}

#[gpui::test]
fn presentation_size_invalidates_the_cached_dock(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        let a = fixture(window, cx);
        let mut b = fixture(window, cx);
        b.cached_visible = ByPointer(a.cached_visible.clone());
        b.metrics = crate::ui::theme::UiMetrics(daruda_config::UiSize::Large);
        assert!(a.content_differs(&b));
        gpui::Empty
    });
}
