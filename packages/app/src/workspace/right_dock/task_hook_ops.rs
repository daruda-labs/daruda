//! Task-specific consumption of terminal observations. Route ownership,
//! pane badges and notifications remain independent of task executions.

use daruda_agent::hooks::status_file::StatusFile;
use gpui::Context;

use crate::runs::RunsHost;
use crate::workspace::Workspace;

impl Workspace {
    pub(in crate::workspace) fn apply_task_hook_observation(
        &mut self,
        file: &StatusFile,
        cx: &mut Context<Self>,
    ) {
        if file.terminal_id.is_some() {
            RunsHost::record_task_cli_hook(file, cx);
        } else {
            self.record_task_cli_transcript(&file.session_id, file.transcript_path.as_deref(), cx);
        }
        self.apply_task_session_changed(&file.session_id, cx);
        if file.last_event == "PostToolUseFailure" {
            self.bump_tool_use_failure(&file.session_id, cx);
        }
        if file.last_event == "PostToolUse"
            && file.tool_name.as_deref() == Some("TodoWrite")
            && let Some(input) = file.tool_input.as_ref()
        {
            self.apply_todo_write(&file.session_id, input, cx);
        }
        if let Some(reason) = crate::runs::cli_hook_end_reason(&file.last_event) {
            let session = file.session_id.clone();
            let terminal = file.terminal_id.as_ref().map(|id| id.as_str().to_owned());
            self.after_task_projection(
                move |workspace, cx| {
                    workspace
                        .claude_sessions
                        .tool_use_failure_counts
                        .remove(&session);
                    RunsHost::settle_cli_terminal(&session, terminal, reason, cx);
                },
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn a_terminal_without_a_task_still_binds_and_updates_its_status(cx: &mut gpui::TestAppContext) {
        let (_, workspace) = crate::workspace::tests::build_workspace(cx);
        cx.update(RunsHost::inline_commits_for_test);
        let directory = tempfile::tempdir().unwrap();
        workspace.update(cx, |workspace, cx| {
            let pane = workspace.active_runtime().focused_pane_id;
            let terminal = crate::hooks::agent_hooks::mint_terminal_id();
            RunsHost::register_terminal(workspace.run_pane(pane), terminal.clone(), cx);
            let mut file = StatusFile::new_hook(
                "unlinked-session",
                directory.path(),
                daruda_agent::SessionStatus::Working,
                "UserPromptSubmit",
            );
            file.terminal_id = Some(terminal.clone());
            let path =
                daruda_agent::hooks::status_file::path_for(directory.path(), file.storage_key());
            daruda_agent::hooks::status_file::write_atomic(&path, &file).unwrap();
            let before = RunsHost::sequence_for_test(cx);
            workspace
                .apply_claude_status_event(crate::hooks::watcher::StatusEvent::Changed(path), cx);
            assert_eq!(RunsHost::sequence_for_test(cx), before);
            assert_eq!(
                workspace.claude_sessions.pty_bindings[&pane].session_id,
                "unlinked-session"
            );
            assert_eq!(
                workspace
                    .claude_sessions
                    .status
                    .get_terminal(&terminal)
                    .unwrap()
                    .status,
                daruda_agent::SessionStatus::Working
            );
        });
    }
}
