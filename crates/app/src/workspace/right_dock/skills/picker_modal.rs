//! Skill picker modal — substring-search list that chains into the
//! [`SkillInvocationModal`](super::invocation_modal::SkillInvocationModal).
//!
//! Opened when a single click could target more than one skill — most
//! commonly the plugin-group header in the right-bar Skills tab, which
//! contains every skill bundled with that plugin. Pick a skill →
//! Enter / click → the picker closes and the invocation modal opens
//! for the chosen skill.
//!
//! Built on top of `crate::ui::list::{FilteredDelegate, list,
//! searchable_list_state}`, mirroring the existing `TaskPickerModal`
//! structure so all picker-style modals in daruda share one
//! interaction model (search input, ↑/↓, Enter, Escape).

use crate::ui::theme;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::agent::skills::Skill;
use crate::surface::strings;
use crate::ui::WindowExt as _;
use crate::ui::list::{FilteredItem, FilteredListState, ListEvent, list, searchable_list_state};
use crate::workspace::ModalView;
use crate::workspace::Workspace;

/// One row in the picker. Carries the full [`Skill`] so the confirm
/// handler can construct the invocation modal's label without going
/// back to the on-disk model.
#[derive(Clone)]
pub struct SkillPickItem {
    pub skill: Skill,
    /// Precomputed display string — `"name — description"` when a
    /// description exists, just `"name"` otherwise. Cached so the
    /// substring filter doesn't rebuild a `SharedString` per
    /// keystroke.
    pub label_text: SharedString,
}

impl FilteredItem for SkillPickItem {
    fn label(&self) -> SharedString {
        self.label_text.clone()
    }
}

pub struct SkillPickerModal {
    panel_focus_handle: FocusHandle,
    list_state: Entity<FilteredListState<SkillPickItem>>,
    workspace: WeakEntity<Workspace>,
    _list_sub: Subscription,
    /// Shown when the caller passes an empty `items` vec — e.g. a
    /// plugin that surfaces no skills (rare but possible during the
    /// FSEvent race between marketplace clone and `installed_plugins.json`
    /// settling).
    empty_hint: Option<SharedString>,
}

impl SkillPickerModal {
    /// Build picker rows for every skill in `skills`. Stable sort by
    /// lowercase display name so re-opening the picker presents the
    /// same order across renders.
    pub fn build_items(skills: &[Skill]) -> Vec<SkillPickItem> {
        let mut items: Vec<SkillPickItem> = skills
            .iter()
            .map(|s| {
                let display_name = display_name_for_row(s);
                let label_text = match s.frontmatter.description.as_deref() {
                    Some(desc) if !desc.is_empty() => {
                        SharedString::from(format!("{display_name}  —  {desc}"))
                    }
                    _ => SharedString::from(display_name),
                };
                SkillPickItem {
                    skill: s.clone(),
                    label_text,
                }
            })
            .collect();
        items.sort_by(|a, b| {
            a.label_text
                .to_ascii_lowercase()
                .cmp(&b.label_text.to_ascii_lowercase())
        });
        items
    }

    pub fn new(
        workspace: WeakEntity<Workspace>,
        items: Vec<SkillPickItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let empty_hint = if items.is_empty() {
            Some(SharedString::from(strings::skills_empty_plugin_picker()))
        } else {
            None
        };

        let list_state = cx.new(|cx| searchable_list_state(items, window, cx));

        let _list_sub = cx.subscribe_in(
            &list_state,
            window,
            move |this, state, ev: &ListEvent, window, cx| match ev {
                ListEvent::Confirm(ix) => {
                    let skill = state
                        .read(cx)
                        .delegate()
                        .item_at(*ix)
                        .map(|i| i.skill.clone());
                    this.confirm(skill, window, cx);
                }
                ListEvent::Cancel => this.dismiss(window, cx),
                ListEvent::Select(_) => {}
            },
        );

        Self {
            panel_focus_handle: cx.focus_handle(),
            list_state,
            workspace,
            _list_sub,
            empty_hint,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    /// Close this picker and immediately open the invocation modal
    /// for the selected skill. Mirrors `TaskPickerModal::dispatch`:
    /// the dialog stack handles close-then-open in a single update
    /// cycle, so the user perceives the transition as a single step.
    fn confirm(&mut self, skill: Option<Skill>, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let Some(skill) = skill else {
            // Stale index (item dropped from the filtered set between
            // arrow-key highlight and Enter). Nothing to invoke; the
            // dialog is already closed, so just return.
            return;
        };
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| {
                ws.open_skill_invocation_modal(&skill, window, cx);
            });
        }
    }
}

/// Mirror of `right_panel::skills::render`'s plugin-row display
/// logic: drop the `<plugin>:` prefix for plugin-scope skills so the
/// picker reads `format` rather than `swift-lsp:format` (the prefix
/// is implied by the picker being opened from the plugin header).
fn display_name_for_row(s: &Skill) -> String {
    use crate::agent::skills::SkillScope;
    if matches!(s.scope, SkillScope::Plugin) {
        s.name
            .split_once(':')
            .map(|(_plugin, name)| name.to_string())
            .unwrap_or_else(|| s.name.clone())
    } else {
        s.name.clone()
    }
}

impl Focusable for SkillPickerModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // Delegate to the list state so the search input takes focus
        // immediately on open — same pattern as `TaskPickerModal`.
        self.list_state.focus_handle(cx)
    }
}

impl ModalView for SkillPickerModal {}

impl Render for SkillPickerModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some(hint) = self.empty_hint.clone() {
            div()
                .py(px(theme::RIGHT_PANEL_PAD_Y))
                .text_color(theme::current(cx).text_subtle)
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .child(hint)
                .into_any_element()
        } else {
            list(&self.list_state).into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .key_context("SkillPickerModal")
            .track_focus(&self.panel_focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.as_str() == "escape" {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
    }
}
