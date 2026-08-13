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

use super::modal_shared::{field_label, session_host_error_to_msg};
use crate::lane::session_host;
use crate::surface::strings as s;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::select::{self, SelectOption, SelectState};
use crate::ui::{InputEvent, InputState, button, button_primary, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use crate::workspace::lane_ops::CreateWorktreePlan;
use daruda_config::SessionHostEntry;
use daruda_core::git::sanitize_branch_name;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::project::{LaneSessionHost, ProjectId};

/// Registry-select sentinel for "no host" — a fresh lane has no "keep
/// current" case (unlike `SessionHostModal`'s `KEEP_CURRENT_SELECT_VALUE`),
/// so the dropdown is just Local + the catalog.
const LOCAL_SELECT_VALUE: &str = "local";

/// The registry dropdown's option list for the create form: Local first
/// (the default), then every catalog entry keyed by its id — no "keep
/// current" entry, since there is no existing lane to preserve.
fn host_select_options(catalog: &[SessionHostEntry]) -> Vec<SelectOption> {
    let mut opts = Vec::with_capacity(catalog.len() + 1);
    opts.push(SelectOption::new(
        LOCAL_SELECT_VALUE,
        s::session_host_option_local(),
    ));
    opts.extend(
        catalog
            .iter()
            .map(|entry| SelectOption::new(entry.id.as_inner().to_string(), entry.label.clone())),
    );
    opts
}

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
    /// Registry host dropdown — Local (default) plus every entry in
    /// `catalog`. See `host_select_options`.
    host_select: Entity<SelectState>,
    /// Working directory on the picked host. Only consulted (and shown)
    /// when `host_select`'s current value names a catalog entry.
    session_path_input: Entity<InputState>,
    /// Subscriptions to all four text inputs — kept alive so PressEnter
    /// + Change events keep flowing into us.
    _input_subscriptions: [Subscription; 4],
    /// Clears the validation banner and re-renders (to show/hide the
    /// session-path field) whenever the dropdown selection changes.
    _host_select_sub: Subscription,
    error: Option<SharedString>,
    submitting: bool,
    /// Workspace that finalizes the create on success. Weak so a
    /// closed window doesn't keep the modal around.
    workspace: WeakEntity<Workspace>,
    /// Captured at open time so the modal doesn't have to re-traverse
    /// the lane list to validate.
    repo_root: PathBuf,
    /// Project this lane will be created under, captured at modal-open
    /// time from the [+] button's row context. Immutable for the modal's
    /// lifetime so submit always targets the intended project regardless
    /// of which project is focused when the user clicks Create.
    project_id: ProjectId,
    /// The workspace's `session_hosts` registry catalog, snapshotted at
    /// modal-open time — same one-time-snapshot shape as `SessionHostModal`.
    catalog: Vec<SessionHostEntry>,
}

impl CreateWorktreeModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        repo_root: PathBuf,
        project_id: ProjectId,
        catalog: Vec<SessionHostEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let branch_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::create_lane_placeholder_branch_name())
        });
        let base_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::create_lane_placeholder_base_ref())
        });
        let description_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::create_lane_placeholder_description())
        });
        let host_select = cx.new(|cx_state| {
            select::state_with_options(
                host_select_options(&catalog),
                Some(&SharedString::from(LOCAL_SELECT_VALUE)),
                window,
                cx_state,
            )
        });
        let session_path_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::session_host_placeholder_session_path())
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
            make_sub(&session_path_input, cx),
        ];
        // Selecting a registry entry (or Local) both clears a stale
        // validation banner and re-renders — the session-path field's
        // visibility follows the current selection.
        let _host_select_sub = cx.subscribe_in(
            &host_select,
            window,
            |this, _, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.error = None;
                    cx.notify();
                }
            },
        );

        Self {
            panel_focus_handle: cx.focus_handle(),
            branch_input,
            base_input,
            description_input,
            host_select,
            session_path_input,
            _input_subscriptions,
            _host_select_sub,
            error: None,
            submitting: false,
            workspace,
            repo_root,
            project_id,
            catalog,
        }
    }

    /// The catalog entry `host_select` currently points at — `None` for
    /// Local, so the session-path field only appears while a registry
    /// entry is selected.
    fn selected_entry(&self, cx: &App) -> Option<&SessionHostEntry> {
        let value = self.host_select.read(cx).selected_value()?.to_string();
        self.catalog
            .iter()
            .find(|entry| entry.id.as_inner().to_string() == value)
    }

    /// Resolve `host_select` + `session_path_input` into the plan's
    /// `session_host`. `None` for Local (the default) — validation and
    /// quoting-safety are delegated to `session_host::sanitized_ssh`/
    /// `sanitized_docker`, never reimplemented here.
    fn build_session_host(&self, cx: &App) -> Result<Option<LaneSessionHost>, String> {
        let value = self
            .host_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string());
        match value.as_deref() {
            None | Some(LOCAL_SELECT_VALUE) => Ok(None),
            Some(_) => {
                let Some(entry) = self.selected_entry(cx) else {
                    // Unreachable in practice: every non-Local option value
                    // is minted from `self.catalog` itself in
                    // `host_select_options`. Fall back to Local rather than
                    // error out.
                    return Ok(None);
                };
                let path = self.session_path_input.read(cx).value().to_string();
                let host = session_host::from_registry_entry(entry, &path)
                    .map_err(session_host_error_to_msg)?;
                Ok(Some(host))
            }
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
            return Err(s::create_lane_err_branch_required());
        }
        let branch = sanitize_branch_name(raw).ok_or_else(s::create_lane_err_branch_invalid)?;
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
        let session_host = self.build_session_host(cx)?;

        Ok(CreateWorktreePlan {
            branch,
            new_path,
            repo_root: self.repo_root.clone(),
            base_ref,
            description,
            session_host,
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let mut plan = match self.validate(cx) {
            Ok(p) => p,
            Err(msg) => {
                self.error = Some(msg.into());
                cx.notify();
                return;
            }
        };
        // Resolve base_ref against the project at submit time. Best-effort:
        // if the workspace is gone we proceed with the raw input and
        // finalize_create_lane will reject if the project itself is gone.
        if let Some(ws) = self.workspace.upgrade() {
            plan.base_ref = ws
                .read(cx)
                .resolve_lane_base_ref(std::mem::take(&mut plan.base_ref));
        }
        let project_id = self.project_id;
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

                let update_result = async_cx.update(|window, app_cx| {
                    let Some(me) = me.upgrade() else { return };
                    // Nested entity.update calls must not overlap, so
                    // run the workspace finalize first and feed its
                    // outcome back into the modal in a separate step.
                    let final_result: Result<(), String> = match result {
                        Err(msg) => Err(msg),
                        Ok(()) => match workspace.upgrade() {
                            Some(ws) => ws
                                .update(app_cx, |ws, cx| {
                                    // Manual lane creation always spawns a
                                    // terminal — the Agent-chat surface is a
                                    // task-only choice.
                                    ws.finalize_create_lane(
                                        plan.clone(),
                                        project_id,
                                        daruda_store::tasks::TaskAgentSurface::Terminal,
                                        window,
                                        cx,
                                    )
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
                if let Err(e) = update_result {
                    LogWriter::log(
                        ErrorReport::new("Create lane completion could not reach workspace")
                            .severity(ErrorSeverity::Warning)
                            .at(file!(), line!())
                            .with_context("error", format!("{e}"))
                            .dedup("lane.create.modal.completion")
                            .build(),
                    );
                }
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
        let t = theme::current(cx).clone();
        let muted_text = t.text_muted;
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(s::create_lane_body_branch_name()),
            )
            .child(input(&self.branch_input, cx, 0))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(s::create_lane_body_base_ref()),
            )
            .child(input(&self.base_input, cx, 1))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(s::create_lane_body_description()),
            )
            .child(input(&self.description_input, cx, 2))
            .child(field_label(s::session_host_field_host(), &t))
            .child(select::select(&self.host_select, cx, 3_isize));

        if self.catalog.is_empty() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(s::create_lane_session_host_registry_empty_hint()),
            );
        }

        if self.selected_entry(cx).is_some() {
            body = body
                .child(field_label(s::session_host_field_session_path(), &t))
                .child(input(&self.session_path_input, cx, 4));
        }

        let create_disabled = self.submitting || self.branch_input.read(cx).value().is_empty();
        let submit_label = if self.submitting {
            s::create_lane_creating()
        } else {
            s::create_lane_submit()
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("create-wt-cancel", s::common_button_cancel()).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    },
                )),
            )
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
    use crate::lane::session_host::{SessionHostError, SessionHostField};
    use crate::test_support::init_gpui_component;
    use daruda_config::SessionHostKind;
    use daruda_store::project::SessionHostId;
    use gpui::{TestAppContext, WindowHandle};

    fn build_modal(
        cx: &mut TestAppContext,
        repo_root: &str,
        catalog: Vec<SessionHostEntry>,
    ) -> (
        WindowHandle<CreateWorktreeModal>,
        Entity<CreateWorktreeModal>,
    ) {
        init_gpui_component(cx);
        let wh = cx.add_window(|window, cx| {
            CreateWorktreeModal::new(
                WeakEntity::new_invalid(),
                PathBuf::from(repo_root),
                0, // test fixture — project_id not exercised
                catalog,
                window,
                cx,
            )
        });
        let modal = wh.root(cx).unwrap();
        (wh, modal)
    }

    fn ssh_entry(label: &str, target: &str) -> SessionHostEntry {
        SessionHostEntry {
            id: SessionHostId::new(),
            label: label.to_string(),
            kind: SessionHostKind::Ssh {
                target: target.to_string(),
            },
        }
    }

    fn select_host(
        wh: &WindowHandle<CreateWorktreeModal>,
        modal: &Entity<CreateWorktreeModal>,
        cx: &mut TestAppContext,
        value: &str,
    ) {
        let select = modal.read_with(cx, |m, _| m.host_select.clone());
        let value = SharedString::from(value.to_string());
        // SILENT-OK: window may drop after modal closes / dialog dismiss on focus restore
        let _ = wh.update(cx, |_root, window, cx| {
            select.update(cx, |s, cx_state| {
                s.set_selected_value(&value, window, cx_state);
            });
        });
    }

    fn set_session_path(
        wh: &WindowHandle<CreateWorktreeModal>,
        modal: &Entity<CreateWorktreeModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.session_path_input.clone(), s);
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
        let (_wh, modal) = build_modal(cx, "/repo", vec![]);
        modal.read_with(cx, |m, cx| {
            let err = m.validate(cx).unwrap_err();
            assert!(err.contains("required"));
        });
    }

    #[gpui::test]
    fn validate_accepts_valid_branch(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/Users/dev/repo", vec![]);
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
        let (wh, modal) = build_modal(cx, "/repo", vec![]);
        set_branch(&wh, &modal, cx, "has space");
        modal.read_with(cx, |m, cx| {
            let err = m.validate(cx).unwrap_err();
            assert!(err.contains("Invalid"));
        });
    }

    #[gpui::test]
    fn validate_captures_base_ref(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/Users/dev/repo", vec![]);
        set_branch(&wh, &modal, cx, "feat/x");
        set_base(&wh, &modal, cx, "origin/main");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.base_ref.as_deref(), Some("origin/main"));
        });
    }

    #[gpui::test]
    fn validate_blank_base_normalizes_to_none(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/repo", vec![]);
        set_branch(&wh, &modal, cx, "feat/x");
        set_base(&wh, &modal, cx, "   ");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert!(plan.base_ref.is_none());
        });
    }

    #[gpui::test]
    fn validate_captures_description(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/repo", vec![]);
        set_branch(&wh, &modal, cx, "feat/x");
        set_description(&wh, &modal, cx, "PR #123 review");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.description.as_deref(), Some("PR #123 review"));
        });
    }

    /// Local is the dropdown's default selection — untouched, the plan
    /// carries no `session_host` at all (the freshly-created lane stays at
    /// `Lane::git`'s `session_host: None` default).
    #[gpui::test]
    fn default_selection_is_local_and_plan_has_no_session_host(cx: &mut TestAppContext) {
        let entry = ssh_entry("Build box", "build-box");
        let (wh, modal) = build_modal(cx, "/repo", vec![entry]);
        set_branch(&wh, &modal, cx, "feat/x");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.session_host, None);
        });
    }

    /// Picking a registry entry and filling in the session path carries a
    /// fully-formed `LaneSessionHost` (registry id included) into the plan.
    #[gpui::test]
    fn selecting_a_registry_entry_sets_the_plans_session_host(cx: &mut TestAppContext) {
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, "/repo", vec![entry]);
        set_branch(&wh, &modal, cx, "feat/x");
        select_host(&wh, &modal, cx, &entry_id.as_inner().to_string());
        set_session_path(&wh, &modal, cx, "/home/user/project");
        modal.read_with(cx, |m, cx| {
            let plan = m.validate(cx).unwrap();
            assert_eq!(
                plan.session_host,
                Some(LaneSessionHost::Ssh {
                    target: "build-box".into(),
                    session_path: "/home/user/project".into(),
                    registry_id: Some(entry_id),
                })
            );
        });
    }

    /// An empty catalog offers only Local — nothing crashes, and the plan's
    /// `session_host` still comes back `None`.
    #[gpui::test]
    fn an_empty_catalog_only_offers_local(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, "/repo", vec![]);
        set_branch(&wh, &modal, cx, "feat/x");
        modal.read_with(cx, |m, cx| {
            assert!(m.catalog.is_empty());
            let plan = m.validate(cx).unwrap();
            assert_eq!(plan.session_host, None);
        });
    }

    /// Picking a registry entry but leaving the session path blank blocks
    /// creation with the same reused `session_host::checked_session_path`
    /// error `SessionHostModal` shows.
    #[gpui::test]
    fn validate_rejects_an_empty_session_path_when_a_registry_entry_is_selected(
        cx: &mut TestAppContext,
    ) {
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, "/repo", vec![entry]);
        set_branch(&wh, &modal, cx, "feat/x");
        select_host(&wh, &modal, cx, &entry_id.as_inner().to_string());
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.validate(cx).unwrap_err(),
                session_host_error_to_msg(SessionHostError::Empty(SessionHostField::SessionPath))
            );
        });
    }

    /// A session path that would escape `session_host::wrap`'s quoting is
    /// rejected the same way — inline error, no lane created.
    #[gpui::test]
    fn validate_rejects_an_unsafe_session_path_when_a_registry_entry_is_selected(
        cx: &mut TestAppContext,
    ) {
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, "/repo", vec![entry]);
        set_branch(&wh, &modal, cx, "feat/x");
        select_host(&wh, &modal, cx, &entry_id.as_inner().to_string());
        set_session_path(&wh, &modal, cx, "/srv/a\"b");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.validate(cx).unwrap_err(),
                session_host_error_to_msg(SessionHostError::Unsafe(SessionHostField::SessionPath))
            );
        });
    }
}
