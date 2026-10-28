//! Paste-first MCP creation. Review a connection, then save to a captured scope.
//! Parsing never runs commands; hidden source fields remain in the draft.

use std::path::PathBuf;

use crate::ui::theme;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use super::modal_shared::{field_error_to_msg, field_label, split_args, transport_options};
use crate::agent::mcp::{
    McpScope, McpServerDraft, McpTransport, NameError, validate_command, validate_name,
    validate_url,
};
use crate::surface::strings;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_primary, checkbox};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use daruda_config::env_lines::format_env_lines;

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
    pub(crate) targets: Vec<crate::agent::configuration::library::Target>,
    pub(crate) config: Option<std::sync::Arc<crate::agent::configuration::Configuration>>,
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
    scope_choice: super::super::form_layout::Choice<McpScope>,
    transport_choice: super::super::form_layout::Choice<McpTransport>,
    transport: McpTransport,
    disabled: bool,
    auth: super::auth::Auth,
    codex_options: super::codex_options::CodexOptions,
    args_rows: Entity<super::rows::Rows>,
    env_rows: Entity<super::rows::Rows>,
    header_rows: Entity<super::rows::Rows>,

    initial: AddMcpInitial,
    paste_input: Entity<InputState>,
    pasting: bool,
    candidates: Vec<McpServerDraft>,
    imported: Option<McpServerDraft>,
    advanced: bool,
    duplicate: bool,

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
        let paste_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(6, 12)
                .placeholder(strings::mcp::quick_placeholder())
        });
        let name_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::mcp::placeholder_name())
        });
        let auth = super::auth::Auth::new(window, cx);
        let codex_options = super::codex_options::CodexOptions::new(window, cx);
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
        let mut scope_options: Vec<McpScope> = if initial.project_root.is_some() {
            vec![McpScope::Project, McpScope::Local, McpScope::User]
        } else {
            vec![McpScope::User]
        };
        if initial
            .config
            .as_ref()
            .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex)
        {
            scope_options.retain(|s| *s != McpScope::Local);
        }
        let scope = if scope_options.contains(&initial.default_scope) {
            initial.default_scope
        } else {
            McpScope::User
        };

        let scope_choice = super::super::form_layout::Choice::new(
            super::modal_shared::scope_options()
                .into_iter()
                .filter(|(value, _)| scope_options.contains(value))
                .collect(),
            scope,
            window,
            cx,
            |this, value, cx| {
                if !this.submitting {
                    this.scope = value;
                    this.clear_error(cx);
                    cx.notify();
                }
            },
        );
        let transport_choice = super::super::form_layout::Choice::new(
            transport_options()
                .into_iter()
                .filter(|(value, _)| {
                    *value != McpTransport::Sse
                        || initial.config.as_ref().is_none_or(|c| {
                            c.provider == crate::agent::configuration::Provider::Claude
                        })
                })
                .map(|(value, label)| (value, label.to_uppercase()))
                .collect(),
            McpTransport::Stdio,
            window,
            cx,
            |this, value, cx| {
                if !this.submitting {
                    this.transport = value;
                    this.clear_error(cx);
                    cx.notify();
                }
            },
        );
        let mut subs = vec![
            forward_input(&auth.input, window, cx),
            forward_multi_input(&paste_input, window, cx),
            forward_input(&name_input, window, cx),
            forward_input(&command_input, window, cx),
            forward_input(&args_input, window, cx),
            forward_input(&url_input, window, cx),
            forward_multi_input(&env_input, window, cx),
            forward_multi_input(&headers_input, window, cx),
        ];

        subs.extend(
            codex_options
                .inputs
                .iter()
                .map(|input| forward_multi_input(input, window, cx)),
        );
        let args_rows = cx.new(|cx| {
            super::rows::Rows::new(args_input.clone(), super::rows::Kind::Arguments, window, cx)
        });
        let env_rows = cx.new(|cx| {
            super::rows::Rows::new(env_input.clone(), super::rows::Kind::Pairs, window, cx)
        });
        let header_rows = cx.new(|cx| {
            super::rows::Rows::new(headers_input.clone(), super::rows::Kind::Pairs, window, cx)
        });
        let mut modal = Self {
            paste_input,
            pasting: true,
            candidates: Vec::new(),
            imported: None,
            advanced: false,
            duplicate: false,
            auth,
            codex_options,
            args_rows,
            env_rows,
            header_rows,
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
            scope_choice,
            transport_choice,
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
        let env = if let Some(d) = self
            .imported
            .as_ref()
            .filter(|d| format_env_lines(&d.env) == env_text)
        {
            d.env.clone()
        } else {
            self.env_rows
                .read(cx)
                .pairs(false, cx)
                .map_err(|e| (3, e))?
        };
        let headers_text = self.headers_input.read(cx).value().to_string();
        let mut headers = if self.transport.is_remote() {
            if let Some(d) = self
                .imported
                .as_ref()
                .filter(|d| format_env_lines(&d.headers) == headers_text)
            {
                d.headers.clone()
            } else {
                self.header_rows
                    .read(cx)
                    .pairs(true, cx)
                    .map_err(|e| (2, e))?
            }
        } else {
            Default::default()
        };

        let mut extra = self
            .imported
            .as_ref()
            .map(|d| d.extra.clone())
            .unwrap_or_default();
        if self
            .initial
            .config
            .as_ref()
            .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex)
        {
            self.codex_options
                .apply(&mut extra, cx)
                .map_err(|(index, error)| (-(index as isize) - 2, error))?;
        }
        if self.transport.is_remote() {
            if self
                .initial
                .config
                .as_ref()
                .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex)
            {
                self.auth
                    .apply_codex(&mut headers, &mut extra, cx)
                    .map_err(|e| (-1, e))?;
            } else {
                self.auth.apply(&mut headers, cx).map_err(|e| (-1, e))?;
            }
        }

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
            extra,
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        if self.pasting {
            self.review_paste(window, cx);
            return;
        }
        let draft = match self.build_draft(cx) {
            Ok(d) => d,
            Err((field, msg)) => {
                self.advanced |= field < -1 || (field == 2 && self.transport.is_remote());
                self.error_field = (field >= 0).then_some(field);
                if field < -1 {
                    if let Some(input) = self.codex_options.inputs.get((-field - 2) as usize) {
                        window.focus(&input.focus_handle(cx), cx);
                    }
                } else if field == -1 {
                    window.focus(&self.auth.input.focus_handle(cx), cx);
                }
                self.error = Some(msg);
                if let Some(input) = self.visible_inputs().get(field as usize) {
                    window.focus(&input.focus_handle(cx), cx);
                }
                let row_focus = if field == 3 {
                    self.env_rows.read(cx).focus_handle(false, cx)
                } else if field == 2 && self.transport.is_remote() {
                    self.header_rows.read(cx).focus_handle(true, cx)
                } else if field == 2 {
                    self.args_rows.read(cx).focus_handle(false, cx)
                } else {
                    None
                };
                if let Some(focus) = row_focus {
                    window.focus(&focus, cx);
                }
                cx.notify();
                return;
            }
        };
        let scope = self.scope;
        let saved_name = draft.name.clone();
        let saved_disabled = draft.disabled;
        let saved_provider = self.initial.config.as_ref().map(|c| c.provider);
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
            // already being updated" (AGENTS.md §4). Defer the modal
            // update so it runs on a fresh update cycle. The deferred
            // closure receives only `&mut App`, so we re-enter the
            // window via `update_window(handle, ...)` to recover the
            // `&mut Window` needed for `dismiss`.
            ws.update(cx, |ws, cx_inner| {
                let result = if let Some(config) = &self.initial.config {
                    let target = self
                        .initial
                        .targets
                        .iter()
                        .find(|t| t.scope == scope)
                        .cloned()
                        .or_else(|| {
                            crate::agent::configuration::library::Target::new(config, scope, None)
                        });
                    let result = target
                        .ok_or(crate::agent::mcp::McpPersistError::NoProjectRoot)
                        .and_then(|t| t.mutate(&draft.name, None, Some(&draft)));
                    ws.refresh_library(cx_inner);
                    result
                } else {
                    ws.add_mcp_server(scope, target_root, draft, cx_inner)
                };
                let me = me.clone();
                cx_inner.defer(move |cx| {
                    // SILENT-OK: window may be gone by the time this defer runs — with no window there is nothing to dismiss or show the save error on, so the no-op is correct.
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let Some(me) = me.upgrade() else { return };
                        match result {
                            Ok(()) => {
                                me.update(cx, |modal, cx| modal.dismiss(window, cx));
                                super::saved::open(
                                    saved_name,
                                    saved_provider,
                                    saved_disabled,
                                    window,
                                    cx,
                                );
                            }
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
        let mut values = vec![
            self.paste_input.read(cx).value().to_string(),
            self.scope.slug().to_string(),
            self.transport.slug().to_string(),
            self.disabled.to_string(),
            self.name_input.read(cx).value().to_string(),
            self.command_input.read(cx).value().to_string(),
            self.args_input.read(cx).value().to_string(),
            self.url_input.read(cx).value().to_string(),
            self.env_input.read(cx).value().to_string(),
            self.headers_input.read(cx).value().to_string(),
        ];
        values.extend(self.auth.values(cx));
        values.extend(self.codex_options.values(cx));
        values
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
        let mut inputs = vec![
            self.auth.input.clone(),
            self.paste_input.clone(),
            self.name_input.clone(),
            self.command_input.clone(),
            self.args_input.clone(),
            self.url_input.clone(),
            self.env_input.clone(),
            self.headers_input.clone(),
        ];
        inputs.extend(self.codex_options.inputs.clone());
        inputs
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
            InputEvent::Change => {
                if this.pasting {
                    this.candidates.clear();
                }
                this.clear_error(cx);
                cx.notify();
            }
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
            InputEvent::Change => {
                if this.pasting {
                    this.candidates.clear();
                }
                this.clear_error(cx);
                cx.notify();
            }
            _ => {}
        },
    )
}

impl Focusable for AddMcpServerModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        if self.pasting {
            self.paste_input.focus_handle(cx)
        } else {
            self.name_input.focus_handle(cx)
        }
    }
}

impl ModalView for AddMcpServerModal {
    fn render_footer(&mut self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        vec![self.form_footer(cx)]
    }
    fn fit_viewport(&self) -> bool {
        true
    }
    fn guards_close(&self) -> bool {
        true
    }
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(window, cx);
    }
}

mod quick_entry;
mod render;

pub fn open_add_mcp_server_modal(
    ws: &mut Workspace,
    prefill_scope: Option<McpScope>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    open_add(ws, prefill_scope, None, window, cx);
}

pub(super) fn open_add_mcp_server_in_target(
    ws: &mut Workspace,
    target: crate::agent::configuration::library::Target,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    open_add(ws, Some(target.scope), Some(target), window, cx);
}

fn open_add(
    ws: &mut Workspace,
    prefill_scope: Option<McpScope>,
    target: Option<crate::agent::configuration::library::Target>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let Some(config) = ws.pin_library() else {
        return;
    };
    // A target rendered for an earlier library snapshot no longer names a
    // writable file; the click still opens the form for the scope it showed.
    let target = target.filter(|target| target.config == *config && target.owner.is_none());
    let snapshot = ws.right_views.library.view.inventory.mcp.clone();
    let existing_names = creation_names(
        &ws.right_views.library.view.inventory.groups,
        &config,
        target.as_ref(),
    );
    let mut targets: Vec<_> = ws
        .right_views
        .library
        .view
        .inventory
        .groups
        .iter()
        .filter(|g| g.target.owner.is_none())
        .map(|g| g.target.clone())
        .collect();
    // The first target of a scope is the form's save destination. Preserve
    // the clicked source when several project configuration files exist.
    if let Some(target) = target {
        targets.retain(|candidate| *candidate != target);
        targets.insert(0, target);
    }
    let initial = AddMcpInitial {
        targets,
        config: Some(config),
        default_scope: prefill_scope.unwrap_or(McpScope::User),
        project_root: snapshot.project_root.clone(),
        existing_names,
    };
    crate::workspace::dialog_helpers::open_form_modal(
        strings::mcp::new_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| AddMcpServerModal::new(workspace, initial, window, cx),
        window,
        cx,
    );
}

fn creation_names(
    groups: &[crate::agent::configuration::library::Group],
    config: &crate::agent::configuration::Configuration,
    selected: Option<&crate::agent::configuration::library::Target>,
) -> Vec<(McpScope, String)> {
    let project_file = selected
        .filter(|target| target.scope == McpScope::Project)
        .map(|target| target.path.clone())
        .or_else(|| config.cwd.as_ref().map(|cwd| config.project_file(cwd)));
    groups
        .iter()
        .filter(|group| {
            group.target.owner.is_none()
                && (group.target.scope != McpScope::Project
                    || project_file.as_ref() == Some(&group.target.path))
        })
        .flat_map(|group| {
            group
                .servers
                .iter()
                .map(|server| (server.scope, server.name.clone()))
        })
        .collect()
}

impl AddMcpServerModal {
    pub(super) fn seed_duplicate(
        &mut self,
        draft: McpServerDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_import(draft, window, cx);
        self.duplicate = true;
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "screenshot")]
impl AddMcpServerModal {
    pub(in crate::workspace) fn seed_stdio_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_input.update(cx, |input, cx| input.set_value(
            r#"{"mcpServers":{"docs":{"command":"npx","args":["-y","@example/docs-mcp"],"env":{"DOCS_TOKEN":"example-token"}}}}"#,
            window, cx));
        self.review_paste(window, cx);
    }

    pub(in crate::workspace) fn seed_error_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seed_for_shot(true, window, cx);
        self.name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let modal = cx.entity();
        window.defer(cx, move |window, cx| {
            modal.update(cx, |modal, cx| modal.submit(window, cx));
        });
    }

    pub(in crate::workspace) fn seed_for_shot(
        &mut self,
        review: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_input.update(cx,|input,cx|input.set_value(
            r#"{"mcpServers":{"docs":{"type":"http","url":"https://docs.example.com/mcp","headers":{"Authorization":"Bearer example-token"}}}}"#,window,cx));
        if review {
            self.review_paste(window, cx);
        }
    }
}

#[cfg(feature = "screenshot")]
impl AddMcpServerModal {
    pub(in crate::workspace) fn seed_codex_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_input.update(cx,|input,cx| input.set_value("[mcp_servers.docs]\nurl = 'https://docs.example.com/mcp'\nbearer_token_env_var = 'DOCS_TOKEN'\nstartup_timeout_sec = 20\nenabled_tools = ['search', 'read']\n",window,cx));
        self.review_paste(window, cx);
        self.advanced = true;
    }
}
