//! Edit-MCP-server modal.
//!
//! Same shape as [`super::add_modal::AddMcpServerModal`] except:
//! - Name + Scope fields are read-only (changing them is a delete +
//!   add round-trip).
//! - Inputs are pre-populated from `EditMcpInitial` snapshot built by
//!   `super::open_edit_mcp_server_modal`.
//! - On submit, calls `Workspace::update_mcp_server` rather than
//!   `add_mcp_server`.

use crate::ui::theme;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use super::modal_shared::{
    field_error_to_msg, field_label, join_args, split_args, transport_options,
};
use crate::agent::mcp::{
    McpScope, McpServerDraft, McpTransport, format_env_lines, validate_command, validate_url,
};
use crate::surface::strings;
use crate::ui::Disableable as _;
use crate::ui::Selectable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_group, button_primary, checkbox};
use crate::workspace::ModalView;
use crate::workspace::Workspace;

fn blank_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Snapshot built by `Workspace::open_edit_mcp_server_modal`. Captures
/// every input the modal pre-populates so its `new()` doesn't need to
/// read the workspace.
#[derive(Clone, Debug)]
pub struct EditMcpInitial {
    pub original: McpServerDraft,
    pub project_root: Option<std::path::PathBuf>,
    pub scope: McpScope,
    pub name: String,
    pub transport: McpTransport,
    pub command: String,
    pub args: String,
    pub url: String,
    pub env_text: String,
    pub headers_text: String,
    pub disabled: bool,
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

impl EditMcpInitial {
    /// Build the snapshot from a parsed `McpServer`. Convenience —
    /// callers don't need to know the env / headers serialisation.
    pub fn from_state(
        state: &crate::agent::mcp::McpSnapshot,
        scope: McpScope,
        name: &str,
        project_root: Option<std::path::PathBuf>,
    ) -> Option<Self> {
        let s = state.find(scope, name)?;
        Some(Self {
            original: McpServerDraft::from_server(s),
            project_root,
            scope,
            name: s.name.clone(),
            transport: s.transport,
            command: s.command.clone().unwrap_or_default(),
            args: join_args(&s.args),
            url: s.url.clone().unwrap_or_default(),
            env_text: format_env_lines(&s.env),
            headers_text: format_env_lines(&s.headers),
            disabled: s.disabled,
            extra: s.extra.clone(),
        })
    }
}

pub struct EditMcpServerModal {
    panel_focus_handle: FocusHandle,
    form: super::super::form_state::FormState,

    command_input: Entity<InputState>,
    args_input: Entity<InputState>,
    url_input: Entity<InputState>,
    env_input: Entity<InputState>,
    headers_input: Entity<InputState>,

    initial: EditMcpInitial,
    transport: McpTransport,
    disabled: bool,

    error: Option<SharedString>,
    error_field: Option<isize>,
    submitting: bool,

    workspace: WeakEntity<Workspace>,
    _input_subs: Vec<Subscription>,
}

impl EditMcpServerModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        initial: EditMcpInitial,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let command_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).default_value(initial.command.clone())
        });
        let args_input = cx
            .new(|cx_state| InputState::new(window, cx_state).default_value(initial.args.clone()));
        let url_input =
            cx.new(|cx_state| InputState::new(window, cx_state).default_value(initial.url.clone()));
        let env_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .auto_grow(2, 6)
                .default_value(initial.env_text.clone())
        });
        let headers_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .auto_grow(2, 6)
                .default_value(initial.headers_text.clone())
        });

        let subs = vec![
            forward_input(&command_input, window, cx),
            forward_input(&args_input, window, cx),
            forward_input(&url_input, window, cx),
            forward_multi_input(&env_input, window, cx),
            forward_multi_input(&headers_input, window, cx),
        ];

        let transport = initial.transport;
        let disabled = initial.disabled;

        let mut modal = Self {
            form: Default::default(),
            panel_focus_handle: cx.focus_handle(),
            command_input,
            args_input,
            url_input,
            env_input,
            headers_input,
            initial,
            transport,
            disabled,
            error: None,
            error_field: None,
            submitting: false,
            workspace,
            _input_subs: subs,
        };
        modal.form.baseline = modal.values(cx);
        modal
    }

    fn build_draft(&self, cx: &gpui::App) -> Result<McpServerDraft, (isize, SharedString)> {
        let command_text = self.command_input.read(cx).value().to_string();
        let url_text = self.url_input.read(cx).value().to_string();
        if let Err(e) = validate_command(&command_text, self.transport) {
            return Err((0, field_error_to_msg(e)));
        }
        if let Err(e) = validate_url(&url_text, self.transport) {
            return Err((0, field_error_to_msg(e)));
        }

        let env_text = self.env_input.read(cx).value().to_string();
        let env = if env_text == self.initial.env_text {
            Ok(self.initial.original.env.clone())
        } else {
            super::modal_shared::parse_pairs(&env_text, false)
        }
        .map_err(|e| (2, e))?;
        let headers_text = self.headers_input.read(cx).value().to_string();
        let headers = if self.transport.is_remote() {
            if headers_text == self.initial.headers_text {
                Ok(self.initial.original.headers.clone())
            } else {
                super::modal_shared::parse_pairs(&headers_text, true)
            }
            .map_err(|e| (1, e))?
        } else {
            Default::default()
        };

        let args_text = self.args_input.read(cx).value().to_string();
        let args = match self.transport {
            McpTransport::Stdio => split_args(&args_text).map_err(|e| (1, e))?,
            _ => Vec::new(),
        };

        Ok(McpServerDraft {
            name: self.initial.name.clone(),
            transport: self.transport,
            command: match self.transport {
                McpTransport::Stdio => blank_to_none(&command_text),
                _ => None,
            },
            args,
            url: match self.transport {
                McpTransport::Sse | McpTransport::Http => blank_to_none(&url_text),
                _ => None,
            },
            env,
            headers: match self.transport {
                McpTransport::Sse | McpTransport::Http => headers,
                _ => Default::default(),
            },
            disabled: self.disabled,
            extra: self.initial.extra.clone(),
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let draft = match self.build_draft(cx) {
            Ok(d) => d,
            Err((field, msg)) => {
                self.error_field = Some(field);
                self.error = Some(msg);
                if let Some(input) = self.visible_inputs().get(field as usize) {
                    window.focus(&input.focus_handle(cx), cx);
                }
                cx.notify();
                return;
            }
        };
        let scope = self.initial.scope;
        let target_root = self.initial.project_root.clone();
        super::super::form_state::freeze_inputs(&self.inputs(), true, cx);
        self.error_field = None;
        self.submitting = true;
        cx.notify();

        if let Some(ws) = self.workspace.upgrade() {
            let me = cx.entity().downgrade();
            let window_handle = window.window_handle();
            // Defer the modal-side update so it lands in a fresh
            // entity update cycle (CLAUDE.md §4 — re-entering the same
            // modal entity from inside its own listener panics). Use
            // `update_window` to recover `&mut Window` for `dismiss`.
            ws.update(cx, |ws, cx_inner| {
                let result = ws.update_mcp_server(scope, target_root, draft, cx_inner);
                let me = me.clone();
                cx_inner.defer(move |cx| {
                    // SILENT-OK: window may be gone by the time this defer runs — with no window there is nothing to dismiss or show the save error on, so the no-op is correct.
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let Some(me) = me.upgrade() else { return };
                        match result {
                            Ok(()) => me.update(cx, |modal, cx| modal.dismiss(window, cx)),
                            Err(e) => me.update(cx, |modal, cx| {
                                modal.submitting = false;
                                super::super::form_state::freeze_inputs(&modal.inputs(), false, cx);
                                modal.error = Some(SharedString::from(e.to_string()));
                                cx.notify();
                            }),
                        }
                    });
                });
            });
        } else {
            self.submitting = false;
            super::super::form_state::freeze_inputs(&self.inputs(), false, cx);
            cx.notify();
        }
    }

    fn values(&self, cx: &App) -> Vec<String> {
        vec![
            self.transport.slug().to_string(),
            self.disabled.to_string(),
            self.command_input.read(cx).value().to_string(),
            self.args_input.read(cx).value().to_string(),
            self.url_input.read(cx).value().to_string(),
            self.env_input.read(cx).value().to_string(),
            self.headers_input.read(cx).value().to_string(),
        ]
    }

    fn visible_inputs(&self) -> Vec<Entity<InputState>> {
        if self.transport.is_remote() {
            vec![
                self.url_input.clone(),
                self.headers_input.clone(),
                self.env_input.clone(),
            ]
        } else {
            vec![
                self.command_input.clone(),
                self.args_input.clone(),
                self.env_input.clone(),
            ]
        }
    }

    fn inputs(&self) -> Vec<Entity<InputState>> {
        vec![
            self.command_input.clone(),
            self.args_input.clone(),
            self.url_input.clone(),
            self.env_input.clone(),
            self.headers_input.clone(),
        ]
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use super::super::form_state::{LeaveChoice, ask_to_leave};
        if self.submitting || self.form.asking {
            return;
        }
        let dirty = self.form.is_dirty(&self.values(cx));
        if !dirty {
            self.dismiss(window, cx);
            return;
        }
        ask_to_leave(
            &mut self.form,
            dirty,
            self.submitting,
            window,
            cx,
            |this, choice, window, cx| {
                this.form.asking = false;
                match choice {
                    LeaveChoice::Save => this.submit(window, cx),
                    LeaveChoice::Discard => this.dismiss(window, cx),
                    LeaveChoice::Continue => {}
                }
            },
        );
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    fn clear_error(&mut self, cx: &mut Context<Self>) {
        if self.error.is_some() {
            self.error = None;
            self.error_field = None;
            cx.notify();
        }
    }
}

fn forward_input(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<EditMcpServerModal>,
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

/// Multi-line input forwarder — Cmd+Enter submits, `Change` clears
/// the banner. Plain Enter is a newline; Escape is delivered through
/// Dialog's outer Cancel action.
fn forward_multi_input(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<EditMcpServerModal>,
) -> Subscription {
    cx.subscribe_in(
        state,
        window,
        |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { secondary } if *secondary => this.submit(window, cx),
            InputEvent::Change => this.clear_error(cx),
            _ => {}
        },
    )
}

impl Focusable for EditMcpServerModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // Land focus on the first input the active transport actually
        // renders — Stdio shows command, remote shows url. Returning
        // command unconditionally would focus an off-screen field when
        // editing an SSE / HTTP server.
        match self.transport {
            McpTransport::Stdio => self.command_input.focus_handle(cx),
            McpTransport::Sse | McpTransport::Http => self.url_input.focus_handle(cx),
        }
    }
}

impl ModalView for EditMcpServerModal {
    fn guards_close(&self) -> bool {
        true
    }
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(window, cx);
    }
}

impl Render for EditMcpServerModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
            .filter(|_| self.error_field.is_none())
            .map(|msg| crate::ui::alert::error("edit-tool-error", msg.clone()));

        // Read-only summary line: "Name @ Scope"
        let summary_text = t.text_body;
        let summary = div()
            .flex()
            .flex_row()
            .gap(px(theme::MCP_HEADER_GAP))
            .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
            .text_color(summary_text)
            .child(SharedString::from(self.initial.name.clone()))
            .child(SharedString::from(format!(
                "@ {}",
                scope_label(self.initial.scope)
            )));

        // Transport segmented selector. Children align with
        // `transport_values` by index; the click callback maps the
        // selected index back to its `McpTransport`.
        let transport_values: Vec<McpTransport> =
            transport_options().iter().map(|(tp, _)| *tp).collect();
        let transport_chip = button_group("transport-group", cx)
            .children(transport_options().into_iter().map(|(tp, label)| {
                button(
                    SharedString::from(format!("transport-{}", tp.slug())),
                    label,
                )
                .selected(self.transport == tp)
            }))
            .on_click(cx.listener(move |this, ixs: &Vec<usize>, _w, cx| {
                if this.submitting {
                    return;
                }
                if let Some(&ix) = ixs.first() {
                    this.transport = transport_values[ix];
                    this.clear_error(cx);
                    cx.notify();
                }
            }));

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::common::field_name(), &t))
            .child(summary)
            .child(field_label(strings::mcp::field_transport(), &t))
            .child(transport_chip);

        match self.transport {
            McpTransport::Stdio => {
                body = body
                    .child(field_label(strings::mcp::field_command(), &t))
                    .child(super::super::form_state::field_input(
                        &self.command_input,
                        cx,
                        0,
                        self.error_field,
                        self.error.as_ref(),
                    ))
                    .child(field_label(strings::mcp::field_args(), &t))
                    .child(super::super::form_state::field_input(
                        &self.args_input,
                        cx,
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
            McpTransport::Sse | McpTransport::Http => {
                body = body
                    .child(field_label(strings::mcp::field_url(), &t))
                    .child(super::super::form_state::field_input(
                        &self.url_input,
                        cx,
                        0,
                        self.error_field,
                        self.error.as_ref(),
                    ))
                    .child(field_label(strings::mcp::field_headers(), &t))
                    .child(super::super::form_state::field_input(
                        &self.headers_input,
                        cx,
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
        }

        // Env sits one past the last transport-specific input and the
        // disabled checkbox one past env, so the cycle ends at the
        // checkbox regardless of transport. Name is read-only here, so
        // indices start at 0 (both branches use 0,1 for their inputs).
        body = body
            .child(field_label(strings::mcp::field_env(), &t))
            .child(super::super::form_state::field_input(
                &self.env_input,
                cx,
                2,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(
                checkbox("mcp-edit-disabled", strings::mcp::field_disabled(), 3)
                    .checked(self.disabled)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        if this.submitting {
                            return;
                        }
                        this.disabled = *checked;
                        cx.notify();
                    })),
            );

        let save_label = if submitting {
            strings::mcp::saving_label()
        } else {
            strings::common::btn_save()
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("edit-mcp-cancel", strings::common::btn_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("edit-mcp-save", save_label)
                    .disabled(submitting)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("EditMcpServerModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(b) = banner {
            p = p.child(b);
        }
        p.child(footer)
    }
}

fn scope_label(scope: McpScope) -> String {
    match scope {
        McpScope::Project => strings::mcp::scope_project(),
        McpScope::Local => strings::mcp::scope_local(),
        McpScope::User => strings::mcp::scope_user(),
    }
}

pub fn open_edit_mcp_server_modal(
    ws: &mut Workspace,
    scope: McpScope,
    name: String,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let lane = ws.active_lane_root();
    let snapshot = cx
        .global::<crate::agent::mcp::McpState>()
        .snapshot_for(lane.as_deref(), &ws.mcp_project_dirs);
    let Some(initial) = EditMcpInitial::from_state(&snapshot, scope, &name, lane) else {
        return;
    };
    crate::workspace::dialog_helpers::open_form_modal(
        strings::mcp::edit_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| EditMcpServerModal::new(workspace, initial, window, cx),
        window,
        cx,
    );
}
