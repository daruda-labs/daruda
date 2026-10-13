//! Edit-skill modal — same chrome as `CreateSkillModal`, prefilled
//! from disk and with the scope chip / name input made read-only
//! (rename happens through a dedicated button).
//!
//! The body editor preserves the body text byte-for-byte. Frontmatter
//! `extra` keys are kept on the modal struct and serialised back on
//! save so unknown YAML fields round-trip losslessly.
//!
//! Tab cycling: GPUI `.tab_group()` on the panel root + each input's
//! `tab_index` assignment.

use std::path::PathBuf;

use crate::ui::theme;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::agent::skills::{
    Skill, SkillDraft, SkillScope, SkillsSnapshot, frontmatter::SkillFrontmatter, persist,
};
use crate::surface::strings;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_primary, checkbox};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use crate::workspace::right_dock::skills::modal_shared::field_label;

fn blank_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub struct EditSkillModal {
    config: Option<std::sync::Arc<crate::agent::configuration::Configuration>>,
    creation_targets: Vec<crate::agent::configuration::library::creation::CreationTarget>,
    panel_focus_handle: FocusHandle,
    form: super::super::form_state::FormState,

    advanced: bool,
    name: String,
    scope: SkillScope,
    dir: PathBuf,

    invocation_input: Entity<InputState>,
    description_input: Entity<InputState>,
    when_to_use_input: Entity<InputState>,
    argument_hint_input: Entity<InputState>,
    allowed_tools_input: Entity<InputState>,
    paths_input: Entity<InputState>,
    model_input: Entity<InputState>,
    body_editor: Entity<InputState>,

    user_invocable: bool,
    disable_model_invocation: bool,

    original_frontmatter: SkillFrontmatter,
    reload_skill: Skill,
    original_source: Option<String>,
    original_path: Option<std::path::PathBuf>,
    companion_path: Option<PathBuf>,
    body_loaded: bool,
    rename_after_save: bool,

    /// Snapshot kept around so future revisions can validate name
    /// collisions inside `submit` without going through the live
    /// workspace entity.
    #[allow(dead_code)]
    state_snapshot: SkillsSnapshot,

    error: Option<SharedString>,
    submitting: bool,

    workspace: WeakEntity<Workspace>,
    _input_subs: Vec<Subscription>,
}

impl EditSkillModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        skill: Skill,
        state_snapshot: SkillsSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Frontmatter (incl. `extra`) was already parsed by the
        // scanner — reuse it instead of synchronously reading the
        // file again on the main thread. The body is the only piece
        // we don't have in memory; load it asynchronously below.
        let frontmatter = skill.frontmatter.clone();
        let body_text = String::new();

        let invocation_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(frontmatter.name.clone().unwrap_or_default())
        });
        let description_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state)
                .placeholder(strings::skills::placeholder_description());
            if let Some(v) = frontmatter.description.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let when_to_use_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state)
                .placeholder(strings::skills::placeholder_when_to_use());
            if let Some(v) = frontmatter.when_to_use.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let argument_hint_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state);
            if let Some(v) = frontmatter.argument_hint.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let allowed_tools_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state);
            if let Some(v) = frontmatter.allowed_tools.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let paths_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state);
            if let Some(v) = frontmatter.paths.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let model_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state);
            if let Some(v) = frontmatter.model.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let body_editor = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state)
                .auto_grow(8, 12)
                .placeholder(strings::skills::placeholder_body_loading());
            if !body_text.is_empty() {
                s = s.default_value(body_text);
            }
            s
        });

        // Body is not in `skill` (only a 200-char preview is). Read
        // the rest of the file off-thread so opening the modal stays
        // frame-rate friendly even on slow disks. The user sees the
        // metadata fields immediately and the body fills in once the
        // background read completes.
        //
        // `InputState::set_value` needs a live `Window`, but the
        // background continuation only has an `AsyncApp`. Capture the
        // current window handle and re-enter via `update_window` to
        // recover it on the next update cycle.
        {
            let path = skill.skill_md_path();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::read_to_string(&path)
                            .map_err(|error| error.to_string())
                            .and_then(|raw| {
                                let (yaml, _) =
                                    crate::agent::skills::frontmatter::split_frontmatter(&raw);
                                let frontmatter =
                                    crate::agent::skills::frontmatter::parse_frontmatter(
                                        yaml.unwrap_or(""),
                                    )
                                    .map_err(|error| error.to_string())?;
                                let companion = path
                                    .parent()
                                    .map(|dir| dir.join("agents/openai.yaml"))
                                    .filter(|p| p.is_file());
                                let physical = daruda_core::path::canonicalize(&path)
                                    .map_err(|error| error.to_string())?;
                                Ok((raw, frontmatter, companion, physical))
                            })
                    })
                    .await;
                // SILENT-OK: closing the modal abandons its pending read.
                let _ = this.update_in(cx, |modal, window, cx| {
                    match result {
                        Ok((source, frontmatter, companion, physical)) => {
                            modal.original_path = Some(physical);
                            modal.companion_path = companion;
                            let (_, body) =
                                crate::agent::skills::frontmatter::split_frontmatter(&source);
                            for (input, value) in [
                                (&modal.invocation_input, frontmatter.name.as_deref()),
                                (&modal.description_input, frontmatter.description.as_deref()),
                                (&modal.when_to_use_input, frontmatter.when_to_use.as_deref()),
                                (
                                    &modal.argument_hint_input,
                                    frontmatter.argument_hint.as_deref(),
                                ),
                                (
                                    &modal.allowed_tools_input,
                                    frontmatter.allowed_tools.as_deref(),
                                ),
                                (&modal.paths_input, frontmatter.paths.as_deref()),
                                (&modal.model_input, frontmatter.model.as_deref()),
                                (&modal.body_editor, Some(body)),
                            ] {
                                input.update(cx, |input, cx| {
                                    input.set_value(value.unwrap_or(""), window, cx)
                                });
                            }
                            modal.user_invocable = frontmatter.user_invocable;
                            modal.disable_model_invocation = frontmatter.disable_model_invocation;
                            modal.original_frontmatter = frontmatter;
                            modal.original_source = Some(source);
                            modal.body_loaded = true;
                            modal.form.baseline = modal.values(cx);
                            super::super::form_state::freeze_inputs(&modal.inputs(), false, cx);
                        }
                        Err(error) => modal.error = Some(error.to_string().into()),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        body_editor.update(cx, |input, cx| input.set_disabled(true, cx));

        let subs = vec![
            forward_input(&invocation_input, window, cx),
            forward_input(&description_input, window, cx),
            forward_input(&when_to_use_input, window, cx),
            forward_input(&argument_hint_input, window, cx),
            forward_input(&allowed_tools_input, window, cx),
            forward_input(&paths_input, window, cx),
            forward_input(&model_input, window, cx),
            forward_body_editor(&body_editor, window, cx),
        ];

        let mut modal = Self {
            config: None,
            creation_targets: Vec::new(),
            advanced: false,
            form: Default::default(),
            panel_focus_handle: cx.focus_handle(),
            name: skill.name.clone(),
            scope: skill.scope,
            dir: skill.dir.clone(),
            invocation_input,
            description_input,
            when_to_use_input,
            argument_hint_input,
            allowed_tools_input,
            paths_input,
            model_input,
            body_editor,
            user_invocable: frontmatter.user_invocable,
            disable_model_invocation: frontmatter.disable_model_invocation,
            original_frontmatter: frontmatter,
            reload_skill: skill.clone(),
            original_source: None,
            original_path: None,
            companion_path: None,
            body_loaded: false,
            rename_after_save: false,
            state_snapshot,
            error: None,
            submitting: false,
            workspace,
            _input_subs: subs,
        };
        modal.form.baseline = modal.values(cx);
        super::super::form_state::freeze_inputs(&modal.inputs(), true, cx);
        modal
    }

    fn build_draft(&self, cx: &gpui::App) -> SkillDraft {
        let mut fm = self.original_frontmatter.clone();
        let value = self.invocation_input.read(cx).value();
        if value.as_str() != fm.name.as_deref().unwrap_or("") {
            fm.name = blank_to_none(&value);
        }
        let value = self.description_input.read(cx).value();
        if value.as_str()
            != self
                .original_frontmatter
                .description
                .as_deref()
                .unwrap_or("")
        {
            fm.description = blank_to_none(&value);
        }
        let value = self.when_to_use_input.read(cx).value();
        if value.as_str()
            != self
                .original_frontmatter
                .when_to_use
                .as_deref()
                .unwrap_or("")
        {
            fm.when_to_use = blank_to_none(&value);
        }
        let value = self.argument_hint_input.read(cx).value();
        if value.as_str()
            != self
                .original_frontmatter
                .argument_hint
                .as_deref()
                .unwrap_or("")
        {
            fm.argument_hint = blank_to_none(&value);
        }
        let value = self.allowed_tools_input.read(cx).value();
        if value.as_str()
            != self
                .original_frontmatter
                .allowed_tools
                .as_deref()
                .unwrap_or("")
        {
            fm.allowed_tools = blank_to_none(&value);
        }
        let value = self.paths_input.read(cx).value();
        if value.as_str() != self.original_frontmatter.paths.as_deref().unwrap_or("") {
            fm.paths = blank_to_none(&value);
        }
        let value = self.model_input.read(cx).value();
        if value.as_str() != self.original_frontmatter.model.as_deref().unwrap_or("") {
            fm.model = blank_to_none(&value);
        }
        fm.user_invocable = self.user_invocable;
        fm.disable_model_invocation = self.disable_model_invocation;

        SkillDraft {
            name: self.name.clone(),
            scope: self.scope,
            frontmatter: fm,
            body: self.body_editor.read(cx).value().to_string(),
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting || !self.body_loaded {
            return;
        }
        let draft = self.build_draft(cx);
        if self
            .config
            .as_ref()
            .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex)
            && (draft
                .frontmatter
                .name
                .as_ref()
                .is_none_or(|s| s.trim().is_empty())
                || draft
                    .frontmatter
                    .description
                    .as_ref()
                    .is_none_or(|s| s.trim().is_empty()))
        {
            self.error = Some(strings::library::codex_skill_required().into());
            let input = if draft
                .frontmatter
                .name
                .as_ref()
                .is_none_or(|s| s.trim().is_empty())
            {
                &self.invocation_input
            } else {
                &self.description_input
            };
            window.focus(&input.focus_handle(cx), cx);
            cx.notify();
            return;
        }
        let expected = self
            .original_source
            .clone()
            .expect("loaded body has a source");
        let source = match crate::agent::skills::frontmatter::render_edited_skill_md(
            &expected,
            &self.original_frontmatter,
            &draft.frontmatter,
            &draft.body,
        ) {
            Ok(source) => source,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let dir = self.dir.clone();
        let expected_path = self.original_path.clone();
        let workspace = self.workspace.clone();
        super::super::form_state::freeze_inputs(&self.inputs(), true, cx);
        self.submitting = true;
        cx.notify();

        let me = cx.entity().downgrade();
        window
            .spawn(cx, async move |async_cx| {
                let result = async_cx
                    .background_executor()
                    .spawn(async move {
                        if daruda_core::path::canonicalize(dir.join("SKILL.md")).ok()
                            != expected_path
                        {
                            return Err(strings::skills::changed_on_disk());
                        }
                        persist::update_skill_source(&dir, &expected, &source).map_err(|error| {
                            match error {
                                persist::PersistError::Io(ref error)
                                    if error.kind() == std::io::ErrorKind::WouldBlock =>
                                {
                                    strings::skills::changed_on_disk()
                                }
                                _ => error.to_string(),
                            }
                        })
                    })
                    .await;
                // SILENT-OK: modal may close during async skill edit
                let _ = async_cx.update(|window, cx| {
                    let Some(me) = me.upgrade() else { return };
                    me.update(cx, |modal, cx| {
                        modal.submitting = false;
                        match result {
                            Ok(_) => {
                                if let Some(ws) = workspace.upgrade() {
                                    ws.update(cx, |ws, cx| ws.refresh_library(cx));
                                }
                                if modal.rename_after_save {
                                    modal.begin_rename(window, cx);
                                } else {
                                    modal.dismiss(window, cx);
                                }
                            }
                            Err(msg) => {
                                modal.rename_after_save = false;
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

    fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.dir.clone();
        let scope = self.scope;
        self.dismiss(window, cx);
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |ws, cx| {
                super::open_rename_skill_modal(ws, scope, dir, window, cx)
            });
        }
    }

    fn request_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use super::super::form_state::{LeaveChoice, ask_to_leave};
        if self.submitting || !self.body_loaded || self.form.asking {
            return;
        }
        let dirty = self.form.is_dirty(&self.values(cx));
        if !dirty {
            self.begin_rename(window, cx);
            return;
        }
        ask_to_leave(
            &mut self.form,
            dirty,
            false,
            window,
            cx,
            |this, choice, window, cx| {
                this.form.asking = false;
                match choice {
                    LeaveChoice::Save => {
                        this.rename_after_save = true;
                        this.submit(window, cx);
                    }
                    LeaveChoice::Discard => this.begin_rename(window, cx),
                    LeaveChoice::Continue => {}
                }
            },
        );
    }

    fn values(&self, cx: &App) -> Vec<String> {
        vec![
            self.user_invocable.to_string(),
            self.disable_model_invocation.to_string(),
            self.invocation_input.read(cx).value().to_string(),
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
            self.invocation_input.clone(),
            self.description_input.clone(),
            self.when_to_use_input.clone(),
            self.argument_hint_input.clone(),
            self.allowed_tools_input.clone(),
            self.paths_input.clone(),
            self.model_input.clone(),
            self.body_editor.clone(),
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
        if !self.body_loaded {
            return;
        }
        if self.error.is_some() {
            self.error = None;
            cx.notify();
        }
    }
}

fn forward_input(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<EditSkillModal>,
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

/// Multi-line body-editor forwarder — Cmd+Enter submits, `Change`
/// clears the banner. Plain Enter inserts a newline; Escape dismisses
/// through Dialog.
fn forward_body_editor(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<EditSkillModal>,
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

impl Focusable for EditSkillModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.body_editor.focus_handle(cx)
    }
}

impl ModalView for EditSkillModal {
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

mod duplicate;
mod reload;
mod render;

pub fn open_edit_skill_modal(
    ws: &mut Workspace,
    dir: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let Some(config) = ws.pin_library() else {
        return;
    };
    let state = ws.right_views.library.view.inventory.skills.clone();
    let creation_targets = ws
        .right_views
        .library
        .view
        .inventory
        .creation_targets
        .clone();

    // Locate the skill in the snapshot. Fail-soft: if the row is
    // stale (deleted concurrently) the modal silently does not open.
    let skill = state
        .project
        .iter()
        .chain(state.personal.iter())
        .find(|s| s.dir == dir)
        .cloned();
    let Some(skill) = skill else { return };

    crate::workspace::dialog_helpers::open_form_modal(
        strings::skills::edit_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| {
            let mut modal = EditSkillModal::new(workspace, skill, state, window, cx);
            modal.config = Some(config);
            modal.creation_targets = creation_targets;
            modal
        },
        window,
        cx,
    );
}

#[cfg(test)]
#[path = "edit_modal/tests.rs"]
mod tests;
