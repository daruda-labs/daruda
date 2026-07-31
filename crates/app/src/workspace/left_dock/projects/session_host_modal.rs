//! Session Host modal — the single place a lane's agent session host
//! (Local / SSH / Docker) and its session path are set together, so a
//! remote setup can never end up as a host without a path or vice versa
//! (see `daruda_store::project::LaneSessionHost`).
//!
//! UI-FM-based form modal, mirroring `right_dock::tools::add_modal`: a
//! segmented kind selector swaps which fields render, `build_host`
//! validates via `lane::session_host::{sanitized_ssh, sanitized_docker}`
//! (the one place that quoting-safety rule lives — see that module), and
//! an inline banner surfaces the first rejected field.

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::lane::session_host::{self, SessionHostError, SessionHostField};
use crate::surface::strings as s;
use crate::ui::Disableable as _;
use crate::ui::Selectable as _;
use crate::ui::WindowExt as _;
use crate::ui::theme;
use crate::ui::{InputEvent, InputState, button, button_group, button_primary, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use daruda_store::project::{LaneRef, LaneSessionHost};

/// Which host kind the form is currently editing. A UI-only selection —
/// distinct from [`LaneSessionHost`] because the user can pick "SSH" before
/// typing a valid target/path, a state `LaneSessionHost` cannot represent.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostKind {
    Local,
    Ssh,
    Docker,
}

fn host_kind_options() -> [(HostKind, String); 3] {
    [
        (HostKind::Local, s::session_host_option_local()),
        (HostKind::Ssh, s::session_host_option_ssh()),
        (HostKind::Docker, s::session_host_option_docker()),
    ]
}

/// Small label rendered above a form field — mirrors
/// `right_dock::tools::modal_shared::field_label`, kept local since this is
/// the only modal in this directory that needs it.
fn field_label(text: impl Into<SharedString>, t: &theme::DarudaTheme) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted)
        .child(text.into())
}

fn session_host_error_to_msg(e: SessionHostError) -> SharedString {
    match e {
        SessionHostError::Empty(SessionHostField::Target) => s::session_host_err_target_empty(),
        SessionHostError::Empty(SessionHostField::Container) => {
            s::session_host_err_container_empty()
        }
        SessionHostError::Empty(SessionHostField::SessionPath) => {
            s::session_host_err_session_path_empty()
        }
        SessionHostError::Unsafe(SessionHostField::Target) => s::session_host_err_target_unsafe(),
        SessionHostError::Unsafe(SessionHostField::Container) => {
            s::session_host_err_container_unsafe()
        }
        SessionHostError::Unsafe(SessionHostField::SessionPath) => {
            s::session_host_err_session_path_unsafe()
        }
    }
    .into()
}

/// Snapshot built by `Workspace::open_session_host_modal` and passed in to
/// the modal's constructor — it never reads the workspace itself.
pub struct SessionHostInitial {
    pub lane_ref: LaneRef,
    /// The lane's current answer. `None` seeds a blank `Local` form —
    /// deliberately never seeded from the legacy `remote_cwd`/agent pair,
    /// which this modal does not represent (see `has_legacy_remote_cwd`).
    pub current: Option<LaneSessionHost>,
    /// Whether the lane is presently relying on the legacy agent-side pair
    /// (`session_host` unanswered, `remote_cwd` set) — drives the in-modal
    /// notice that saving here (Local included) retires it.
    pub has_legacy_remote_cwd: bool,
}

pub struct SessionHostModal {
    panel_focus_handle: FocusHandle,

    target_input: Entity<InputState>,
    container_input: Entity<InputState>,
    session_path_input: Entity<InputState>,
    _input_subs: [Subscription; 3],

    kind: HostKind,
    has_legacy_remote_cwd: bool,

    error: Option<SharedString>,
    submitting: bool,

    workspace: WeakEntity<Workspace>,
    lane_ref: LaneRef,
}

impl SessionHostModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        initial: SessionHostInitial,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (kind, target, container, session_path) = match &initial.current {
            Some(LaneSessionHost::Local) | None => (HostKind::Local, "", "", ""),
            Some(LaneSessionHost::Ssh {
                target,
                session_path,
            }) => (HostKind::Ssh, target.as_str(), "", session_path.as_str()),
            Some(LaneSessionHost::Docker {
                container,
                session_path,
            }) => (
                HostKind::Docker,
                "",
                container.as_str(),
                session_path.as_str(),
            ),
        };

        let target_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::session_host_placeholder_target())
                .default_value(target)
        });
        let container_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::session_host_placeholder_container())
                .default_value(container)
        });
        let session_path_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::session_host_placeholder_session_path())
                .default_value(session_path)
        });

        let subs = [
            forward_input(&target_input, window, cx),
            forward_input(&container_input, window, cx),
            forward_input(&session_path_input, window, cx),
        ];

        Self {
            panel_focus_handle: cx.focus_handle(),
            target_input,
            container_input,
            session_path_input,
            _input_subs: subs,
            kind,
            has_legacy_remote_cwd: initial.has_legacy_remote_cwd,
            error: None,
            submitting: false,
            workspace,
            lane_ref: initial.lane_ref,
        }
    }

    fn build_host(&self, cx: &App) -> Result<LaneSessionHost, SharedString> {
        match self.kind {
            HostKind::Local => Ok(LaneSessionHost::Local),
            HostKind::Ssh => {
                let target = self.target_input.read(cx).value().to_string();
                let path = self.session_path_input.read(cx).value().to_string();
                session_host::sanitized_ssh(&target, &path).map_err(session_host_error_to_msg)
            }
            HostKind::Docker => {
                let container = self.container_input.read(cx).value().to_string();
                let path = self.session_path_input.read(cx).value().to_string();
                session_host::sanitized_docker(&container, &path).map_err(session_host_error_to_msg)
            }
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let host = match self.build_host(cx) {
            Ok(h) => h,
            Err(msg) => {
                self.error = Some(msg);
                cx.notify();
                return;
            }
        };
        let target = self.lane_ref;
        self.submitting = true;
        cx.notify();

        let Some(ws) = self.workspace.upgrade() else {
            self.submitting = false;
            cx.notify();
            return;
        };
        let me = cx.entity().downgrade();
        let window_handle = window.window_handle();
        // Same deferred re-entry shape as `AddMcpServerModal::submit` — this
        // runs inside `cx.listener`, i.e. while this modal entity is
        // already being updated, so the dismiss has to land on a fresh
        // update cycle.
        ws.update(cx, |ws, cx_inner| {
            ws.set_lane_session_host(target, host, cx_inner);
            cx_inner.defer(move |cx| {
                // SILENT-OK: window may be gone by the time this defer runs — nothing left to dismiss.
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    let Some(me) = me.upgrade() else { return };
                    me.update(cx, |modal, cx| modal.dismiss(window, cx));
                });
            });
        });
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    fn clear_error(&mut self, cx: &mut Context<Self>) {
        if self.error.is_some() {
            self.error = None;
            cx.notify();
        }
    }
}

fn forward_input(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<SessionHostModal>,
) -> Subscription {
    cx.subscribe_in(
        state,
        window,
        |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } => this.submit(window, cx),
            InputEvent::Change => this.clear_error(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        },
    )
}

impl Focusable for SessionHostModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        // No field is common to every kind (Local has none at all), so the
        // panel itself is the one focus target every state can offer —
        // mirrors `RemoveWorktreeModal`.
        self.panel_focus_handle.clone()
    }
}

impl ModalView for SessionHostModal {}

impl Render for SessionHostModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
            .map(|msg| crate::ui::alert::error("session-host-error", msg.clone()));
        let legacy_notice = self.has_legacy_remote_cwd.then(|| {
            crate::ui::alert::info("session-host-legacy", s::session_host_legacy_notice())
        });

        let kind_options = host_kind_options();
        let kind_values: Vec<HostKind> = kind_options.iter().map(|(k, _)| *k).collect();
        let kind_chip = button_group("session-host-kind-group")
            .children(kind_options.iter().map(|(kind, label)| {
                button(
                    SharedString::from(format!("session-host-kind-{}", *kind as u8)),
                    label.clone(),
                )
                .selected(self.kind == *kind)
            }))
            .on_click(cx.listener(move |this, ixs: &Vec<usize>, _w, cx| {
                if let Some(&ix) = ixs.first() {
                    this.kind = kind_values[ix];
                    cx.notify();
                }
            }));

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(s::session_host_field_host(), &t))
            .child(kind_chip);

        match self.kind {
            HostKind::Local => {}
            HostKind::Ssh => {
                body = body
                    .child(field_label(s::session_host_field_target(), &t))
                    .child(input(&self.target_input, cx, 1))
                    .child(field_label(s::session_host_field_session_path(), &t))
                    .child(input(&self.session_path_input, cx, 2));
            }
            HostKind::Docker => {
                body = body
                    .child(field_label(s::session_host_field_container(), &t))
                    .child(input(&self.container_input, cx, 1))
                    .child(field_label(s::session_host_field_session_path(), &t))
                    .child(input(&self.session_path_input, cx, 2));
            }
        }

        let save_label = if submitting {
            s::session_host_saving()
        } else {
            s::session_host_save()
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("session-host-cancel", s::session_host_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.dismiss(w, cx))),
            )
            .child(
                button_primary("session-host-save", save_label)
                    .disabled(submitting)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("SessionHostModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(b) = legacy_notice {
            p = p.child(b);
        }
        if let Some(b) = banner {
            p = p.child(b);
        }
        p.child(footer)
    }
}

pub fn open_session_host_modal(
    ws: &mut Workspace,
    target: LaneRef,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(lane) = ws.lane_for(target) else {
        return;
    };
    let initial = SessionHostInitial {
        lane_ref: target,
        current: lane.session_host.clone(),
        has_legacy_remote_cwd: lane.session_host.is_none() && lane.remote_cwd.is_some(),
    };
    let workspace = cx.weak_entity();
    crate::workspace::dialog_helpers::open_form_modal(
        s::session_host_modal_title(),
        None,
        move |window, cx| SessionHostModal::new(workspace, initial, window, cx),
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::init_gpui_component;
    use gpui::{TestAppContext, WindowHandle};

    fn build_modal(
        cx: &mut TestAppContext,
        current: Option<LaneSessionHost>,
        has_legacy_remote_cwd: bool,
    ) -> (WindowHandle<SessionHostModal>, Entity<SessionHostModal>) {
        init_gpui_component(cx);
        let initial = SessionHostInitial {
            lane_ref: LaneRef {
                project: 0,
                lane: 0,
            },
            current,
            has_legacy_remote_cwd,
        };
        let wh = cx.add_window(|window, cx| {
            SessionHostModal::new(WeakEntity::new_invalid(), initial, window, cx)
        });
        let modal = wh.root(cx).unwrap();
        (wh, modal)
    }

    fn set_field(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
        field: fn(&SessionHostModal) -> Entity<InputState>,
        s: &str,
    ) {
        let state = modal.read_with(cx, |m, _| field(m));
        // SILENT-OK: window may drop after modal closes / dialog dismiss on focus restore
        let _ = wh.update(cx, |_root, window, cx| {
            state.update(cx, |i, cx_state| {
                i.set_value(s.to_string(), window, cx_state);
            });
        });
    }

    fn set_target(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.target_input.clone(), s);
    }

    fn set_container(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.container_input.clone(), s);
    }

    fn set_session_path(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
        s: &str,
    ) {
        set_field(wh, modal, cx, |m| m.session_path_input.clone(), s);
    }

    #[gpui::test]
    fn seeds_local_and_blank_when_never_answered(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, None, false);
        modal.read_with(cx, |m, cx| {
            assert!(matches!(m.kind, HostKind::Local));
            assert_eq!(m.build_host(cx), Ok(LaneSessionHost::Local));
        });
    }

    #[gpui::test]
    fn seeds_from_the_lanes_current_ssh_host(cx: &mut TestAppContext) {
        let current = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
        };
        let (_wh, modal) = build_modal(cx, Some(current.clone()), false);
        modal.read_with(cx, |m, cx| {
            assert!(matches!(m.kind, HostKind::Ssh));
            assert_eq!(m.build_host(cx), Ok(current));
        });
    }

    #[gpui::test]
    fn seeds_from_the_lanes_current_docker_host(cx: &mut TestAppContext) {
        let current = LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
        };
        let (_wh, modal) = build_modal(cx, Some(current.clone()), false);
        modal.read_with(cx, |m, cx| {
            assert!(matches!(m.kind, HostKind::Docker));
            assert_eq!(m.build_host(cx), Ok(current));
        });
    }

    #[gpui::test]
    fn local_kind_ignores_whatever_the_hidden_fields_hold(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None, false);
        set_target(&wh, &modal, cx, "leftover-target");
        modal.update(cx, |m, _| m.kind = HostKind::Local);
        modal.read_with(cx, |m, cx| {
            assert_eq!(m.build_host(cx), Ok(LaneSessionHost::Local));
        });
    }

    #[gpui::test]
    fn ssh_kind_builds_from_its_two_fields(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None, false);
        modal.update(cx, |m, _| m.kind = HostKind::Ssh);
        set_target(&wh, &modal, cx, "build-box");
        set_session_path(&wh, &modal, cx, "/home/user/project");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Ok(LaneSessionHost::Ssh {
                    target: "build-box".into(),
                    session_path: "/home/user/project".into(),
                })
            );
        });
    }

    #[gpui::test]
    fn docker_kind_builds_from_its_two_fields(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None, false);
        modal.update(cx, |m, _| m.kind = HostKind::Docker);
        set_container(&wh, &modal, cx, "dev-1");
        set_session_path(&wh, &modal, cx, "/workspace");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Ok(LaneSessionHost::Docker {
                    container: "dev-1".into(),
                    session_path: "/workspace".into(),
                })
            );
        });
    }

    /// An unsafe input never reaches `set_lane_session_host` — `build_host`
    /// rejects it up front and `submit` (see its body) bails before saving.
    #[gpui::test]
    fn ssh_kind_rejects_an_empty_target_without_saving(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None, false);
        modal.update(cx, |m, _| m.kind = HostKind::Ssh);
        set_session_path(&wh, &modal, cx, "/srv/app");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Err(session_host_error_to_msg(SessionHostError::Empty(
                    SessionHostField::Target
                )))
            );
        });
    }

    #[gpui::test]
    fn ssh_kind_rejects_a_session_path_that_would_escape_the_quoting(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None, false);
        modal.update(cx, |m, _| m.kind = HostKind::Ssh);
        set_target(&wh, &modal, cx, "box");
        set_session_path(&wh, &modal, cx, "/srv/a\"b");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Err(session_host_error_to_msg(SessionHostError::Unsafe(
                    SessionHostField::SessionPath
                )))
            );
        });
    }

    /// Drives the in-modal legacy-combo notice (`has_legacy_remote_cwd`) —
    /// `open_session_host_modal` computes it as `session_host.is_none() &&
    /// remote_cwd.is_some()`; this only checks it round-trips into the
    /// modal's own field, which the render path reads to show the banner.
    #[gpui::test]
    fn legacy_notice_flag_round_trips_from_the_snapshot(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, None, true);
        modal.read_with(cx, |m, _| assert!(m.has_legacy_remote_cwd));

        let (_wh, modal) = build_modal(cx, None, false);
        modal.read_with(cx, |m, _| assert!(!m.has_legacy_remote_cwd));
    }
}
