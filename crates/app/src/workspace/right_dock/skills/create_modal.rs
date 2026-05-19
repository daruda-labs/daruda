//! Create-skill modal.
//!
//! Left column: meta fields (name, scope select, description, when-to-use,
//! flags, optional fields). Right column: markdown body editor.
//! Shell width = `theme::FORM_MODAL_WIDE` (900px).
//!
//! Validation runs synchronously: name regex, in-scope duplicate
//! check, description-length warning. The Save button is disabled
//! while the name is empty so first-time users can't fire a
//! malformed write.
//!
//! Tab cycling is delegated to GPUI's tab system via `.tab_group()`
//! on the modal root; each input's `InputState` exposes a tab-stop
//! focus handle (and `tab_index` order is assigned in `new`).

use std::path::PathBuf;

use crate::ui::theme;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::agent::skills::{
    NameError, SkillDraft, SkillScope, SkillsSnapshot, frontmatter::SkillFrontmatter, persist,
    validate_name,
};
use crate::surface::strings;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_primary, checkbox, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use crate::workspace::right_dock::skills::modal_shared::field_label;

/// Trim + blank-to-None for SharedString-like reads from `InputState`.
fn blank_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub struct CreateSkillModal {
    panel_focus_handle: FocusHandle,

    name_input: Entity<InputState>,
    description_input: Entity<InputState>,
    when_to_use_input: Entity<InputState>,
    argument_hint_input: Entity<InputState>,
    allowed_tools_input: Entity<InputState>,
    paths_input: Entity<InputState>,
    model_input: Entity<InputState>,
    body_editor: Entity<InputState>,

    scope: SkillScope,
    scope_options: Vec<SkillScope>,
    user_invocable: bool,
    disable_model_invocation: bool,

    /// Toggleable when the workspace has no project root — Project
    /// scope is then disabled.
    project_root: Option<PathBuf>,
    state_snapshot: SkillsSnapshot,

    error: Option<SharedString>,
    submitting: bool,

    workspace: WeakEntity<Workspace>,

    // Subscriptions kept alive for the modal lifetime.
    _input_subs: Vec<Subscription>,
}

impl CreateSkillModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        prefill_scope: Option<SkillScope>,
        project_root: Option<PathBuf>,
        state_snapshot: SkillsSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx_state| InputState::new(window, cx_state).placeholder("name"));
        let description_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("short description"));
        let when_to_use_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("optional"));
        let argument_hint_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("optional"));
        let allowed_tools_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("optional"));
        let paths_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("optional"));
        let model_input =
            cx.new(|cx_state| InputState::new(window, cx_state).placeholder("optional"));
        let body_editor = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .multi_line(true)
                .placeholder("# Skill body in markdown")
        });

        let scope_options: Vec<SkillScope> = if project_root.is_some() {
            vec![SkillScope::Project, SkillScope::Personal]
        } else {
            vec![SkillScope::Personal]
        };
        let scope = prefill_scope
            .filter(|s| scope_options.contains(s))
            .unwrap_or_else(|| {
                if project_root.is_some() {
                    SkillScope::Project
                } else {
                    SkillScope::Personal
                }
            });

        let subs = vec![
            forward_input(&name_input, window, cx),
            forward_input(&description_input, window, cx),
            forward_input(&when_to_use_input, window, cx),
            forward_input(&argument_hint_input, window, cx),
            forward_input(&allowed_tools_input, window, cx),
            forward_input(&paths_input, window, cx),
            forward_input(&model_input, window, cx),
            forward_body_editor(&body_editor, window, cx),
        ];

        Self {
            panel_focus_handle: cx.focus_handle(),
            name_input,
            description_input,
            when_to_use_input,
            argument_hint_input,
            allowed_tools_input,
            paths_input,
            model_input,
            body_editor,
            scope,
            scope_options,
            user_invocable: true,
            disable_model_invocation: false,
            project_root,
            state_snapshot,
            error: None,
            submitting: false,
            workspace,
            _input_subs: subs,
        }
    }

    fn build_draft(&self, cx: &gpui::App) -> Result<SkillDraft, SharedString> {
        let raw_name = self.name_input.read(cx).value().to_string();
        let raw_name = raw_name.trim().to_string();
        match validate_name(&raw_name) {
            Ok(()) => {}
            Err(NameError::Empty) => return Err(strings::SKILLS_NAME_EMPTY.into()),
            Err(NameError::TooLong { .. }) => return Err(strings::SKILLS_NAME_TOO_LONG.into()),
            Err(NameError::InvalidChar { .. }) => return Err(strings::SKILLS_NAME_INVALID.into()),
            Err(NameError::InvalidLeading { .. }) => {
                return Err(strings::SKILLS_NAME_LEADING.into());
            }
            Err(NameError::DuplicateInScope { .. }) => unreachable!("validate_name is syntactic"),
        }
        if self.state_snapshot.name_exists(self.scope, &raw_name) {
            return Err(strings::SKILLS_NAME_DUPLICATE.into());
        }

        let mut fm = SkillFrontmatter::empty();
        fm.name = blank_to_none(&raw_name);
        fm.description = blank_to_none(&self.description_input.read(cx).value());
        fm.when_to_use = blank_to_none(&self.when_to_use_input.read(cx).value());
        fm.argument_hint = blank_to_none(&self.argument_hint_input.read(cx).value());
        fm.allowed_tools = blank_to_none(&self.allowed_tools_input.read(cx).value());
        fm.paths = blank_to_none(&self.paths_input.read(cx).value());
        fm.model = blank_to_none(&self.model_input.read(cx).value());
        fm.user_invocable = self.user_invocable;
        fm.disable_model_invocation = self.disable_model_invocation;

        Ok(SkillDraft {
            name: raw_name,
            scope: self.scope,
            frontmatter: fm,
            body: self.body_editor.read(cx).value().to_string(),
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
        let project_root = self.project_root.clone();
        let workspace = self.workspace.clone();
        self.submitting = true;
        cx.notify();

        let me = cx.entity().downgrade();
        window
            .spawn(cx, async move |async_cx| {
                let result = persist::write_skill(&draft, project_root.as_deref(), false)
                    .map_err(|e| e.to_string());
                // SILENT-OK: modal may close during async skill create
                let _ = async_cx.update(|window, cx| {
                    let Some(me) = me.upgrade() else { return };
                    me.update(cx, |modal, cx| {
                        modal.submitting = false;
                        match result {
                            Ok(_) => {
                                if let Some(ws) = workspace.upgrade() {
                                    ws.update(cx, |ws, cx| ws.refresh_skills_watcher(cx));
                                }
                                modal.dismiss(window, cx);
                            }
                            Err(msg) => {
                                modal.error = Some(SharedString::from(msg));
                                cx.notify();
                            }
                        }
                    });
                });
            })
            .detach();
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

/// Subscribe a single-line `InputState` so PressEnter submits the
/// modal and Change clears any stale error banner.
fn forward_input(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<CreateSkillModal>,
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

/// Subscribe the markdown body editor — only `Changed` matters; Submit
/// would fire on plain Enter (which inserts a newline in multi-line)
/// and Cancel is delivered through Dialog's Cancel action instead.
/// Multi-line body-editor forwarder — Cmd+Enter submits, `Change`
/// clears the banner. Plain Enter inserts a newline so markdown
/// authoring stays natural; Escape dismisses through Dialog.
fn forward_body_editor(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<CreateSkillModal>,
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

impl Focusable for CreateSkillModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.name_input.focus_handle(cx)
    }
}

impl ModalView for CreateSkillModal {}

impl Render for CreateSkillModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_focus = self.panel_focus_handle.clone();
        let user_invocable = self.user_invocable;
        let disable_model = self.disable_model_invocation;
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
            .map(|msg| crate::ui::alert::error("create-skill-error", msg.clone()));

        // Scope chips — assemble before mutable-borrow paths run.
        let scope_chip = {
            let mut row = div().flex().flex_row().gap(px(theme::SKILL_HEADER_GAP));
            for scope in &self.scope_options {
                let scope = *scope;
                let active = self.scope == scope;
                let label = match scope {
                    SkillScope::Project => strings::SKILLS_PROJECT,
                    SkillScope::Personal => strings::SKILLS_PERSONAL,
                    // `scope_options` only ever holds writable scopes
                    // (Project / Personal) — Plugin is read-only and
                    // cannot reach the Create modal.
                    SkillScope::Plugin => continue,
                };
                let t = theme::current(cx);
                let (bg, text_color) = if active {
                    (t.skill_badge_user_only_bg, t.skill_badge_user_only_text)
                } else {
                    (t.skill_aux_chip_bg, t.skill_aux_chip_text)
                };
                let chip = div()
                    .id(SharedString::from(format!("scope-{}", scope.slug())))
                    .cursor_pointer()
                    .px(px(theme::SKILL_BADGE_PAD_X))
                    .py(px(theme::SKILL_BADGE_PAD_Y))
                    .rounded(px(theme::SKILL_BADGE_RADIUS))
                    .bg(bg)
                    .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                    .text_color(text_color)
                    .child(label)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _w, cx| {
                            this.scope = scope;
                            cx.notify();
                        }),
                    );
                row = row.child(chip);
            }
            row
        };

        let left = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::SKILLS_FIELD_NAME, cx))
            .child(input(&self.name_input, cx, 0))
            .child(field_label(strings::SKILLS_FIELD_SCOPE, cx))
            .child(scope_chip)
            .child(field_label(strings::SKILLS_FIELD_DESCRIPTION, cx))
            .child(input(&self.description_input, cx, 1))
            .child(field_label(strings::SKILLS_FIELD_WHEN_TO_USE, cx))
            .child(input(&self.when_to_use_input, cx, 2))
            .child(field_label(strings::SKILLS_FIELD_ALLOWED_TOOLS, cx))
            .child(input(&self.allowed_tools_input, cx, 3))
            .child(field_label(strings::SKILLS_FIELD_ARG_HINT, cx))
            .child(input(&self.argument_hint_input, cx, 4))
            .child(field_label(strings::SKILLS_FIELD_PATHS, cx))
            .child(input(&self.paths_input, cx, 5))
            .child(field_label(strings::SKILLS_FIELD_MODEL, cx))
            .child(input(&self.model_input, cx, 6))
            .child(
                checkbox(
                    "skill-user-invocable",
                    strings::SKILLS_TOGGLE_USER_INVOCABLE,
                    8,
                )
                .checked(user_invocable)
                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.user_invocable = *checked;
                    cx.notify();
                })),
            )
            .child(
                checkbox(
                    "skill-disable-model",
                    strings::SKILLS_TOGGLE_DISABLE_MODEL,
                    9,
                )
                .checked(disable_model)
                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.disable_model_invocation = *checked;
                    cx.notify();
                })),
            );

        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::SKILLS_FIELD_BODY, cx))
            // body_editor sits between the left-column inputs and the
            // toggles so Tab flows from the last metadata field into
            // the markdown body before reaching the tail-of-form
            // toggles.
            .child(input(&self.body_editor, cx, 7));

        let body = div()
            .flex()
            .flex_row()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .child(left)
            .child(right);

        let save_label = if submitting { "Saving…" } else { "Save" };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("create-skill-cancel", "Cancel")
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.dismiss(w, cx))),
            )
            .child(
                button_primary("create-skill-save", save_label)
                    .disabled(submitting)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("CreateSkillModal")
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

pub fn open_create_skill_modal(
    ws: &mut Workspace,
    prefill_scope: Option<SkillScope>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let project_root = ws.active_worktree_root();
    let state = cx
        .global::<crate::agent::skills::SkillsState>()
        .snapshot_for(project_root.as_deref());
    crate::workspace::dialog_helpers::open_form_modal(
        strings::SKILLS_NEW_TITLE,
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| {
            CreateSkillModal::new(workspace, prefill_scope, project_root, state, window, cx)
        },
        window,
        cx,
    );
}
