//! Session Host modal — the single place a lane's agent session host
//! (Local / a registered SSH or Docker host) and its session path are set
//! together, so a remote setup can never end up as a host without a path or
//! vice versa (see `daruda_store::project::LaneSessionHost`).
//!
//! UI-FM-based form modal, mirroring `right_dock::tools::add_modal`: a
//! single registry dropdown (`crate::ui::select`) picks Local, "keep the
//! lane's current value" (the default whenever that value isn't already a
//! live registry link), or one catalog entry — free-text `target`/`container`
//! entry was removed in favor of the registry (see `settings_window::sections
//! ::session_hosts`, where a host is actually registered). `build_host`
//! validates via `lane::session_host::from_registry_entry` (the one place
//! that quoting-safety rule lives — see that module), and an inline banner
//! surfaces the first rejected field.

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::lane::session_host::{self, LinkStatus, SessionHostError, SessionHostField};
use crate::surface::strings as s;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::select::{self, SelectOption, SelectState};
use crate::ui::theme;
use crate::ui::{InputEvent, InputState, button, button_primary, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use daruda_config::SessionHostEntry;
use daruda_store::project::{LaneRef, LaneSessionHost, SessionHostId};

/// Registry-select sentinel for "no host" — never a real `SessionHostId`
/// string, so it can't collide with a catalog entry.
const LOCAL_SELECT_VALUE: &str = "local";

/// Registry-select sentinel meaning "leave the lane's current value
/// untouched". Offered — and selected by default — only when that current
/// value is a remote host that isn't already a live registry link
/// ([`LinkStatus::Unlinked`] or [`LinkStatus::Orphaned`]), so opening this
/// modal and hitting Save without touching the dropdown can never silently
/// downgrade a working remote lane to Local.
const KEEP_CURRENT_SELECT_VALUE: &str = "keep-current";

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

/// `host`'s registry link, read straight off whichever variant carries it —
/// `None` for `Local` and for a free-text `Ssh`/`Docker` host.
fn host_registry_id(host: &LaneSessionHost) -> Option<SessionHostId> {
    match host {
        LaneSessionHost::Ssh { registry_id, .. } | LaneSessionHost::Docker { registry_id, .. } => {
            *registry_id
        }
        LaneSessionHost::Local => None,
    }
}

/// The bare `target`/`container` string a `Ssh`/`Docker` host currently
/// carries — what the "keep current" option's label shows, so the user can
/// see what they'd be preserving before deciding to replace it.
fn host_display_value(host: &LaneSessionHost) -> Option<&str> {
    match host {
        LaneSessionHost::Ssh { target, .. } => Some(target),
        LaneSessionHost::Docker { container, .. } => Some(container),
        LaneSessionHost::Local => None,
    }
}

/// `current`'s session path, or `""` for `Local`/unanswered — the seed value
/// for `session_path_input`.
fn current_session_path(current: &Option<LaneSessionHost>) -> &str {
    match current {
        Some(
            LaneSessionHost::Ssh { session_path, .. }
            | LaneSessionHost::Docker { session_path, .. },
        ) => session_path,
        _ => "",
    }
}

/// The registry dropdown's option list: an optional leading "keep current"
/// entry (see [`KEEP_CURRENT_SELECT_VALUE`]), then Local, then every catalog
/// entry keyed by its id (so a later label rename doesn't change the value
/// the select stores).
fn registry_select_options(
    catalog: &[SessionHostEntry],
    keep_current: Option<&LaneSessionHost>,
) -> Vec<SelectOption> {
    let mut opts = Vec::with_capacity(catalog.len() + 2);
    if let Some(value) = keep_current.and_then(host_display_value) {
        opts.push(SelectOption::new(
            KEEP_CURRENT_SELECT_VALUE,
            s::session_host_option_keep_current(value),
        ));
    }
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
    /// The workspace's `session_hosts` registry catalog at the moment the
    /// modal was opened — a one-time snapshot, like `current` and
    /// `has_legacy_remote_cwd` above.
    pub catalog: Vec<SessionHostEntry>,
}

pub struct SessionHostModal {
    panel_focus_handle: FocusHandle,

    registry_select: Entity<SelectState>,
    session_path_input: Entity<InputState>,
    _session_path_sub: Subscription,
    _registry_select_sub: Subscription,

    catalog: Vec<SessionHostEntry>,
    /// The lane's host exactly as it was when the modal opened. `build_host`
    /// returns this verbatim when the "keep current" option is (still)
    /// selected — the only thing standing between an Unlinked/Orphaned
    /// legacy lane and an accidental downgrade to Local on Save.
    original_host: Option<LaneSessionHost>,
    /// Whether `original_host`'s registry link is dangling
    /// (`LinkStatus::Orphaned`) — drives the Orphaned banner.
    orphaned: bool,
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
        let catalog = initial.catalog;
        let link_status = initial
            .current
            .as_ref()
            .map(|host| session_host::registry_link_status(host, &catalog));
        let orphaned = matches!(link_status, Some(LinkStatus::Orphaned));
        // Fresh gets its own catalog entry pre-selected below (equally
        // non-destructive to re-save); Unlinked/Orphaned get "keep current"
        // instead, since neither has a live entry to pre-select.
        let keep_current = match link_status {
            Some(LinkStatus::Orphaned) | Some(LinkStatus::Unlinked) => initial.current.as_ref(),
            _ => None,
        };
        let selected_value: SharedString = match (&link_status, &initial.current) {
            (Some(LinkStatus::Fresh), Some(host)) => host_registry_id(host)
                .expect("Fresh implies a resolvable registry id")
                .as_inner()
                .to_string()
                .into(),
            (Some(LinkStatus::Orphaned), _) | (Some(LinkStatus::Unlinked), _) => {
                KEEP_CURRENT_SELECT_VALUE.into()
            }
            _ => LOCAL_SELECT_VALUE.into(),
        };
        let session_path = current_session_path(&initial.current).to_string();

        let registry_select = cx.new(|cx_state| {
            select::state_with_options(
                registry_select_options(&catalog, keep_current),
                Some(&selected_value),
                window,
                cx_state,
            )
        });
        let session_path_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::session_host_placeholder_session_path())
                .default_value(session_path)
        });

        let session_path_sub = forward_input(&session_path_input, window, cx);
        let registry_select_sub = cx.subscribe_in(
            &registry_select,
            window,
            |this, _, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.clear_error(cx);
                    cx.notify();
                }
            },
        );

        Self {
            panel_focus_handle: cx.focus_handle(),
            registry_select,
            session_path_input,
            _session_path_sub: session_path_sub,
            _registry_select_sub: registry_select_sub,
            catalog,
            original_host: initial.current,
            orphaned,
            has_legacy_remote_cwd: initial.has_legacy_remote_cwd,
            error: None,
            submitting: false,
            workspace,
            lane_ref: initial.lane_ref,
        }
    }

    /// The catalog entry the dropdown currently points at — `None` for
    /// Local and for the "keep current" sentinel, both of which resolve
    /// through `build_host` without consulting the catalog.
    fn selected_entry(&self, cx: &App) -> Option<&SessionHostEntry> {
        let value = self.registry_select.read(cx).selected_value()?.to_string();
        self.catalog
            .iter()
            .find(|entry| entry.id.as_inner().to_string() == value)
    }

    fn build_host(&self, cx: &App) -> Result<LaneSessionHost, SharedString> {
        let value = self
            .registry_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string());
        match value.as_deref() {
            Some(KEEP_CURRENT_SELECT_VALUE) => {
                Ok(self.original_host.clone().unwrap_or(LaneSessionHost::Local))
            }
            None | Some(LOCAL_SELECT_VALUE) => Ok(LaneSessionHost::Local),
            Some(_) => {
                let Some(entry) = self.selected_entry(cx) else {
                    // Unreachable in practice: every non-sentinel option
                    // value is minted from `self.catalog` itself in
                    // `registry_select_options`. Fall back to Local rather
                    // than panic.
                    return Ok(LaneSessionHost::Local);
                };
                let path = self.session_path_input.read(cx).value().to_string();
                session_host::from_registry_entry(entry, &path).map_err(session_host_error_to_msg)
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
        let orphaned_notice = self.orphaned.then(|| {
            crate::ui::alert::warning("session-host-orphaned", s::session_host_orphaned_banner())
        });

        let showing_session_path = self.selected_entry(cx).is_some();

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(s::session_host_field_host(), &t))
            .child(select::select(&self.registry_select, cx, 1_isize));

        if self.catalog.is_empty() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(t.text_muted)
                    .child(s::session_host_registry_empty_hint()),
            );
        }

        if showing_session_path {
            body = body
                .child(field_label(s::session_host_field_session_path(), &t))
                .child(input(&self.session_path_input, cx, 2));
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
        if let Some(b) = orphaned_notice {
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
        catalog: ws.session_hosts.clone(),
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
    use daruda_config::SessionHostKind;
    use gpui::{TestAppContext, WindowHandle};

    fn ssh_entry(label: &str, target: &str) -> SessionHostEntry {
        SessionHostEntry {
            id: SessionHostId::new(),
            label: label.to_string(),
            kind: SessionHostKind::Ssh {
                target: target.to_string(),
            },
        }
    }

    fn docker_entry(label: &str, container: &str) -> SessionHostEntry {
        SessionHostEntry {
            id: SessionHostId::new(),
            label: label.to_string(),
            kind: SessionHostKind::Docker {
                container: container.to_string(),
            },
        }
    }

    fn build_modal(
        cx: &mut TestAppContext,
        current: Option<LaneSessionHost>,
        has_legacy_remote_cwd: bool,
        catalog: Vec<SessionHostEntry>,
    ) -> (WindowHandle<SessionHostModal>, Entity<SessionHostModal>) {
        init_gpui_component(cx);
        let initial = SessionHostInitial {
            lane_ref: LaneRef {
                project: 0,
                lane: 0,
            },
            current,
            has_legacy_remote_cwd,
            catalog,
        };
        let wh = cx.add_window(|window, cx| {
            SessionHostModal::new(WeakEntity::new_invalid(), initial, window, cx)
        });
        let modal = wh.root(cx).unwrap();
        (wh, modal)
    }

    fn select_entry(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
        entry_id: SessionHostId,
    ) {
        let select = modal.read_with(cx, |m, _| m.registry_select.clone());
        let value = SharedString::from(entry_id.as_inner().to_string());
        // SILENT-OK: window may drop after modal closes / dialog dismiss on focus restore
        let _ = wh.update(cx, |_root, window, cx| {
            select.update(cx, |s, cx_state| {
                s.set_selected_value(&value, window, cx_state);
            });
        });
    }

    fn select_local(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
    ) {
        let select = modal.read_with(cx, |m, _| m.registry_select.clone());
        let value = SharedString::from(LOCAL_SELECT_VALUE);
        // SILENT-OK: window may drop after modal closes / dialog dismiss on focus restore
        let _ = wh.update(cx, |_root, window, cx| {
            select.update(cx, |s, cx_state| {
                s.set_selected_value(&value, window, cx_state);
            });
        });
    }

    fn set_session_path(
        wh: &WindowHandle<SessionHostModal>,
        modal: &Entity<SessionHostModal>,
        cx: &mut TestAppContext,
        value: &str,
    ) {
        let input_state = modal.read_with(cx, |m, _| m.session_path_input.clone());
        // SILENT-OK: window may drop after modal closes / dialog dismiss on focus restore
        let _ = wh.update(cx, |_root, window, cx| {
            input_state.update(cx, |i, cx_state| {
                i.set_value(value.to_string(), window, cx_state);
            });
        });
    }

    #[gpui::test]
    fn seeds_local_and_blank_when_never_answered(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, None, false, vec![]);
        modal.read_with(cx, |m, cx| {
            assert!(!m.orphaned);
            assert_eq!(m.build_host(cx), Ok(LaneSessionHost::Local));
        });
    }

    /// A `Fresh` registry link pre-selects its own catalog entry, and
    /// re-saving without touching the dropdown reproduces the same host
    /// (registry id included) since the catalog target hasn't moved.
    #[gpui::test]
    fn seeds_the_dropdown_from_a_fresh_registry_link(cx: &mut TestAppContext) {
        let entry = ssh_entry("Build box", "vm-work");
        let current = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(entry.id),
        };
        let (_wh, modal) = build_modal(cx, Some(current.clone()), false, vec![entry]);
        modal.read_with(cx, |m, cx| {
            assert!(!m.orphaned);
            assert_eq!(m.build_host(cx), Ok(current));
        });
    }

    /// Picking a registry entry stores its id plus the entry's current
    /// target as the lane's cached value, keyed off the user's session path.
    #[gpui::test]
    fn picking_a_registry_entry_saves_its_id_and_target(cx: &mut TestAppContext) {
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, None, false, vec![entry]);
        select_entry(&wh, &modal, cx, entry_id);
        set_session_path(&wh, &modal, cx, "/home/user/project");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Ok(LaneSessionHost::Ssh {
                    target: "build-box".into(),
                    session_path: "/home/user/project".into(),
                    registry_id: Some(entry_id),
                })
            );
        });
    }

    #[gpui::test]
    fn picking_a_docker_registry_entry_saves_its_id_and_container(cx: &mut TestAppContext) {
        let entry = docker_entry("Dev container", "dev-1");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, None, false, vec![entry]);
        select_entry(&wh, &modal, cx, entry_id);
        set_session_path(&wh, &modal, cx, "/workspace");
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Ok(LaneSessionHost::Docker {
                    container: "dev-1".into(),
                    session_path: "/workspace".into(),
                    registry_id: Some(entry_id),
                })
            );
        });
    }

    #[gpui::test]
    fn picking_a_registry_entry_rejects_an_empty_session_path(cx: &mut TestAppContext) {
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, None, false, vec![entry]);
        select_entry(&wh, &modal, cx, entry_id);
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Err(session_host_error_to_msg(SessionHostError::Empty(
                    SessionHostField::SessionPath
                )))
            );
        });
    }

    #[gpui::test]
    fn picking_a_registry_entry_rejects_a_session_path_that_would_escape_the_quoting(
        cx: &mut TestAppContext,
    ) {
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, None, false, vec![entry]);
        select_entry(&wh, &modal, cx, entry_id);
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

    /// A catalog with no entries offers only "Local" — nothing crashes, and
    /// the modal's hint condition (`catalog.is_empty()`, read by `render`)
    /// is exactly this.
    #[gpui::test]
    fn an_empty_catalog_only_offers_local(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, None, false, vec![]);
        modal.read_with(cx, |m, cx| {
            assert!(m.catalog.is_empty());
            assert_eq!(m.build_host(cx), Ok(LaneSessionHost::Local));
        });
    }

    /// The core regression guard: an `Orphaned` host (its `registry_id` no
    /// longer resolves) must show the banner, but opening the modal and
    /// saving immediately — without touching the dropdown — must reproduce
    /// the lane's last-known value byte for byte, never Local.
    #[gpui::test]
    fn an_orphaned_host_shows_the_banner_and_saving_untouched_keeps_its_value(
        cx: &mut TestAppContext,
    ) {
        let missing_id = SessionHostId::new();
        let current = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(missing_id),
        };
        let (_wh, modal) = build_modal(cx, Some(current.clone()), false, vec![]);
        modal.read_with(cx, |m, cx| {
            assert!(m.orphaned);
            assert_eq!(m.build_host(cx), Ok(current));
        });
    }

    /// The other half of the regression guard: a legacy free-input host
    /// (`registry_id: None`) is `Unlinked`, not `Orphaned` — no banner — but
    /// still must round-trip untouched on an immediate Save, since it was
    /// never broken, just never registered.
    #[gpui::test]
    fn a_legacy_free_input_host_shows_no_banner_and_saving_untouched_keeps_its_value(
        cx: &mut TestAppContext,
    ) {
        let current = LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: None,
        };
        // An unrelated catalog entry must not change the outcome.
        let unrelated = ssh_entry("Unrelated", "other-box");
        let (_wh, modal) = build_modal(cx, Some(current.clone()), false, vec![unrelated]);
        modal.read_with(cx, |m, cx| {
            assert!(!m.orphaned);
            assert_eq!(m.build_host(cx), Ok(current));
        });
    }

    /// A legacy/orphaned lane can still be migrated forward: explicitly
    /// picking a registry entry replaces "keep current" and the saved host
    /// now points at the registry going forward.
    #[gpui::test]
    fn a_legacy_host_can_be_migrated_to_a_registry_entry(cx: &mut TestAppContext) {
        let current = LaneSessionHost::Ssh {
            target: "old-free-text".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        let entry = ssh_entry("Build box", "build-box");
        let entry_id = entry.id;
        let (wh, modal) = build_modal(cx, Some(current), false, vec![entry]);
        select_entry(&wh, &modal, cx, entry_id);
        modal.read_with(cx, |m, cx| {
            assert_eq!(
                m.build_host(cx),
                Ok(LaneSessionHost::Ssh {
                    target: "build-box".into(),
                    session_path: "/srv/app".into(),
                    registry_id: Some(entry_id),
                })
            );
        });
    }

    /// Explicitly picking Local overrides the "keep current" default —
    /// deliberate, unlike leaving the default untouched.
    #[gpui::test]
    fn explicitly_picking_local_overrides_keep_current(cx: &mut TestAppContext) {
        let current = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        let (wh, modal) = build_modal(cx, Some(current), false, vec![]);
        select_local(&wh, &modal, cx);
        modal.read_with(cx, |m, cx| {
            assert_eq!(m.build_host(cx), Ok(LaneSessionHost::Local));
        });
    }

    /// Drives the in-modal legacy-combo notice (`has_legacy_remote_cwd`) —
    /// `open_session_host_modal` computes it as `session_host.is_none() &&
    /// remote_cwd.is_some()`; this only checks it round-trips into the
    /// modal's own field, which the render path reads to show the banner.
    #[gpui::test]
    fn legacy_notice_flag_round_trips_from_the_snapshot(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, None, true, vec![]);
        modal.read_with(cx, |m, _| assert!(m.has_legacy_remote_cwd));

        let (_wh, modal) = build_modal(cx, None, false, vec![]);
        modal.read_with(cx, |m, _| assert!(!m.has_legacy_remote_cwd));
    }
}
