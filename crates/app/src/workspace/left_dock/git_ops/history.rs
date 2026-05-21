//! History-changing operations — commit / amend / push / pull / fetch.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{AppContext as _, Context, Window};

use crate::surface::strings as app_strings;
use crate::ui::ButtonVariant;
use crate::workspace::dialog_helpers::open_confirm_dialog;
use crate::workspace::{CommitChanges, PushChanges, Workspace};

impl Workspace {
    /// Commit staged changes with the current commit-message input text.
    /// Validates the message, then opens a confirm dialog summarising the
    /// commit. The actual git operation runs in [`Self::do_commit_changes`] only
    /// after the user confirms.
    pub(in crate::workspace) fn on_commit_changes(
        &mut self,
        _: &CommitChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_op_in_flight {
            return;
        }

        let message = {
            let panel = self.git_commit_input.read(cx);
            panel.text(cx).to_string()
        };
        if message.trim().is_empty() {
            let report = ErrorReport::new("Commit message cannot be empty")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("git.commit.empty_message")
                .build();
            self.report_error(report, cx);
            return;
        }

        let staged_count = self
            .git_status_cache
            .get(&self.active)
            .map(|s| s.staged.len())
            .unwrap_or(0);
        let first_line = message.lines().next().unwrap_or("").to_string();
        let body = format!("{staged_count} file(s) staged — {first_line}");

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::git_confirm_commit_title(),
            body,
            app_strings::git_confirm_commit_ok(),
            ButtonVariant::Primary,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let message = message.clone();
                    let wh = window.window_handle();
                    ws.update(app_cx, |ws, cx| ws.do_commit_changes(message, wh, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Run `git commit -m <message>` in the background. Caller must have
    /// already validated the message and obtained user confirmation.
    /// `wh` is captured so the post-commit `set_text("")` on the commit
    /// input can recover a live `&mut Window` after the async git call
    /// returns.
    fn do_commit_changes(
        &mut self,
        message: String,
        wh: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(repo_root) = self.git_repo_root_for(self.active) else {
            return;
        };
        let active_ref = self.active;

        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();

        let message_bg = message.clone();
        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_commit(&repo_root, &message_bg),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                match result {
                    Ok(()) => {
                        let input = ws.git_commit_input.clone();
                        if cx
                            .update_window(wh, |_, window, cx| {
                                input.update(cx, |panel, cx_state| {
                                    panel.set_text("", window, cx_state)
                                });
                            })
                            .is_err()
                        {
                            // Window closed during async commit — input no longer exists.
                        }
                        ws.refresh_git_status(active_ref, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git commit failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("repo", redact_home(&repo_for_report))
                            .dedup("git.commit")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
            },
        )
        .detach();
    }

    /// Amend the last commit with the current staged changes and the given
    /// message. Opens a confirm dialog warning about history rewrite before
    /// the actual amend runs in [`Self::do_commit_amend`].
    pub(in crate::workspace) fn on_commit_amend(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        if self.git_op_in_flight {
            return;
        }

        let message = {
            let panel = self.git_commit_input.read(cx);
            panel.text(cx).to_string()
        };
        if message.trim().is_empty() {
            let report = ErrorReport::new("Commit message cannot be empty")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("git.amend.empty_message")
                .build();
            self.report_error(report, cx);
            return;
        }

        if self.git_repo_root_for(self.active).is_none() {
            return;
        }

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::git_confirm_amend_title(),
            app_strings::git_confirm_amend_body(),
            app_strings::git_confirm_amend_ok(),
            ButtonVariant::Danger,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let message = message.clone();
                    let wh = window.window_handle();
                    ws.update(app_cx, |ws, cx| ws.do_commit_amend(message, wh, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Run `git commit --amend -m <message>` in the background. Caller must
    /// have obtained user confirmation. `wh` is captured so the
    /// post-amend `set_text("")` on the commit input recovers a live
    /// `&mut Window` after the async git call.
    fn do_commit_amend(
        &mut self,
        message: String,
        wh: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(repo_root) = self.git_repo_root_for(self.active) else {
            return;
        };
        let active_ref = self.active;

        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();

        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_commit_amend(&repo_root, &message),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                match result {
                    Ok(()) => {
                        let input = ws.git_commit_input.clone();
                        if cx
                            .update_window(wh, |_, window, cx| {
                                input.update(cx, |panel, cx_state| {
                                    panel.set_text("", window, cx_state)
                                });
                            })
                            .is_err()
                        {
                            // Window closed during async amend — input no longer exists.
                        }
                        ws.refresh_git_status(active_ref, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git commit --amend failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("repo", redact_home(&repo_for_report))
                            .dedup("git.amend")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
            },
        )
        .detach();
    }

    /// Push the current branch to the remote (no action struct needed — for
    /// direct calls from left-dock render closures that can't import `PushChanges`).
    pub(in crate::workspace) fn trigger_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_push_changes(&PushChanges, window, cx);
    }

    /// Push the current branch to the remote. Opens a confirm dialog before
    /// running the actual push — the git operation lives in [`Self::do_push`].
    pub(in crate::workspace) fn on_push_changes(
        &mut self,
        _: &PushChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_op_in_flight {
            return;
        }
        if self.git_repo_root_for(self.active).is_none() {
            return;
        }

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::git_confirm_push_title(),
            app_strings::git_confirm_push_body(),
            app_strings::git_confirm_push_ok(),
            ButtonVariant::Primary,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(app_cx, |ws, cx| ws.do_push(cx));
                }
            },
            window,
            cx,
        );
    }

    /// Run `git push` in the background. Caller must have obtained user
    /// confirmation.
    fn do_push(&mut self, cx: &mut Context<Self>) {
        let Some(repo_root) = self.git_repo_root_for(self.active) else {
            return;
        };

        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();

        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_push(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                cx.notify();
                if let Err(e) = result {
                    let report = ErrorReport::new("git push failed")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("repo", redact_home(&repo_for_report))
                        .dedup("git.push")
                        .build();
                    ws.report_error(report, cx);
                }
            },
        )
        .detach();
    }

    /// Fetch from all remotes.
    pub(in crate::workspace) fn on_fetch(&mut self, cx: &mut Context<Self>) {
        if self.git_op_in_flight {
            return;
        }
        let Some(repo_root) = self.git_repo_root_for(self.active) else {
            return;
        };
        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();
        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_fetch(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                if let Err(e) = result {
                    let report = ErrorReport::new("git fetch failed")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("repo", redact_home(&repo_for_report))
                        .dedup("git.fetch")
                        .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Pull from the remote tracking branch.
    pub(in crate::workspace) fn on_pull(&mut self, cx: &mut Context<Self>) {
        if self.git_op_in_flight {
            return;
        }
        let Some(repo_root) = self.git_repo_root_for(self.active) else {
            return;
        };
        let active_ref = self.active;
        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();
        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_pull(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(active_ref, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git pull failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("repo", redact_home(&repo_for_report))
                            .dedup("git.pull")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }
}
