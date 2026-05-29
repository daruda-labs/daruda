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
    chip_button, field_error_to_msg, field_label, scope_options, split_args, transport_options,
};
use crate::agent::mcp::{
    McpScope, McpServerDraft, McpTransport, NameError, parse_env_lines, validate_command,
    validate_name, validate_url,
};
use crate::surface::strings;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_primary, checkbox, input};
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
        let name_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("filesystem"));
        let command_input = cx.new(|cx_state| InputState::new(window, cx_state).placeholder("npx"));
        let args_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder("-y @modelcontextprotocol/server-x")
        });
        let url_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder("https://localhost:8080/mcp")
        });
        let env_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .multi_line(true)
                .placeholder("FOO=bar")
        });
        let headers_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .multi_line(true)
                .placeholder("Authorization=Bearer ...")
        });

        let scope_options: Vec<McpScope> = if initial.project_root.is_some() {
            vec![McpScope::Project, McpScope::Personal]
        } else {
            vec![McpScope::Personal]
        };
        let scope = if scope_options.contains(&initial.default_scope) {
            initial.default_scope
        } else {
            McpScope::Personal
        };

        let subs = vec![
            forward_input(&name_input, window, cx),
            forward_input(&command_input, window, cx),
            forward_input(&args_input, window, cx),
            forward_input(&url_input, window, cx),
            forward_multi_input(&env_input, window, cx),
            forward_multi_input(&headers_input, window, cx),
        ];

        Self {
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
            submitting: false,
            workspace,
            _input_subs: subs,
        }
    }

    fn build_draft(&self, cx: &gpui::App) -> Result<McpServerDraft, SharedString> {
        let raw_name = self.name_input.read(cx).value().to_string();
        let raw_name = raw_name.trim().to_string();
        match validate_name(&raw_name) {
            Ok(()) => {}
            Err(NameError::Empty) => return Err(strings::mcp_name_empty().into()),
            Err(NameError::TooLong { .. }) => return Err(strings::mcp_name_too_long().into()),
            Err(NameError::InvalidChar { .. }) => return Err(strings::mcp_name_invalid().into()),
            Err(NameError::InvalidLeading { .. }) => return Err(strings::mcp_name_leading().into()),
            Err(NameError::DuplicateInScope { .. }) => unreachable!("validate_name is syntactic"),
        }
        if self
            .initial
            .existing_names
            .iter()
            .any(|(scope, name)| *scope == self.scope && name == &raw_name)
        {
            return Err(strings::mcp_name_duplicate().into());
        }

        let command_text = self.command_input.read(cx).value().to_string();
        let url_text = self.url_input.read(cx).value().to_string();
        if let Err(e) = validate_command(&command_text, self.transport) {
            return Err(field_error_to_msg(e));
        }
        if let Err(e) = validate_url(&url_text, self.transport) {
            return Err(field_error_to_msg(e));
        }

        let env_text = self.env_input.read(cx).value().to_string();
        let env = parse_env_lines(&env_text).map_err(field_error_to_msg)?;
        let headers_text = self.headers_input.read(cx).value().to_string();
        let headers = parse_env_lines(&headers_text).map_err(field_error_to_msg)?;

        let args_text = self.args_input.read(cx).value().to_string();
        let args = match self.transport {
            McpTransport::Stdio => split_args(&args_text),
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
            Err(msg) => {
                self.error = Some(msg);
                cx.notify();
                return;
            }
        };
        let scope = self.scope;
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
                let result = ws.add_mcp_server(scope, draft, cx_inner);
                let me = me.clone();
                cx_inner.defer(move |cx| {
                    // SILENT-OK: modal may close during async tool save
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let Some(me) = me.upgrade() else { return };
                        match result {
                            Ok(()) => me.update(cx, |modal, cx| modal.dismiss(window, cx)),
                            Err(e) => me.update(cx, |modal, cx| {
                                modal.submitting = false;
                                modal.error = Some(SharedString::from(e.to_string()));
                                cx.notify();
                            }),
                        }
                    });
                });
            });
        } else {
            self.submitting = false;
            cx.notify();
        }
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

impl ModalView for AddMcpServerModal {}

impl Render for AddMcpServerModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
            .map(|msg| crate::ui::alert::error("add-tool-error", msg.clone()));

        // Scope chip strip.
        let scope_chip = {
            let mut row = div().flex().flex_row().gap(px(theme::MCP_HEADER_GAP));
            for (scope, label) in scope_options() {
                if !self.scope_options.contains(&scope) {
                    continue;
                }
                let active = self.scope == scope;
                row = row.child(chip_button(
                    SharedString::from(format!("scope-{}", scope.slug())),
                    label,
                    active,
                    cx,
                    cx.listener(move |this, _, _w, cx| {
                        this.scope = scope;
                        cx.notify();
                    }),
                ));
            }
            row
        };

        // Transport chip strip.
        let transport_chip = {
            let mut row = div().flex().flex_row().gap(px(theme::MCP_HEADER_GAP));
            for (transport, label) in transport_options() {
                let active = self.transport == transport;
                row = row.child(chip_button(
                    SharedString::from(format!("transport-{}", transport.slug())),
                    label,
                    active,
                    cx,
                    cx.listener(move |this, _, _w, cx| {
                        this.transport = transport;
                        cx.notify();
                    }),
                ));
            }
            row
        };

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::mcp_field_name()))
            .child(input(&self.name_input, cx, 0))
            .child(field_label(strings::mcp_field_scope()))
            .child(scope_chip)
            .child(field_label(strings::mcp_field_transport()))
            .child(transport_chip);

        match self.transport {
            McpTransport::Stdio => {
                body = body
                    .child(field_label(strings::mcp_field_command()))
                    .child(input(&self.command_input, cx, 1))
                    .child(field_label(strings::mcp_field_args()))
                    .child(input(&self.args_input, cx, 2));
            }
            McpTransport::Sse | McpTransport::Http => {
                body = body
                    .child(field_label(strings::mcp_field_url()))
                    .child(input(&self.url_input, cx, 1))
                    .child(field_label(strings::mcp_field_headers()))
                    .child(input(&self.headers_input, cx, 2));
            }
        }

        // Disabled checkbox slot — its tab_index sits one past the last
        // transport-specific input so it lands at the end of the cycle
        // regardless of the active transport (Stdio uses 1,2; remote
        // uses 1,2 for url+headers — env uses 3 across both branches).
        body = body
            .child(field_label(strings::mcp_field_env()))
            .child(input(&self.env_input, cx, 3))
            .child(
                checkbox("mcp-disabled", strings::mcp_field_disabled(), 4)
                    .checked(self.disabled)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        this.disabled = *checked;
                        cx.notify();
                    })),
            );

        let save_label = if submitting {
            strings::mcp_saving_label()
        } else {
            strings::mcp_button_add()
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("add-mcp-cancel", strings::mcp_button_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.dismiss(w, cx))),
            )
            .child(
                button_primary("add-mcp-save", save_label)
                    .disabled(submitting)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("AddMcpServerModal")
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
        .snapshot_for(lane.as_deref());
    let initial = AddMcpInitial {
        default_scope: prefill_scope.unwrap_or(McpScope::Personal),
        project_root: snapshot.project_root.clone(),
        existing_names: snapshot.all_names(),
    };
    crate::workspace::dialog_helpers::open_form_modal(
        strings::mcp_new_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| AddMcpServerModal::new(workspace, initial, window, cx),
        window,
        cx,
    );
}
