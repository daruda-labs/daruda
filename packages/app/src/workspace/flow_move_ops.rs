//! Moving a flow into its worktree's `.daruda/flows/`: the deliberate step
//! `create_flow_in` leaves to the person instead of writing there by default.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::LaneRef;
use gpui::{App, Context, WeakEntity, Window};

use super::Workspace;
use super::flow_paths::{FlowOrigin, flow_label, flows_dir, origin_label};
use crate::surface::strings as s;

/// Where a move would put a flow, and what the moved copy would hide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowMovePlan {
    pub to: PathBuf,
    pub origin: FlowOrigin,
    /// The other scopes holding a flow of the same file name. The repository
    /// is the narrowest scope, so the moved copy wins that name over each.
    pub shadows: Vec<FlowOrigin>,
}

impl Workspace {
    /// What moving `path` into `lane`'s repository would do; `None` when the
    /// flow is already there, or is not one of the lane's flow directories'.
    pub(in crate::workspace) fn flow_move_plan(
        &self,
        lane: LaneRef,
        path: &Path,
    ) -> Option<FlowMovePlan> {
        let sources = self.flow_sources_for(lane)?;
        let dirs = sources.dirs();
        let from_dir = path.parent()?;
        let file_name = path.file_name()?;
        let (_, origin) = dirs
            .iter()
            .find(|(dir, _)| daruda_core::path::same_path(dir, from_dir))?;
        if *origin == FlowOrigin::Repo {
            return None;
        }
        let shadows = dirs
            .iter()
            .filter(|(_, other)| *other != FlowOrigin::Repo && other != origin)
            .filter(|(dir, _)| dir.join(file_name).is_file())
            .map(|(_, other)| *other)
            .collect();
        Some(FlowMovePlan {
            to: flows_dir(&sources.lane).join(file_name),
            origin: *origin,
            shadows,
        })
    }

    /// Copy the flow at `from` into `lane`'s repository, then remove the
    /// original, keeping an open graph of it pointed at the copy.
    ///
    /// The copy is written with `create_new`, so a repository flow of the
    /// same name is never overwritten. Run history needs no change: runs
    /// belong to the worktree and `run.yaml` holds the resolved spec.
    pub(in crate::workspace) fn move_flow_to_repository(
        &mut self,
        lane: LaneRef,
        from: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(plan) = self.flow_move_plan(lane, from) else {
            return;
        };
        let text = match std::fs::read_to_string(from) {
            Ok(text) => text,
            Err(e) => return self.report_flow_move_error(from, &e, cx),
        };
        if let Some(dir) = plan.to.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            return self.report_flow_move_error(dir, &e, cx);
        }
        if let Err(e) = super::flow_file_ops::write_new_file(&plan.to, &text) {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                self.report_error(
                    ErrorReport::new(s::flow::move_failed_title())
                        .severity(ErrorSeverity::Warning)
                        .message(s::flow::move_taken(flow_label(&plan.to)))
                        .dedup("flow.move_taken")
                        .at(file!(), line!())
                        .build(),
                    cx,
                );
                return;
            }
            return self.report_flow_move_error(&plan.to, &e, cx);
        }
        // The copy already wins the name, so a failed removal leaves a
        // shadowed duplicate rather than a lost flow; it is reported and the
        // graph still follows the copy that runs.
        if let Err(e) = std::fs::remove_file(from) {
            self.report_flow_move_error(from, &e, cx);
        }
        // A shared flow can be open for another lane, where the copy in this
        // lane's repository is not one of its flows: that graph has lost its
        // file, not moved with it.
        match self.open_graph_of(from, cx) {
            Some((_, open_lane, _)) if open_lane == lane => {
                self.repoint_flow_graph(from, &plan.to, window, cx);
            }
            Some((_, _, view)) => view.update(cx, |view, cx| view.report_file_gone(cx)),
            None => {}
        }
        self.invalidate_flow_list();
        // The repository's flows directory may have just been created, and
        // the watcher can only anchor on one that exists.
        self.respawn_flow_watcher(cx);
        cx.notify();
    }

    /// The palette's way in: the open graph's flow, when it is not already
    /// the repository's.
    pub(in crate::workspace) fn on_move_flow_to_repository(
        &mut self,
        _: &super::MoveFlowToRepository,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, lane, view)) = self.open_graph() else {
            return;
        };
        let path = view.read(cx).path().to_path_buf();
        self.ask_before_moving(lane, path, window, cx);
    }

    /// Ask, then move on yes. The plan is made from `self` before the dialog
    /// opens: every caller is already inside this workspace's update, where
    /// reading it again through its entity would panic.
    pub(in crate::workspace) fn ask_before_moving(
        &self,
        lane: LaneRef,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(plan) = self.flow_move_plan(lane, &path) else {
            return;
        };
        let weak = cx.entity().downgrade();
        open_move_dialog(lane, path, &plan, weak, window, cx);
    }

    fn report_flow_move_error(
        &mut self,
        path: &Path,
        error: &std::io::Error,
        cx: &mut Context<Self>,
    ) {
        self.report_error(
            ErrorReport::new(s::flow::move_failed_title())
                .severity(ErrorSeverity::Error)
                .message(s::flow::file_op_failed(
                    path.display().to_string(),
                    error.to_string(),
                ))
                .dedup("flow.file_op")
                .at(file!(), line!())
                .build(),
            cx,
        );
    }
}

/// The question itself, for a plan already made. Its own funnel so the
/// screenshot scenario shows the dialog a person gets.
pub(in crate::workspace) fn open_move_dialog(
    lane: LaneRef,
    path: PathBuf,
    plan: &FlowMovePlan,
    ws: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    crate::ui::dialog::open_confirm_dialog(
        s::flow::move_confirm_title(),
        move_confirm_body(&flow_label(&path), plan),
        s::flow::move_confirm_ok(),
        crate::ui::dialog::ButtonVariant::Primary,
        move |_, window, app| {
            let path = path.clone();
            if let Some(ws) = ws.upgrade() {
                ws.update(app, |ws, cx| {
                    ws.move_flow_to_repository(lane, &path, window, cx)
                });
            }
        },
        window,
        cx,
    );
}

fn move_confirm_body(name: &str, plan: &FlowMovePlan) -> String {
    let mut body = s::flow::move_confirm_body(name, origin_label(plan.origin));
    for shadowed in &plan.shadows {
        body.push(' ');
        body.push_str(&s::flow::move_confirm_shadows(origin_label(*shadowed)));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_names_every_scope_the_copy_will_hide() {
        let plan = FlowMovePlan {
            to: PathBuf::from("/lane/.daruda/flows/ship.yaml"),
            origin: FlowOrigin::Global,
            shadows: vec![FlowOrigin::Project],
        };
        let body = move_confirm_body("ship.yaml", &plan);
        assert!(body.starts_with(&s::flow::move_confirm_body(
            "ship.yaml",
            origin_label(FlowOrigin::Global)
        )));
        assert!(body.ends_with(&s::flow::move_confirm_shadows(origin_label(
            FlowOrigin::Project
        ))));
    }
}
