//! History-changing operations — commit / amend / push / pull / fetch.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{AppContext as _, Context, Window};

use crate::surface::strings as app_strings;
use crate::ui::ButtonVariant;
use crate::workspace::dialog_helpers::open_confirm_dialog;
use crate::workspace::{CommitChanges, CommitMode, PushChanges, Workspace};

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

        // In amend mode the primary button (and Cmd+Enter) amends instead of
        // creating a new commit.
        if self.is_amend_mode() {
            self.perform_amend(window, cx);
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
        self.notify_left_dock(cx);

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

    /// Whether the Commit split button is currently in amend mode.
    pub(in crate::workspace) fn is_amend_mode(&self) -> bool {
        matches!(self.commit_mode, CommitMode::Amend { .. })
    }

    /// Dropdown action under the Commit split button. Normal mode → enter
    /// amend mode (see [`Self::enter_amend_mode`]); amend mode → cancel back to
    /// a normal commit (see [`Self::exit_amend_mode`]).
    pub(in crate::workspace) fn on_commit_amend(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        if self.git_op_in_flight {
            return;
        }
        if self.is_amend_mode() {
            self.exit_amend_mode(window, cx);
        } else {
            self.enter_amend_mode(window, cx);
        }
    }

    /// Switch the Commit split button into amend mode. The box text present
    /// when entering is saved on the `Amend` variant so Cancel can restore it.
    /// A non-empty box keeps the user's text and enters immediately; an empty
    /// box first loads the tip commit message (async) and enters once it
    /// arrives. A repo with no commits stays in normal mode and toasts.
    fn enter_amend_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo_root) = self.git_repo_root_for(self.active) else {
            return;
        };

        let current = self.git_commit_input.read(cx).text(cx).to_string();
        if !current.trim().is_empty() {
            // Keep the user's own draft as both the box content and the saved
            // draft, so Cancel returns to exactly this normal-commit state.
            self.set_commit_mode(
                CommitMode::Amend {
                    saved_draft: current,
                },
                cx,
            );
            return;
        }

        // Empty box → load HEAD's message, then enter amend mode in the
        // continuation. The saved draft is empty (Cancel restores an empty box).
        // `wh` recovers a live `&mut Window` for `set_text`.
        let wh = window.window_handle();
        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_head_message(&repo_root),
            move |ws, result, cx| match result {
                Ok(message) if !message.trim().is_empty() => {
                    let input = ws.git_commit_input.clone();
                    if cx
                        .update_window(wh, |_, window, cx| {
                            input.update(cx, |panel, cx_state| {
                                panel.set_text(message.as_str(), window, cx_state)
                            });
                        })
                        .is_err()
                    {
                        // Window closed during async load — input is gone.
                        return;
                    }
                    ws.set_commit_mode(
                        CommitMode::Amend {
                            saved_draft: String::new(),
                        },
                        cx,
                    );
                }
                Ok(_) => {
                    // Tip commit has an empty message (--allow-empty-message) —
                    // nothing useful to prefill, so don't enter amend mode.
                    let report = ErrorReport::new(app_strings::git_amend_load_failed())
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("git.amend.load_failed")
                        .build();
                    ws.report_error(report, cx);
                }
                Err(e) => {
                    // Most commonly: the repo has no commits yet, so there is
                    // nothing to amend. Surface the real git error in details.
                    let report = ErrorReport::new(app_strings::git_amend_load_failed())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("repo", redact_home(&repo_for_report))
                        .dedup("git.amend.load_failed")
                        .build();
                    ws.report_error(report, cx);
                }
            },
        )
        .detach();
    }

    /// Cancel amend mode: restore the box to the draft saved when amend mode
    /// was entered (the user's own message, or empty if we'd prefilled), then
    /// switch back to normal Commit labels — so a user who cancels can commit
    /// their original message without it being wiped. Safe to call
    /// unconditionally; a no-op when not in amend mode. Used by Cancel Amend
    /// and by lane switches.
    pub(in crate::workspace) fn exit_amend_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let CommitMode::Amend { saved_draft } = &self.commit_mode else {
            return;
        };
        let saved_draft = saved_draft.clone();
        self.git_commit_input.update(cx, |panel, cx_state| {
            panel.set_text(saved_draft.as_str(), window, cx_state);
        });
        self.set_commit_mode(CommitMode::Normal, cx);
    }

    /// Set the commit mode and resync the split button: primary label
    /// (Commit ↔ Amend), dropdown label (Amend Last Commit ↔ Cancel Amend),
    /// and the disabled state (amend is allowed with zero staged changes —
    /// a message-only amend — so the disable rule differs by mode).
    fn set_commit_mode(&mut self, mode: CommitMode, cx: &mut Context<Self>) {
        let amend = matches!(mode, CommitMode::Amend { .. });
        self.commit_mode = mode;
        let (primary, dropdown) = if amend {
            (
                app_strings::git_amend_btn(),
                app_strings::git_cancel_amend(),
            )
        } else {
            (
                app_strings::git_commit_btn(),
                app_strings::ctx_git_commit_amend(),
            )
        };
        self.git_commit_input.update(cx, |panel, cx_state| {
            panel.set_action_label("commit", primary, cx_state);
            panel.set_action_dropdown_label("commit", 0, dropdown, cx_state);
        });
        self.sync_commit_buttons(cx);
    }

    /// Perform the amend the user set up in amend mode: validate the (prefilled
    /// or edited) message, confirm the history rewrite, then run the amend.
    fn perform_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.git_commit_input.read(cx).text(cx).to_string();
        if message.trim().is_empty() {
            // User cleared the prefilled message; amend still needs one.
            let report = ErrorReport::new(app_strings::git_amend_needs_message())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("git.amend.needs_message")
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
        self.notify_left_dock(cx);

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
                        // Amend succeeded → leave amend mode (restore the
                        // Commit labels). The box was just cleared above.
                        ws.set_commit_mode(CommitMode::Normal, cx);
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
        self.notify_left_dock(cx);

        let repo_for_report = repo_root.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_push(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                cx.notify();
                ws.notify_left_dock(cx);
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
        self.notify_left_dock(cx);
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
                ws.notify_left_dock(cx);
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
        self.notify_left_dock(cx);
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
                ws.notify_left_dock(cx);
            },
        )
        .detach();
    }
}
