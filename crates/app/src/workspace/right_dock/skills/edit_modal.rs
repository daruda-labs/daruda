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
use crate::ui::{InputEvent, InputState, button, button_primary, checkbox, input};
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
    panel_focus_handle: FocusHandle,

    name: String,
    scope: SkillScope,
    dir: PathBuf,

    description_input: Entity<InputState>,
    when_to_use_input: Entity<InputState>,
    argument_hint_input: Entity<InputState>,
    allowed_tools_input: Entity<InputState>,
    paths_input: Entity<InputState>,
    model_input: Entity<InputState>,
    body_editor: Entity<InputState>,

    user_invocable: bool,
    disable_model_invocation: bool,

    /// Carry-over of every YAML key daruda doesn't interpret. Saved
    /// back on submit so external tooling that adds a field (e.g.
    /// `hooks:` blocks) survives the round-trip.
    extra: std::collections::BTreeMap<String, serde_yaml::Value>,
    /// Unparsed `name` / `arguments` fields preserved through the
    /// frontmatter serializer.
    frontmatter_name: Option<String>,
    frontmatter_arguments: Vec<String>,

    project_root: Option<PathBuf>,
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
        project_root: Option<PathBuf>,
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

        let description_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state).placeholder("description");
            if let Some(v) = frontmatter.description.as_deref() {
                s = s.default_value(v.to_string());
            }
            s
        });
        let when_to_use_input = cx.new(|cx_state| {
            let mut s = InputState::new(window, cx_state).placeholder("when to use");
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
                .multi_line(true)
                .placeholder("Loading body…");
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
            let body_editor = body_editor.clone();
            let path = skill.skill_md_path();
            let wh = window.window_handle();
            cx.spawn(async move |_, async_cx| {
                let body = async_cx
                    .background_executor()
                    .spawn(async move {
                        let raw = std::fs::read_to_string(&path).unwrap_or_default();
                        let (_, body) = crate::agent::skills::frontmatter::split_frontmatter(&raw);
                        body.trim_start_matches('\n').to_string()
                    })
                    .await;
                // SILENT-OK: modal may close during async skill edit
                let _ = async_cx.update(|app_cx| {
                    // SILENT-OK: modal may close during async skill edit
                    let _ = app_cx.update_window(wh, |_, window, cx| {
                        body_editor.update(cx, |inp, cx_state| {
                            // Only fill the body if the user hasn't begun
                            // typing — otherwise we'd clobber their edits
                            // when a slow disk read finally lands.
                            if inp.value().is_empty() {
                                inp.set_value(body, window, cx_state);
                            }
                        });
                    });
                });
            })
            .detach();
        }

        let subs = vec![
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
            name: skill.name.clone(),
            scope: skill.scope,
            dir: skill.dir.clone(),
            description_input,
            when_to_use_input,
            argument_hint_input,
            allowed_tools_input,
            paths_input,
            model_input,
            body_editor,
            user_invocable: frontmatter.user_invocable,
            disable_model_invocation: frontmatter.disable_model_invocation,
            extra: frontmatter.extra,
            frontmatter_name: frontmatter.name,
            frontmatter_arguments: frontmatter.arguments,
            project_root,
            state_snapshot,
            error: None,
            submitting: false,
            workspace,
            _input_subs: subs,
        }
    }

    fn build_draft(&self, cx: &gpui::App) -> SkillDraft {
        let mut fm = SkillFrontmatter::empty();
        fm.name = self.frontmatter_name.clone();
        fm.description = blank_to_none(&self.description_input.read(cx).value());
        fm.when_to_use = blank_to_none(&self.when_to_use_input.read(cx).value());
        fm.argument_hint = blank_to_none(&self.argument_hint_input.read(cx).value());
        fm.allowed_tools = blank_to_none(&self.allowed_tools_input.read(cx).value());
        fm.paths = blank_to_none(&self.paths_input.read(cx).value());
        fm.model = blank_to_none(&self.model_input.read(cx).value());
        fm.user_invocable = self.user_invocable;
        fm.disable_model_invocation = self.disable_model_invocation;
        fm.arguments = self.frontmatter_arguments.clone();
        fm.extra = self.extra.clone();

        SkillDraft {
            name: self.name.clone(),
            scope: self.scope,
            frontmatter: fm,
            body: self.body_editor.read(cx).value().to_string(),
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        let draft = self.build_draft(cx);
        let project_root = self.project_root.clone();
        let workspace = self.workspace.clone();
        self.submitting = true;
        cx.notify();

        let me = cx.entity().downgrade();
        window
            .spawn(cx, async move |async_cx| {
                let result = persist::write_skill(&draft, project_root.as_deref(), true)
                    .map_err(|e| e.to_string());
                // SILENT-OK: modal may close during async skill edit
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
        self.description_input.focus_handle(cx)
    }
}

impl ModalView for EditSkillModal {}

impl Render for EditSkillModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_focus = self.panel_focus_handle.clone();
        let name_label = self.name.clone();
        let scope_label = match self.scope {
            SkillScope::Project => strings::skills_project(),
            SkillScope::Personal => strings::skills_personal(),
            // Plugin scope is read-only — `open_edit_skill_modal` (the
            // free fn this modal opens through) refuses to open for
            // plugin skills, so this arm is unreachable in practice.
            // The value is purely defensive.
            SkillScope::Plugin => strings::skills_plugin(),
        };
        let user_invocable = self.user_invocable;
        let disable_model = self.disable_model_invocation;

        let left = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::skills_field_name()))
            .child(readonly_value(name_label, cx))
            .child(field_label(strings::skills_field_scope()))
            .child(readonly_value(scope_label.to_string(), cx))
            .child(field_label(strings::skills_field_description()))
            .child(input(&self.description_input, cx, 0))
            .child(field_label(strings::skills_field_when_to_use()))
            .child(input(&self.when_to_use_input, cx, 1))
            .child(field_label(strings::skills_field_allowed_tools()))
            .child(input(&self.allowed_tools_input, cx, 2))
            .child(field_label(strings::skills_field_arg_hint()))
            .child(input(&self.argument_hint_input, cx, 3))
            .child(field_label(strings::skills_field_paths()))
            .child(input(&self.paths_input, cx, 4))
            .child(field_label(strings::skills_field_model()))
            .child(input(&self.model_input, cx, 5))
            .child(
                checkbox(
                    "edit-skill-user-invocable",
                    strings::skills_toggle_user_invocable(),
                    7,
                )
                .checked(user_invocable)
                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    this.user_invocable = *checked;
                    cx.notify();
                })),
            )
            .child(
                checkbox(
                    "edit-skill-disable-model",
                    strings::skills_toggle_disable_model(),
                    8,
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
            .child(field_label(strings::skills_field_body()))
            // body_editor sits between the left-column inputs and the
            // two toggles so Tab moves directly from the last metadata
            // field into the markdown body — toggles come last as
            // tail-of-form tweaks.
            .child(input(&self.body_editor, cx, 6));

        let body = div()
            .flex()
            .flex_row()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .child(left)
            .child(right);

        let dir_for_finder = self.dir.clone();
        let dir_for_rename = self.dir.clone();
        let scope_for_rename = self.scope;
        let workspace_for_finder = self.workspace.clone();
        let workspace_for_rename = self.workspace.clone();
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("edit-skill-rename", strings::skills_button_rename()).on_click(cx.listener(
                    move |this, _: &ClickEvent, w, cx| {
                        // Close the edit dialog first so the rename
                        // prompt stays the topmost dialog.
                        this.dismiss(w, cx);
                        if let Some(ws) = workspace_for_rename.upgrade() {
                            let dir = dir_for_rename.clone();
                            ws.update(cx, |ws, cx| {
                                super::open_rename_skill_modal(ws, scope_for_rename, dir, w, cx);
                            });
                        }
                    },
                )),
            )
            .child(
                button("edit-skill-finder", strings::skills_button_open_finder()).on_click(
                    cx.listener(move |_, _: &ClickEvent, _w, cx| {
                        if let Some(ws) = workspace_for_finder.upgrade() {
                            let dir = dir_for_finder.clone();
                            ws.update(cx, |ws, cx| ws.open_skill_dir_in_finder(&dir, cx));
                        }
                    }),
                ),
            )
            .child(
                button("edit-skill-cancel", "Cancel")
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.dismiss(w, cx))),
            )
            .child(
                button_primary(
                    "edit-skill-save",
                    if self.submitting { "Saving…" } else { "Save" },
                )
                .disabled(self.submitting)
                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let banner = self
            .error
            .as_ref()
            .map(|msg| crate::ui::alert::error("edit-skill-error", msg.clone()));

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("EditSkillModal")
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

fn readonly_value(value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .px(px(theme::MODAL_INPUT_PAD))
        .py(px(theme::MODAL_INPUT_PAD))
        .rounded(px(theme::MODAL_BUTTON_RADIUS))
        .bg(t.modal_input_bg)
        .border_1()
        .border_color(t.modal_input_border)
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(theme::TEXT_PRIMARY)
        .child(value.into())
}

pub fn open_edit_skill_modal(
    ws: &mut Workspace,
    dir: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let project_root = ws.active_lane_root();
    let state = cx
        .global::<crate::agent::skills::SkillsState>()
        .snapshot_for(project_root.as_deref());

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
        strings::skills_edit_title(),
        Some(px(crate::ui::theme::FORM_MODAL_WIDE)),
        move |window, cx| EditSkillModal::new(workspace, skill, project_root, state, window, cx),
        window,
        cx,
    );
}
