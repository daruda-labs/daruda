//! Paste-first skill creation with a body-first form and optional metadata.
//! Captures the destination and preserves imported source until explicit Save.

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
use crate::ui::Selectable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_group, button_primary, checkbox};
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
    form: super::super::form_state::FormState,

    name_input: Entity<InputState>,
    invocation_input: Entity<InputState>,
    paste_input: Entity<InputState>,
    pasting: bool,
    advanced: bool,
    imported: Option<crate::agent::skills::import::ImportedSkill>,
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
    error_field: Option<isize>,
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
        let paste_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(6, 12)
                .placeholder(strings::skills::quick_placeholder())
        });
        let invocation_input = cx.new(|cx| InputState::new(window, cx));
        let name_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::skills::placeholder_name())
        });
        let description_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(strings::skills::placeholder_description())
        });
        let when_to_use_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(strings::skills::placeholder_when_to_use())
        });
        let argument_hint_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::skills::placeholder_optional())
        });
        let allowed_tools_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::skills::placeholder_optional())
        });
        let paths_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::skills::placeholder_optional())
        });
        let model_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(strings::skills::placeholder_optional())
        });
        let body_editor = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .auto_grow(8, 12)
                .placeholder(strings::skills::placeholder_body())
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
            forward_body_editor(&paste_input, window, cx),
            forward_input(&invocation_input, window, cx),
            forward_input(&name_input, window, cx),
            forward_input(&description_input, window, cx),
            forward_input(&when_to_use_input, window, cx),
            forward_input(&argument_hint_input, window, cx),
            forward_input(&allowed_tools_input, window, cx),
            forward_input(&paths_input, window, cx),
            forward_input(&model_input, window, cx),
            forward_body_editor(&body_editor, window, cx),
        ];

        let mut modal = Self {
            paste_input,
            invocation_input,
            pasting: true,
            advanced: false,
            imported: None,
            form: Default::default(),
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
            error_field: None,
            submitting: false,
            workspace,
            _input_subs: subs,
        };
        modal.form.baseline = modal.values(cx);
        modal
    }

    fn build_draft(&self, cx: &gpui::App) -> Result<SkillDraft, (isize, SharedString)> {
        let raw_name = self.name_input.read(cx).value().to_string();
        let raw_name = raw_name.trim().to_string();
        match validate_name(&raw_name) {
            Ok(()) => {}
            Err(NameError::Empty) => return Err((0, strings::common::name_required().into())),
            Err(NameError::TooLong { .. }) => {
                return Err((0, strings::skills::name_too_long().into()));
            }
            Err(NameError::InvalidChar { .. }) => {
                return Err((0, strings::skills::name_invalid().into()));
            }
            Err(NameError::InvalidLeading { .. }) => {
                return Err((0, strings::skills::name_leading().into()));
            }
            Err(NameError::DuplicateInScope { .. }) => unreachable!("validate_name is syntactic"),
        }
        if self.state_snapshot.name_exists(self.scope, &raw_name) {
            return Err((0, strings::skills::name_duplicate().into()));
        }

        let mut fm = self
            .imported
            .as_ref()
            .map(|i| i.frontmatter.clone())
            .unwrap_or_else(SkillFrontmatter::empty);
        let value = self.invocation_input.read(cx).value();
        if value.as_str() != fm.name.as_deref().unwrap_or("") {
            fm.name = blank_to_none(&value);
        }
        let value = self.description_input.read(cx).value();
        if value.as_str() != fm.description.as_deref().unwrap_or("") {
            fm.description = blank_to_none(&value);
        }
        let value = self.when_to_use_input.read(cx).value();
        if value.as_str() != fm.when_to_use.as_deref().unwrap_or("") {
            fm.when_to_use = blank_to_none(&value);
        }
        let value = self.argument_hint_input.read(cx).value();
        if value.as_str() != fm.argument_hint.as_deref().unwrap_or("") {
            fm.argument_hint = blank_to_none(&value);
        }
        let value = self.allowed_tools_input.read(cx).value();
        if value.as_str() != fm.allowed_tools.as_deref().unwrap_or("") {
            fm.allowed_tools = blank_to_none(&value);
        }
        let value = self.paths_input.read(cx).value();
        if value.as_str() != fm.paths.as_deref().unwrap_or("") {
            fm.paths = blank_to_none(&value);
        }
        let value = self.model_input.read(cx).value();
        if value.as_str() != fm.model.as_deref().unwrap_or("") {
            fm.model = blank_to_none(&value);
        }
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
        if self.pasting {
            self.review_paste(window, cx);
            return;
        }
        let draft = match self.build_draft(cx) {
            Ok(d) => d,
            Err((field, msg)) => {
                self.error_field = Some(field);
                self.error = Some(msg);
                if let Some(input) = self.inputs().get(field as usize) {
                    window.focus(&input.focus_handle(cx), cx);
                }
                cx.notify();
                return;
            }
        };
        let imported = self.imported.clone();
        let project_root = self.project_root.clone();
        let workspace = self.workspace.clone();
        super::super::form_state::freeze_inputs(&self.inputs(), true, cx);
        self.error_field = None;
        self.submitting = true;
        cx.notify();

        let me = cx.entity().downgrade();
        window
            .spawn(cx, async move |async_cx| {
                let result = async_cx
                    .background_executor()
                    .spawn(async move {
                        let source = if let Some(imported) = imported {
                            crate::agent::skills::frontmatter::render_edited_skill_md(
                                &imported.source,
                                &imported.frontmatter,
                                &draft.frontmatter,
                                &draft.body,
                            )
                            .map_err(|e| e.to_string())?
                        } else {
                            crate::agent::skills::frontmatter::render_skill_md(
                                &draft.frontmatter,
                                &draft.body,
                            )
                        };
                        persist::create_skill_source(&draft, project_root.as_deref(), &source)
                            .map_err(|e| e.to_string())
                    })
                    .await;
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
                                super::super::form_state::freeze_inputs(&modal.inputs(), false, cx);
                                modal.error = Some(SharedString::from(msg));
                                cx.notify();
                            }
                        }
                    });
                });
            })
            .detach();
    }

    fn values(&self, cx: &App) -> Vec<String> {
        vec![
            self.paste_input.read(cx).value().to_string(),
            self.invocation_input.read(cx).value().to_string(),
            self.scope.slug().to_string(),
            self.user_invocable.to_string(),
            self.disable_model_invocation.to_string(),
            self.name_input.read(cx).value().to_string(),
            self.description_input.read(cx).value().to_string(),
            self.when_to_use_input.read(cx).value().to_string(),
            self.argument_hint_input.read(cx).value().to_string(),
            self.allowed_tools_input.read(cx).value().to_string(),
            self.paths_input.read(cx).value().to_string(),
            self.model_input.read(cx).value().to_string(),
            self.body_editor.read(cx).value().to_string(),
        ]
    }

    fn inputs(&self) -> Vec<Entity<InputState>> {
        vec![
            self.name_input.clone(),
            self.description_input.clone(),
            self.when_to_use_input.clone(),
            self.argument_hint_input.clone(),
            self.allowed_tools_input.clone(),
            self.paths_input.clone(),
            self.model_input.clone(),
            self.body_editor.clone(),
            self.paste_input.clone(),
            self.invocation_input.clone(),
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
        if self.pasting {
            self.paste_input.focus_handle(cx)
        } else {
            self.body_editor.focus_handle(cx)
        }
    }
}

impl ModalView for CreateSkillModal {
    fn guards_close(&self) -> bool {
        true
    }
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave(window, cx);
    }
}

mod quick_entry;
mod render;

pub fn open_create_skill_modal(
    ws: &mut Workspace,
    prefill_scope: Option<SkillScope>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let project_root = ws.active_lane_root();
    let state = cx
        .global::<crate::agent::skills::SkillsState>()
        .snapshot_for(project_root.as_deref());
    crate::workspace::dialog_helpers::open_form_modal(
        strings::skills::new_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| {
            CreateSkillModal::new(workspace, prefill_scope, project_root, state, window, cx)
        },
        window,
        cx,
    );
}

#[cfg(test)]
#[path = "create_modal/tests.rs"]
mod tests;

impl CreateSkillModal {
    pub(super) fn seed_duplicate(
        &mut self,
        source: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match crate::agent::skills::import::parse(&source) {
            Ok(imported) => {
                self.apply_import(imported, window, cx);
                self.name_input.update(cx, |input, cx| {
                    input.set_value(format!("{name}-copy"), window, cx)
                });
                // A new invocation defaults to its new folder identifier.
                self.invocation_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(()) => {
                self.paste_input
                    .update(cx, |input, cx| input.set_value(source, window, cx));
                self.error = Some(strings::skills::import_error().into());
            }
        }
    }
}

#[cfg(feature = "screenshot")]
impl CreateSkillModal {
    pub(in crate::workspace) fn seed_for_shot(
        &mut self,
        review: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_input.update(cx,|input,cx|input.set_value(
            "---\nname: review-changes\ndescription: Review changes before a commit.\n---\n# Review changes\n\nRead the diff and nearby source.\n\n1. Find correctness issues.\n2. Explain the impact with a concrete example.\n3. Cite the affected file and line.\n",window,cx));
        if review {
            self.review_paste(window, cx);
        }
    }
}
