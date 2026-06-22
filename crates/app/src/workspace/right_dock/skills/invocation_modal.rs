//! Skill invocation modal — `/<skill> <user input>` dispatcher.
//!
//! Opened when a user clicks a skill row in the right-bar Skills tab.
//! The modal collects free-form input and on Submit writes
//! `/<display_name> <user input>\n` straight into the focused terminal
//! pane's PTY, mirroring how the user would type the slash command
//! into the Claude Code TUI manually.
//!
//! Plugin-scope skills carry a `<plugin_local>:<skill>` namespace per
//! the Claude Code spec; the namespacing is baked into the
//! `display_name` at call time so the modal stays scope-agnostic.

use crate::ui::theme;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::agent::skills::SkillScope;
use crate::surface::strings;
use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{InputEvent, InputState, button, button_primary, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;

/// Plain-data carrier used to populate the invocation modal. Built by
/// `Workspace::open_skill_invocation_modal` from a `Skill`; the modal
/// itself stays decoupled from the on-disk model so future entry
/// points (command palette, global shortcut) can construct labels
/// without an actual `Skill` value.
#[derive(Clone)]
pub struct SkillInvocationLabel {
    /// Without the leading `/` — e.g. `"format"` or `"swift-lsp:format"`.
    pub display_name: String,
    /// Frontmatter `description` if present. Rendered as a secondary
    /// line under the header.
    pub description: Option<String>,
    /// Frontmatter `argument-hint` if present. Used as the Input
    /// placeholder so the user sees what input the skill expects.
    pub argument_hint: Option<String>,
    /// Carried for Phase B (Skill picker) which surfaces a per-scope
    /// chip — not consumed in Phase A's solo-modal flow, but the
    /// carrier stays scope-agnostic so the picker can construct
    /// labels the same way.
    #[allow(dead_code)]
    pub scope: SkillScope,
    /// Captured at open time so the submit handler sends to the same
    /// pane the user was looking at when they clicked, even if focus
    /// moves while the modal is up.
    pub target_pane_id: crate::workspace::main_area::pane_tree::PaneId,
}

pub struct SkillInvocationModal {
    panel_focus_handle: FocusHandle,
    label: SkillInvocationLabel,
    input: Entity<InputState>,
    workspace: WeakEntity<Workspace>,
    submitting: bool,
    _input_subs: Vec<Subscription>,
}

impl SkillInvocationModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        label: SkillInvocationLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder: SharedString = label
            .argument_hint
            .clone()
            .map(SharedString::from)
            .unwrap_or_else(|| SharedString::from(strings::skills_invoke_placeholder_default()));
        // Multi-line `gpui_component::Input` with Cmd+Enter as
        // `PressEnter { secondary: true }` — plain Enter inserts a
        // newline so users can compose multi-line prompts. Escape is
        // delivered by Dialog's outer Cancel action.
        let input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .multi_line(true)
                .placeholder(placeholder)
        });
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { secondary } = ev
                    && *secondary
                {
                    this.submit(window, cx);
                }
            }),
        ];

        Self {
            panel_focus_handle: cx.focus_handle(),
            label,
            input,
            workspace,
            submitting: false,
            _input_subs: subs,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.submitting = true;
        cx.notify();

        let user_input = self.input.read(cx).value().to_string();
        let cmd = if user_input.is_empty() {
            format!("/{}\n", self.label.display_name)
        } else {
            format!("/{} {}\n", self.label.display_name, user_input)
        };

        let target_pane = self.label.target_pane_id;
        let display_name = self.label.display_name.clone();
        let Some(ws) = self.workspace.upgrade() else {
            // Workspace already dropped — close ourselves.
            self.dismiss(window, cx);
            return;
        };

        let delivered = ws.update(cx, |ws, _cx| ws.send_to_pane(target_pane, cmd.as_bytes()));

        if !delivered {
            ws.update(cx, |ws, cx| {
                let report = ErrorReport::new("Skill invocation failed")
                    .severity(ErrorSeverity::Warning)
                    .message(strings::skills_invoke_no_terminal())
                    .at(file!(), line!())
                    .with_context("skill", &display_name)
                    .dedup("skills.invoke.no_terminal")
                    .build();
                ws.report_error(report, cx);
            });
            self.submitting = false;
            cx.notify();
            return;
        }

        self.dismiss(window, cx);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }
}

impl Focusable for SkillInvocationModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.panel_focus_handle.clone()
    }
}

impl ModalView for SkillInvocationModal {}

impl Render for SkillInvocationModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_focus = self.panel_focus_handle.clone();

        let title = SharedString::from(format!("/{}", self.label.display_name));

        let t = theme::current(cx);
        let title_text = t.text_primary;
        let description_text = t.text_body;

        let mut header = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP / 2.0))
            .child(
                div()
                    .text_size(px(theme::MODAL_TITLE_FONT_SIZE))
                    .text_color(title_text)
                    .child(title),
            );
        if let Some(desc) = self.label.description.as_deref()
            && !desc.is_empty()
        {
            header = header.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_text)
                    .child(SharedString::from(desc.to_string())),
            );
        }

        let body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(header)
            // Wrap the input in a min-height container so it has
            // visible room before the user types. Without an explicit
            // `min_h`, the input collapses to zero in a flex_col with
            // no parent height constraint (Dialog chrome sizes itself
            // to content).
            .child(
                div()
                    .flex()
                    .min_h(px(theme::MODAL_NOTES_TEXTAREA_MIN_H))
                    .child(input(&self.input, cx, 0)),
            );

        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("skill-invoke-cancel", strings::skills_invoke_cancel()).on_click(
                    cx.listener(|this, _: &ClickEvent, w, cx| {
                        this.dismiss(w, cx);
                    }),
                ),
            )
            .child(
                button_primary(
                    "skill-invoke-submit",
                    if self.submitting {
                        strings::skills_invoke_submitting()
                    } else {
                        strings::skills_invoke_submit()
                    },
                )
                .disabled(self.submitting)
                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        div()
            .flex()
            .flex_col()
            .key_context("SkillInvocationModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
            .child(footer)
    }
}
