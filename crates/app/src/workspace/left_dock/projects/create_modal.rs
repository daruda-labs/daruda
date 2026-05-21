//! Create-lane modal — text input is delegated to the
//! `gpui_component::Input` family (via `crate::ui::input`), so this
//! file owns only the modal-level coordination: builder, two buttons,
//! validation, the workspace finalize handoff, and an Enter-to-submit
//! subscription on each input.
//!
//! Tab / Shift+Tab focus cycling is handled by GPUI itself: each
//! `InputState` focus handle is created with `tab_stop(true)`, and
//! the modal root carries `.tab_group()`. Escape is handled by
//! `Dialog`'s outer Cancel action — the modal no longer wires a
//! panel-level key handler.

use std::path::PathBuf;

use crate::ui::theme;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_primary, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use crate::workspace::lane_ops::{CreateWorktreePlan, sanitize_branch_name};

pub struct CreateWorktreeModal {
    /// Panel focus handle — `.track_focus` target for the modal root
    /// so the dialog's tab group is anchored to a real focusable
    /// element. Not focused itself.
    panel_focus_handle: FocusHandle,
    /// Branch name (required). Submission validates via
    /// `sanitize_branch_name`.
    branch_input: Entity<InputState>,
    /// Base ref to branch from (optional; blank → git's default,
    /// usually current HEAD). Free-form so the user can type a remote
    /// (`origin/main`), local branch, tag, or SHA without us
    /// pre-listing them.
    base_input: Entity<InputState>,
    /// Free-form description (optional). Surfaced in the left dock
    /// row so an idle lane from last week is still self-describing.
    description_input: Entity<InputState>,
    /// Subscriptions to all three inputs — kept alive so PressEnter
    /// + Change events keep flowing into us.
    _input_subscriptions: [Subscription; 3],
    error: Option<SharedString>,
    submitting: bool,
    /// Workspace that finalizes the create on success. Weak so a
    /// closed window doesn't keep the modal around.
    workspace: WeakEntity<Workspace>,
    /// Captured at open time so the modal doesn't have to re-traverse
    /// the lane list to validate.
    repo_root: PathBuf,
}

impl CreateWorktreeModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        repo_root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let branch_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder("branch name (required)")
        });
        let base_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder("base ref — blank = current HEAD")
        });
        let description_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder("description (optional)")
        });

        // Tab order is fully driven by the `tab_index` argument to
        // `crate::ui::input(&state, cx, N)` at render time — GPUI's
        // tab system builds the cycle from `Input::tab_index` baked
        // on the rendered element, not from focus-handle mutations.

        // PressEnter from any field triggers overall submit; Change
        // clears the validation banner so the user sees their edit
        // didn't carry the stale error forward.
        let make_sub = |state: &Entity<InputState>, this_cx: &mut Context<Self>| {
            this_cx.subscribe_in(
                state,
                window,
                |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    InputEvent::Change => {
                        if this.error.is_some() {
                            this.error = None;
                            cx.notify();
                        }
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            )
        };
        let _input_subscriptions = [
            make_sub(&branch_input, cx),
            make_sub(&base_input, cx),
            make_sub(&description_input, cx),
        ];

        Self {
            panel_focus_handle: cx.focus_handle(),
            branch_input,
            base_input,
            description_input,
            _input_subscriptions,
            error: None,
            submitting: false,
            workspace,
            repo_root,
        }
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    /// Pure validation — derives the plan from whatever the inputs
    /// currently hold. `base_ref` / `description` are normalized (trimmed +
    /// blank-to-None). Public for tests; production callers go
    /// through `submit`.
    pub(crate) fn validate(&self, cx: &gpui::App) -> Result<CreateWorktreePlan, String> {
        let raw = self.branch_input.read(cx).value().to_string();
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("Branch name is required.".to_string());
        }
        let branch = sanitize_branch_name(raw).ok_or_else(|| "Invalid branch name.".to_string())?;
        let repo_name = self
            .repo_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");
        let path_suffix = branch.replace('/', "-");
        let new_path = self
            .repo_root
            .parent()
            .unwrap_or(&self.repo_root)
            .join(format!("{repo_name}-{path_suffix}"));

        let base_ref = blank_to_none(&self.base_input.read(cx).value());
        let description = blank_to_none(&self.description_input.read(cx).value());

        Ok(CreateWorktreePlan {
            branch,
            new_path,
            repo_root: self.repo_root.clone(),
            base_ref,
            description,
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let plan = match self.validate(cx) {
            Ok(p) => p,
            Err(msg) => {
                self.error = Some(msg.into());
                cx.notify();
                return;
            }
        };
        self.submitting = true;
        cx.notify();

        let me = cx.entity().downgrade();
        let workspace = self.workspace.clone();
        let git_repo = plan.repo_root.clone();
        let git_path = plan.new_path.clone();
        let git_branch = plan.branch.clone();
        let git_base = plan.base_ref.clone();
        window
            .spawn(cx, async move |async_cx| {
                let result: Result<(), String> = async_cx
                    .background_executor()
                    .spawn(async move {
                        crate::lane::git::add_lane(
                            &git_repo,
                            &git_path,
                            Some(&git_branch),
                            git_base.as_deref(),
                        )
                        .map_err(|e| e.to_string())
                    })
                    .await;

                // SILENT-OK: workspace may drop after create modal closes / dialog dismiss on focus restore
                let _ = async_cx.update(|window, app_cx| {
                    let Some(me) = me.upgrade() else { return };
                    // Nested entity.update calls must not overlap, so
                    // run the workspace finalize first and feed its
                    // outcome back into the modal in a separate step.
                    let final_result: Result<(), String> = match result {
                        Err(msg) => Err(msg),
                        Ok(()) => match workspace.upgrade() {
                            Some(ws) => ws
                                .update(app_cx, |ws, cx| {
                                    ws.finalize_create_lane(plan.clone(), window, cx)
                                })
                                // The left dock opener doesn't need the
                                // newly-spawned pane id — only
                                // start_task does. Discard it.
                                .map(|_pane_id| ()),
                            None => Ok(()),
                        },
                    };
                    me.update(app_cx, |modal, cx| {
                        modal.submitting = false;
                        match final_result {
                            Ok(()) => modal.dismiss(window, cx),
                            Err(msg) => {
                                modal.error = Some(msg.into());
                                cx.notify();
                            }
                        }
                    });
                });
            })
            .detach();
    }
}

/// Trim then collapse `""` → `None`. Used by the optional inputs
/// (base ref, description) to normalize the empty case.
fn blank_to_none(s: &SharedString) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

impl Focusable for CreateWorktreeModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // Delegate to the branch input so initial focus lands on the
        // required field — user can type immediately.
        self.branch_input.focus_handle(cx)
    }
}

impl ModalView for CreateWorktreeModal {}

impl Render for CreateWorktreeModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_text = theme::current(cx).muted_text;
        let body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child("Branch name for the new lane. A sibling directory will be created."),
            )
            .child(input(&self.branch_input, cx, 0))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child("Base ref (optional) — branch from `main`, `origin/main`, etc."),
            )
            .child(input(&self.base_input, cx, 1))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child("Description (optional) — shown in the left dock."),
            )
            .child(input(&self.description_input, cx, 2));

        let create_disabled = self.submitting || self.branch_input.read(cx).value().is_empty();
        let submit_label = if self.submitting {
            "Creating…"
        } else {
            "Create"
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(button("create-wt-cancel", "Cancel").on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| {
                    this.dismiss(window, cx);
                },
            )))
            .child(
                button_primary("create-wt-submit", submit_label)
                    .disabled(create_disabled)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    })),
            );

        let mut panel = div()
            .flex()
            .flex_col()
            .key_context("CreateWorktreeModal")
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(banner) = self
            .error
            .as_ref()
            .map(|msg| crate::ui::alert::error("create-lane-error", msg.clone()))
        {
            panel = panel.child(banner);
        }
        panel.child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::init_gpui_component;
    use gpui::{TestAppContext, WindowHandle};

    fn build_modal(
        cx: &mut TestAppContext,
        repo_root: &str,
    ) -> (
        WindowHandle<CreateWorktreeModal>,
        Entity<CreateWorktreeModal>,
    ) {
        init_gpui_component(cx);
        let wh = cx.add_window(|window, cx| {
            CreateWorktreeModal::new(
                WeakEntity::new_invalid(),
                PathBuf::from(repo_root),
                window,
                cx,
            )
        });
        let modal = wh.root(cx).unwrap();
        (wh, modal)
    }

    fn set_field(
        wh: &WindowHandle<CreateWorktreeModal>,
        modal: &Entity<CreateWorktreeModal>,
        cx: &mut TestAppContext,
        field: fn(&CreateWorktreeModal) -> Entity<InputState>,
        s: &str,
    ) {
        let state = modal.read_with(cx, |m, _| field(m));
        // SILENT-OK: workspace may drop after create modal closes / dialog dismiss on focus restore
        let _ = wh.update(cx, |_root, window, cx| {
            state.update(cx, |i, cx_state| {
                i.set_value(s.to_string(), window, cx_state);
            });
        });
    }

    fn set_branch(
        wh: &WindowHandle<CreateWorktreeModal>,
        modal: &Entity<CreateWorktreeModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.branch_input.clone(), s);
    }

    fn set_base(
        wh: &WindowHandle<CreateWorktreeModal>,
        modal: &Entity<CreateWorktreeModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.base_input.clone(), s);
    }

    fn set_description(
        wh: &WindowHandle<CreateWorktreeModal>,
        modal: &Entity<CreateWorktreeModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.description_input.clone(), s);
    }

    #[gpui::test]
    fn validate_rejects_empty_input(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, "/repo");
        modal.read_with(cx, |m, cx| {
            let err = m.validate(cx).unwrap_err();
            assert!(err.contains("required"));
        });
    }

    #[gpui::test]
    fn validate_accepts_valid_branch(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/Users/dev/repo");
        set_branch(&wh, &modal, cx, "feat/sidebar");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.branch, "feat/sidebar");
            assert_eq!(
                plan.new_path.to_string_lossy(),
                "/Users/dev/repo-feat-sidebar"
            );
        });
    }

    #[gpui::test]
    fn validate_rejects_invalid_chars(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/repo");
        set_branch(&wh, &modal, cx, "has space");
        modal.read_with(cx, |m, cx| {
            let err = m.validate(cx).unwrap_err();
            assert!(err.contains("Invalid"));
        });
    }

    #[gpui::test]
    fn validate_captures_base_ref(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/Users/dev/repo");
        set_branch(&wh, &modal, cx, "feat/x");
        set_base(&wh, &modal, cx, "origin/main");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.base_ref.as_deref(), Some("origin/main"));
        });
    }

    #[gpui::test]
    fn validate_blank_base_normalizes_to_none(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/repo");
        set_branch(&wh, &modal, cx, "feat/x");
        set_base(&wh, &modal, cx, "   ");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert!(plan.base_ref.is_none());
        });
    }

    #[gpui::test]
    fn validate_captures_description(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/repo");
        set_branch(&wh, &modal, cx, "feat/x");
        set_description(&wh, &modal, cx, "PR #123 review");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.description.as_deref(), Some("PR #123 review"));
        });
    }
}
