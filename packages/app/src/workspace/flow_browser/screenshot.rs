//! In-memory flow browser fixtures; never write into the restored repository.

use super::{FlowGrouping, FlowScope, FlowTab, listing::FlowListing};
use crate::workspace::{
    Workspace,
    flow_history::{FlowHistory, FlowRunEntry},
    flow_paths::{FlowOrigin, FoundFlow},
    flow_request::FlowSource,
    flow_runs::{ParkedAsk, RunHandle, RunStage},
    pages::Page,
};
use gpui::{Context, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowBrowserShot {
    Definitions,
    Grouped,
    Empty,
    Runs,
    Asking,
}

impl Workspace {
    pub(in crate::workspace) fn seed_flow_browser_for_shot(
        &mut self,
        shot: FlowBrowserShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_flow::marker::RunStatus;
        if self.projects.is_empty() {
            let project = crate::project::Project::bootstrap_placeholder(
                self.next_project_id,
                self.data_dir.clone(),
            );
            self.next_project_id += 1;
            self.set_active(daruda_store::project::LaneRef {
                project: project.id,
                lane: project.lanes[0].id,
            });
            self.projects.push(project);
        }
        self.flows.browser.state.scope = FlowScope::Current;
        let tab = match shot {
            FlowBrowserShot::Runs | FlowBrowserShot::Asking => FlowTab::Runs,
            _ => FlowTab::Definitions,
        };
        self.set_flow_tab(tab, cx);
        self.open_page(Page::Flows, window, cx);
        if let Some(project) = self.project_for_mut(self.active.project) {
            project.name = "daruda".into();
        }
        let root = self.data_dir.join("flow-preview");
        let files: Vec<_> = [
            ("release-validation.yaml", FlowOrigin::Repo),
            (
                "design-review-and-accessibility-check.yaml",
                FlowOrigin::Repo,
            ),
            ("daily-code-review.yaml", FlowOrigin::Project),
            ("documentation-refresh.yaml", FlowOrigin::Project),
            ("security-audit.yaml", FlowOrigin::Global),
            ("triage.yaml", FlowOrigin::Global),
        ]
        .into_iter()
        .map(|(name, origin)| FoundFlow {
            name: name.to_owned(),
            path: root.join(name),
            origin,
        })
        .collect();
        let now = std::time::SystemTime::now();
        let modified = files.iter().map(|file| (file.path.clone(), now)).collect();
        let source = FlowSource::File(files[0].path.clone());
        self.flows
            .list
            .put(self.active, FlowListing { files, modified });
        if shot == FlowBrowserShot::Grouped {
            self.flows.browser.state.grouping = FlowGrouping::Origin;
            self.flows.browser.state.toggle_group(FlowOrigin::Project);
        }
        if shot == FlowBrowserShot::Empty {
            self.flows.browser.searches[tab.index()].update(cx, |input, cx| {
                input.set_value("no matching flow", window, cx)
            });
        }
        let millis = chrono::Utc::now().timestamp_millis() as u128;
        let live_dir = root.join(crate::workspace::flow_request::run_id(millis, 42, 7));
        let stage = if shot == FlowBrowserShot::Asking {
            let (reply, _rx) = smol::channel::bounded(1);
            RunStage::Asking {
                question: std::sync::Arc::new(ParkedAsk::new(daruda_flow::runner::PendingAsk {
                    node: "release-check".into(),
                    attempt: 1,
                    ask_id: 1,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".into(),
                        detail: Some("cargo test -p daruda".into()),
                        options: vec![
                            daruda_acp::PermissionChoice {
                                option_id: "once".into(),
                                name: "Allow once".into(),
                                kind: daruda_acp::PermissionKindView::AllowOnce,
                            },
                            daruda_acp::PermissionChoice {
                                option_id: "reject".into(),
                                name: "Reject".into(),
                                kind: daruda_acp::PermissionKindView::RejectOnce,
                            },
                        ],
                    },
                    reply,
                })),
                queued: Default::default(),
            }
        } else {
            RunStage::Node {
                id: "release-check".into(),
                attempt: 2,
            }
        };
        self.flows.runs.insert(
            self.active,
            RunHandle::seeded(live_dir.clone(), source, stage),
        );
        let history = [
            RunStatus::Running,
            RunStatus::Done,
            RunStatus::Failed,
            RunStatus::Crashed,
            RunStatus::Stalled,
            RunStatus::Canceled,
            RunStatus::Unknown,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, status)| {
            let dir = if index == 0 {
                live_dir.clone()
            } else {
                root.join(crate::workspace::flow_request::run_id(
                    millis - index as u128 * 3600000,
                    42,
                    index as u32,
                ))
            };
            FlowRunEntry {
                dir,
                status,
                report: None,
                started: crate::surface::strings::flow::run_started_at(
                    chrono::Utc::now() - chrono::Duration::hours(index as i64),
                )
                .into(),
            }
        })
        .collect();
        self.flows
            .history
            .put(self.active, FlowHistory::seeded(history));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_states_cover_both_tabs_and_attention() {
        assert_ne!(FlowBrowserShot::Runs, FlowBrowserShot::Asking);
        assert_ne!(FlowBrowserShot::Definitions, FlowBrowserShot::Grouped);
    }
}
