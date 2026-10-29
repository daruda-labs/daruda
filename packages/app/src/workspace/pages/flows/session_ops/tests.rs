use super::*;
use crate::workspace::tests::build_workspace_with;
use daruda_chat::view::AgentSessionStatus;
use daruda_flow::session::{SessionAccount, SessionBinding};
use gpui::{AppContext as _, TestAppContext};

#[gpui::test]
fn continuation_preserves_identity_reuses_exact_panes_and_returns_to_evidence(
    cx: &mut TestAppContext,
) {
    let root = tempfile::tempdir().unwrap();
    let dir = super::super::execution_fixture::seed(
        root.path(),
        super::super::execution_fixture::ExecutionShot::Completed,
    );
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update(Workspace::fake_agent_connections_for_test);
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    let origin = workspace.update_in(&mut vcx, |ws, window, cx| {
        let origin = RunOrigin {
            lane: ws.active,
            dir: dir.clone(),
            node: "prepare".into(),
            evidence: 1,
        };
        let binding = SessionBinding {
            node: origin.node.clone(),
            evidence: 1,
            attempt: 1,
            agent: "claude".into(),
            account: Some(SessionAccount { managed: None }),
            cwd: root.path().into(),
            session_id: "exact-flow-session".into(),
        };
        ws.attach_flow_session(origin.clone(), binding.clone(), window, cx);
        let pane = ws.active_runtime().focused_pane_id;
        assert!(ws.requires_exact_resume(pane, cx));
        let view = ws.agent_chat_view(pane).unwrap().clone();
        view.update(cx, |view, cx| {
            view.set_session_id_for_test(Some(binding.session_id.clone()));
            view.set_status_for_test(AgentSessionStatus::Connected);
            cx.notify();
        });
        let count = ws.active_runtime().panes.len();
        ws.attach_flow_session(origin.clone(), binding.clone(), window, cx);
        assert_eq!(ws.active_runtime().panes.len(), count);
        assert_eq!(ws.active_runtime().focused_pane_id, pane);
        let other_cwd = root.path().join("other");
        std::fs::create_dir_all(&other_cwd).unwrap();
        ws.attach_flow_session(
            origin.clone(),
            SessionBinding {
                cwd: other_cwd,
                ..binding.clone()
            },
            window,
            cx,
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            count + 1,
            "a bare session ID cannot identify a pane"
        );
        ws.attach_flow_session(origin.clone(), binding.clone(), window, cx);
        assert!(
            ws.active_runtime().tabs[ws.active_runtime().active_tab_index]
                .layout
                .contains(pane)
        );
        ws.attach_flow_session(
            origin.clone(),
            SessionBinding {
                cwd: std::env::temp_dir(),
                ..binding.clone()
            },
            window,
            cx,
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            count + 1,
            "a moved execution cannot attach outside its Worktree"
        );
        ws.attach_flow_session(
            origin.clone(),
            SessionBinding {
                account: Some(SessionAccount {
                    managed: Some(uuid::Uuid::nil().to_string()),
                }),
                ..binding
            },
            window,
            cx,
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            count + 1,
            "missing accounts never fall back to system"
        );
        ws.open_flow_run_origin(origin.clone(), window, cx);
        origin
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Flows));
        let detail = ws.pages.flows.detail.as_ref().unwrap();
        assert_eq!(detail.lane, origin.lane);
        let FlowDetailBody::Run(run) = &detail.body else {
            panic!("run detail");
        };
        assert_eq!(run.dir, origin.dir);
        assert_eq!(
            run.execution.inspector.selection().cloned(),
            Some(ExecutionSelection::Attempt(origin.node, origin.evidence))
        );
    });
}

#[gpui::test]
fn exact_loading_survives_workspace_restore_without_persisting_navigation(cx: &mut TestAppContext) {
    use crate::workspace::main_area::{
        pane::{SessionRestore, TabEntry},
        pane_tree::PaneLayout,
    };
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let config = daruda_config::Config::default();
    let (window, workspace) = build_workspace_with(cx, &config, Some(project));
    let (state, projects) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let mut pane = ws.create_agent_chat_pane(
                    Some(daruda_store::project::PaneCwd::Local(root.path().into())),
                    Some("saved-flow-session".into()),
                    "claude".into(),
                    None,
                    window,
                    cx,
                );
                pane.agent_chat_content_mut().unwrap().restore = SessionRestore::Exact {
                    flow: Some(RunOrigin {
                        lane: ws.active,
                        dir: root.path().into(),
                        node: "prepare".into(),
                        evidence: 1,
                    }),
                };
                let pane_id = pane.id;
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().panes.push(pane);
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
                ws.snapshot_for_disk(cx)
            })
        })
        .unwrap();
    let (restored_window, restored) = build_workspace_with(cx, &config, None);
    cx.update_window(restored_window.into(), |_, window, cx| {
        restored.update(cx, |ws, cx| {
            ws.restore_from_disk(&state, &projects, window, cx);
            let chat = ws
                .active_runtime()
                .panes
                .iter()
                .find(|p| p.agent_chat_content().is_some())
                .unwrap();
            assert!(ws.requires_exact_resume(chat.id, cx));
            assert!(
                chat.agent_chat_content()
                    .unwrap()
                    .restore
                    .flow_origin()
                    .is_none()
            );
            assert_eq!(
                chat.agent_chat_view().unwrap().read(cx).session_id(),
                Some("saved-flow-session")
            );
        })
    })
    .unwrap();
}

#[gpui::test]
fn continuation_rechecks_binding_and_rejects_a_result_after_navigation(cx: &mut TestAppContext) {
    use super::super::execution_fixture::{ExecutionShot, seed};
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Completed);
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update(Workspace::fake_agent_connections_for_test);
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(crate::workspace::pages::Page::Flows, window, cx);
        ws.open_run_detail(ws.active, &dir, window, cx);
    });
    vcx.run_until_parked();
    workspace.update_in(&mut vcx, |ws, window, cx| {
        let id = ws.pages.flows.detail.as_ref().unwrap().id;
        let request = ws.prepare_flow_session(id).unwrap();
        let origin = request.origin.clone();
        let binding = request
            .revalidate()
            .expect("recorded binding still matches");
        let mut changed = binding.clone();
        changed.session_id = "different-session".into();
        assert!(
            ContinuationRequest {
                origin: origin.clone(),
                binding: changed
            }
            .revalidate()
            .is_none()
        );
        let panes = ws.active_runtime().panes.len();
        ws.select_execution_node(id, "review".into(), cx);
        ws.complete_flow_session(id, origin.clone(), Some(binding.clone()), window, cx);
        assert_eq!(ws.active_runtime().panes.len(), panes);
        assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Flows));
        ws.install_run_detail(ws.active, dir.clone(), cx);
        ws.complete_flow_session(id, origin, Some(binding), window, cx);
        assert_eq!(ws.active_runtime().panes.len(), panes);
        assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Flows));
    });
}
