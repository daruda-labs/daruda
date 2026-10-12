//! Add-MCP-server modal.
//!
//! UI-FM-based form modal: name + scope + transport + transport-specific
//! fields (command/args for stdio, url/headers for sse/http) + env +
//! disabled toggle. Snapshot opener pattern: `AddMcpInitial` is built by
//! `super::open_add_mcp_server_modal` so the modal's `new()` never
//! re-enters the workspace entity.
//!
//! Tab navigation: GPUI `.tab_group()` on the panel root + per-input
//! `tab_index`. The transport chip swaps which inputs are rendered but
//! the unused inputs' focus handles simply sit out of paint and never
//! receive focus.

use std::path::PathBuf;

use crate::ui::theme;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use super::modal_shared::{
    field_error_to_msg, field_label, scope_options, split_args, transport_options,
};
use crate::agent::mcp::{
    McpScope, McpServerDraft, McpTransport, NameError, validate_command, validate_name,
    validate_url,
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

/// Snapshot built by `Workspace::open_add_mcp_server_modal` and passed
/// in to the modal's constructor. The modal never reads the workspace
/// — every value it needs is captured here.
#[derive(Clone, Debug)]
pub struct AddMcpInitial {
    pub default_scope: McpScope,
    pub project_root: Option<PathBuf>,
    /// `(scope, name)` pairs of every server already in either scope —
    /// drives the "duplicate name" banner.
    pub existing_names: Vec<(McpScope, String)>,
}

pub struct AddMcpServerModal {
    panel_focus_handle: FocusHandle,
    form: super::super::form_state::FormState,

    name_input: Entity<InputState>,
    command_input: Entity<InputState>,
    args_input: Entity<InputState>,
    url_input: Entity<InputState>,
    env_input: Entity<InputState>,
    headers_input: Entity<InputState>,

    scope: McpScope,
    scope_options: Vec<McpScope>,
    transport: McpTransport,
    disabled: bool,

    initial: AddMcpInitial,

    error: Option<SharedString>,
    error_field: Option<isize>,
    submitting: bool,

    workspace: WeakEntity<Workspace>,
    _input_subs: Vec<Subscription>,
}

impl AddMcpServerModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        initial: AddMcpInitial,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::mcp::placeholder_name())
        });
        let command_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::mcp::placeholder_command())
        });
        let args_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::mcp::placeholder_args())
        });
        let url_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::mcp::placeholder_url())
        });
        let env_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .auto_grow(2, 6)
                .placeholder(strings::mcp::placeholder_env())
        });
        let headers_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .auto_grow(2, 6)
                .placeholder(strings::mcp::placeholder_headers())
        });

        // Project + Local require an active lane root; User is always
        // available. Order mirrors the Tools-tab section order.
        let scope_options: Vec<McpScope> = if initial.project_root.is_some() {
            vec![McpScope::Project, McpScope::Local, McpScope::User]
        } else {
            vec![McpScope::User]
        };
        let scope = if scope_options.contains(&initial.default_scope) {
            initial.default_scope
        } else {
            McpScope::User
        };

        let subs = vec![
            forward_input(&name_input, window, cx),
            forward_input(&command_input, window, cx),
            forward_input(&args_input, window, cx),
            forward_input(&url_input, window, cx),
            forward_multi_input(&env_input, window, cx),
            forward_multi_input(&headers_input, window, cx),
        ];

        let mut modal = Self {
            form: Default::default(),
            panel_focus_handle: cx.focus_handle(),
            name_input,
            command_input,
            args_input,
            url_input,
            env_input,
            headers_input,
            scope,
            scope_options,
            transport: McpTransport::Stdio,
            disabled: false,
            initial,
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
        let raw_name = self.name_input.read(cx).value().to_string();
        let raw_name = raw_name.trim().to_string();
        match validate_name(&raw_name) {
            Ok(()) => {}
            Err(NameError::Empty) => return Err((0, strings::common::name_required().into())),
            Err(NameError::TooLong { .. }) => {
                return Err((0, strings::mcp::name_too_long().into()));
            }
            Err(NameError::InvalidChar { .. }) => {
                return Err((0, strings::mcp::name_invalid().into()));
            }
            Err(NameError::InvalidLeading { .. }) => {
                return Err((0, strings::mcp::name_leading().into()));
            }
            Err(NameError::DuplicateInScope { .. }) => unreachable!("validate_name is syntactic"),
        }
        if self
            .initial
            .existing_names
            .iter()
            .any(|(scope, name)| *scope == self.scope && name == &raw_name)
        {
            return Err((0, strings::mcp::name_duplicate().into()));
        }

        let command_text = self.command_input.read(cx).value().to_string();
        let url_text = self.url_input.read(cx).value().to_string();
        if let Err(e) = validate_command(&command_text, self.transport) {
            return Err((1, field_error_to_msg(e)));
        }
        if let Err(e) = validate_url(&url_text, self.transport) {
            return Err((1, field_error_to_msg(e)));
        }

        let env_text = self.env_input.read(cx).value().to_string();
        let env = super::modal_shared::parse_pairs(&env_text, false).map_err(|e| (3, e))?;
        let headers_text = self.headers_input.read(cx).value().to_string();
        let headers = if self.transport.is_remote() {
            super::modal_shared::parse_pairs(&headers_text, true).map_err(|e| (2, e))?
        } else {
            Default::default()
        };

        let args_text = self.args_input.read(cx).value().to_string();
        let args = match self.transport {
            McpTransport::Stdio => split_args(&args_text).map_err(|e| (2, e))?,
            _ => Vec::new(),
        };

        Ok(McpServerDraft {
            name: raw_name,
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
            extra: Default::default(),
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
        let scope = self.scope;
        let target_root = self.initial.project_root.clone();
        super::super::form_state::freeze_inputs(&self.inputs(), true, cx);
        self.error_field = None;
        self.submitting = true;
        cx.notify();

        if let Some(ws) = self.workspace.upgrade() {
            let me = cx.entity().downgrade();
            let window_handle = window.window_handle();
            // `submit` runs inside `cx.listener` — i.e. while this
            // modal entity is already being updated. Calling
            // `me.update(...)` synchronously inside the success/error
            // branch panics with "cannot update <Modal> while it is
            // already being updated" (CLAUDE.md §4). Defer the modal
            // update so it runs on a fresh update cycle. The deferred
            // closure receives only `&mut App`, so we re-enter the
            // window via `update_window(handle, ...)` to recover the
            // `&mut Window` needed for `dismiss`.
            ws.update(cx, |ws, cx_inner| {
                let result = ws.add_mcp_server(scope, target_root, draft, cx_inner);
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
            self.scope.slug().to_string(),
            self.transport.slug().to_string(),
            self.disabled.to_string(),
            self.name_input.read(cx).value().to_string(),
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
                self.name_input.clone(),
                self.url_input.clone(),
                self.headers_input.clone(),
                self.env_input.clone(),
            ]
        } else {
            vec![
                self.name_input.clone(),
                self.command_input.clone(),
                self.args_input.clone(),
                self.env_input.clone(),
            ]
        }
    }

    fn inputs(&self) -> Vec<Entity<InputState>> {
        vec![
            self.name_input.clone(),
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
    cx: &mut Context<AddMcpServerModal>,
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

/// Subscribe a multi-line `InputState` (env / headers) so Cmd+Enter
/// submits and `Change` clears the validation banner. Plain Enter
/// inserts a newline because `multi_line(true)` is set; Escape is
/// handled by Dialog's outer Cancel action.
fn forward_multi_input(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<AddMcpServerModal>,
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

impl Focusable for AddMcpServerModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.name_input.focus_handle(cx)
    }
}

impl ModalView for AddMcpServerModal {
    fn guards_close(&self) -> bool {
        true
    }
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(window, cx);
    }
}

mod render;

pub fn open_add_mcp_server_modal(
    ws: &mut Workspace,
    prefill_scope: Option<McpScope>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let lane = ws.active_lane_root();
    let snapshot = cx
        .global::<crate::agent::mcp::McpState>()
        .snapshot_for(lane.as_deref(), &ws.mcp_project_dirs);
    let initial = AddMcpInitial {
        default_scope: prefill_scope.unwrap_or(McpScope::User),
        project_root: snapshot.project_root.clone(),
        existing_names: snapshot.all_names(),
    };
    crate::workspace::dialog_helpers::open_form_modal(
        strings::mcp::new_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| AddMcpServerModal::new(workspace, initial, window, cx),
        window,
        cx,
    );
}
