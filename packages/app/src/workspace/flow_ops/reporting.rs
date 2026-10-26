//! Static validation, run navigation, and localized refusal reporting.

use std::path::Path;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use crate::agent::launch_resolve::ConnectCommandError;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::flow_request::FlowSubmitError;

impl Workspace {
    /// Report what can be known without running the flow. Opens no session,
    /// takes no lock, and creates no run directory.
    pub(super) fn validate_flow(
        &mut self,
        lane: daruda_store::project::LaneRef,
        path: &Path,
        profile: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = crate::workspace::flow_paths::read_flow_name(path);
        let (title, body) = match self.check_flow(lane, path, profile, cx) {
            Ok(issues) if issues.is_empty() => (s::flow::valid_title(), s::flow::valid_body(&name)),
            Ok(issues) => (
                s::flow::invalid_title(&name, issues.len()),
                issue_report(&issues),
            ),
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))) => (
                s::flow::invalid_title(&name, issues.len()),
                issue_report(&issues),
            ),
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Parse(detail))) => {
                (s::flow::parse_failed_title(), detail)
            }
            // Not about the flow: no lane, a remote lane, an unreadable
            // file. The same refusal `Run Flow…` would give.
            Err(other) => {
                self.report_flow_refusal(other, cx);
                return;
            }
        };
        crate::workspace::dialog_helpers::open_alert_dialog(
            title,
            body,
            s::common::btn_close(),
            window,
            cx,
        );
    }

    /// Bring the run in `lane` into view: switch to its lane if needed,
    /// open the right dock, and show the Flows tab.
    ///
    /// The panel is lane-scoped, so a question in another lane is answered
    /// *there* — this is the one move that gets you to it. Deliberately a
    /// named op rather than three calls at the click site: the three have to
    /// happen together, and a surface that did two of them would land on the
    /// right lane with the panel still hidden.
    pub(in crate::workspace) fn reveal_flow_run(
        &mut self,
        lane: daruda_store::project::LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active != lane {
            self.activate_lane(lane, window, cx);
        }
        self.set_flow_scope(
            crate::workspace::flow_browser::FlowScope::Worktree(lane),
            cx,
        );
        self.set_flow_tab(crate::workspace::flow_browser::FlowTab::Runs, cx);
        self.flows.browser.state.run_filter = crate::workspace::flow_browser::RunFilter::All;
        self.clear_flow_search(window, cx);
        self.open_page(crate::workspace::pages::Page::Flows, window, cx);
    }

    pub(super) fn report_flow_refusal(&mut self, error: FlowSubmitError, cx: &mut Context<Self>) {
        let (message, detail) = super::flow_refusal_text(error);
        self.report_error(
            ErrorReport::new(message)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("detail", detail)
                .dedup("flow.submit.refused")
                .build(),
            cx,
        );
    }
}

/// A refusal's headline and detail, worded once for every surface that
/// reports one.
pub(in crate::workspace) fn flow_refusal_text(error: FlowSubmitError) -> (String, String) {
    match error {
        FlowSubmitError::NoLane => (s::flow::no_lane(), String::new()),
        FlowSubmitError::LaneUnresolvable { path } => (
            s::flow::lane_unresolvable(path.display().to_string()),
            String::new(),
        ),
        FlowSubmitError::RemoteLane { agent } => (s::flow::remote_lane(&agent), String::new()),
        FlowSubmitError::LockHeld { pid, lock_dir } => (
            s::flow::lock_held(pid),
            lock_dir.map_or_else(String::new, |dir| {
                s::flow::lock_held_detail(dir.join(".lock").display().to_string())
            }),
        ),
        // The reason carries its own already-localized wording — the same
        // one an agent chat pane shows for the identical refusal.
        FlowSubmitError::AgentLaunchRefused { agent, reason } => (
            s::flow::agent_launch_refused(&agent),
            match reason {
                ConnectCommandError::NoRemotePath => s::agent_chat::no_remote_cwd(),
                ConnectCommandError::JsonStdioRemote => {
                    s::agent_chat::json_stdio_remote_unsupported()
                }
                ConnectCommandError::JsonStdioEnv => s::agent_chat::json_stdio_env_unsupported(),
            },
        ),
        FlowSubmitError::UnusableSessionHost { agent, reason } => (
            s::flow::session_host_unusable(&agent),
            daruda_settings::labels::host_error(reason),
        ),
        FlowSubmitError::Read { path, message } => (
            s::flow::read_failed_title(),
            format!("{}: {message}", path.display()),
        ),
        FlowSubmitError::Load(daruda_flow::FlowError::Parse(detail)) => {
            (s::flow::parse_failed_title(), detail)
        }
        FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))
        | FlowSubmitError::Invalid(issues) => (
            s::flow::invalid_title("", issues.len()),
            issue_report(&issues),
        ),
        // The engine's own words: it is the one that decides what can be
        // continued, and a second wording here would be a second answer
        // to the same question.
        FlowSubmitError::Resume(e) => (s::flow::resume_refused(), e.to_string()),
    }
}

/// Every problem, in the wording the user gets. `ValidationIssue.message`
/// is developer detail and deliberately never appears here.
pub(in crate::workspace) fn issue_report(issues: &[daruda_flow::error::ValidationIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            s::flow::issue_line(
                issue.node.as_ref().map(|n| n.as_str()),
                &s::flow::issue(&issue.kind),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use daruda_flow::error::{ValidationIssue, ValidationKind};

    use super::issue_report;

    /// The whole point of the `kind` → wording split: the developer text
    /// carries file paths and internal names, and must not reach a user.
    #[test]
    fn a_report_never_shows_the_developer_message() {
        let issues = vec![ValidationIssue {
            node: Some("gate".into()),
            kind: ValidationKind::Cycle,
            message: "internal detail nobody should read".to_string(),
        }];
        let report = issue_report(&issues);
        assert!(!report.contains("internal detail"), "{report}");
        assert!(report.contains("gate"), "{report}");
    }

    /// Every problem at once — `load` collects within a stage precisely so
    /// an author fixes them in one pass rather than one per run.
    #[test]
    fn the_report_names_every_problem_at_once() {
        let issues = vec![
            ValidationIssue {
                node: Some("a".into()),
                kind: ValidationKind::MissingAgent,
                message: String::new(),
            },
            ValidationIssue {
                node: Some("b".into()),
                kind: ValidationKind::DuplicateOutput,
                message: String::new(),
            },
        ];
        assert_eq!(issue_report(&issues).lines().count(), 2);
    }
}
