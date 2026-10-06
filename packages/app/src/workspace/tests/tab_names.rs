//! Naming a tab: the name the tab strip shows, and the one the phone sees in
//! a remote message about an agent running in it.

use super::*;

fn with_agent_tab(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    u64,
    crate::workspace::main_area::pane_tree::PaneId,
) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let (tab_id, pane_id) = ws.read_with(cx, |ws, _| {
        let rt = ws.active_runtime();
        (rt.tabs[rt.active_tab_index].id, rt.focused_pane_id)
    });
    (wh, ws, tab_id, pane_id)
}

fn active_label(ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext) -> String {
    ws.read_with(cx, |ws, cx| {
        let rt = ws.active_runtime();
        ws.tab_label(&rt.tabs[rt.active_tab_index], cx)
            .unwrap_or_default()
            .to_string()
    })
}

#[gpui::test]
async fn a_named_tab_shows_its_name(cx: &mut TestAppContext) {
    let (_wh, ws, tab_id, _pane) = with_agent_tab(cx);
    let default = active_label(&ws, cx);
    ws.update(cx, |ws, cx| {
        ws.rename_tab(tab_id, Some("review".into()), cx)
    });
    assert_eq!(active_label(&ws, cx), "review");

    ws.update(cx, |ws, cx| ws.rename_tab(tab_id, None, cx));
    assert_eq!(
        active_label(&ws, cx),
        default,
        "clearing the name restores the default"
    );
}

#[gpui::test]
async fn only_a_given_name_reaches_the_phone(cx: &mut TestAppContext) {
    let (_wh, ws, tab_id, pane) = with_agent_tab(cx);
    let quiet = ws.read_with(cx, |ws, _| ws.pane_tab_name(pane));
    assert_eq!(quiet, None, "the default label is not a name the user gave");
    ws.update(cx, |ws, cx| {
        ws.rename_tab(tab_id, Some("review".into()), cx)
    });
    let named = ws.read_with(cx, |ws, _| ws.pane_tab_name(pane));
    assert_eq!(named.as_deref(), Some("review"));
}

#[gpui::test]
async fn the_remote_header_names_the_tab_beside_the_agent(cx: &mut TestAppContext) {
    let (_wh, ws, tab_id, pane) = with_agent_tab(cx);
    let (before, agent) = ws.update(cx, |ws, cx| {
        let agent = ws
            .agent_chat_view(pane)
            .unwrap()
            .read(cx)
            .agent_name
            .clone();
        (ws.telegram_header(pane, cx), agent)
    });
    ws.update(cx, |ws, cx| {
        ws.rename_tab(tab_id, Some("review".into()), cx)
    });
    let after = ws.update(cx, |ws, cx| ws.telegram_header(pane, cx));
    assert_eq!(
        after,
        before.replace(
            &agent,
            &crate::surface::strings::control::agent_with_tab(&agent, "review")
        ),
        "only the agent line gains the tab name"
    );
}

#[gpui::test]
async fn a_pane_in_a_parked_lane_still_knows_its_tab_name(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path().join("gone"));
    let (wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let (tab_id, pane) = ws.read_with(cx, |ws, _| {
        let rt = ws.active_runtime();
        (rt.tabs[rt.active_tab_index].id, rt.focused_pane_id)
    });
    ws.update(cx, |ws, cx| {
        ws.rename_tab(tab_id, Some("parked".into()), cx)
    });
    let other = ws.update(cx, |ws, _| {
        let project = ws.active.project;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, dir.path().join("other"));
        lane.tab_order = 1;
        ws.project_for_mut(project).unwrap().lanes.push(lane);
        daruda_store::project::LaneRef {
            project,
            lane: lane_id,
        }
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(other, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let name = ws.read_with(cx, |ws, _| ws.pane_tab_name(pane));
    assert_eq!(name.as_deref(), Some("parked"));
}
