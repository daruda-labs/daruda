//! Macro edit modal for creating or editing a `ButtonWidget`.
//!
//! Create starts from defaults; edit seeds fields from the source widget and
//! keeps `widget_id` for submit. GPUI Tab order follows field order plus the
//! Record button. Submit is synchronous Workspace mutation only.

use crate::ui::theme;
use daruda_store::panels::{ButtonDisplay, ButtonWidget, TabId, WidgetId, new_widget_id};
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Keystroke,
    MouseDownEvent, Render, SharedString, Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::WindowExt as _;
use crate::ui::checkbox;
use crate::ui::{InputEvent, InputState, button, button_primary, input};
use crate::workspace::ModalView;
use crate::workspace::Workspace;

pub struct MacroEditModal {
    label_input: Entity<InputState>,
    send_input: Entity<InputState>,
    icon_input: Entity<InputState>,
    shortcut_input: Entity<InputState>,
    auto_enter: bool,
    display_as_icon: bool,
    error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    tab_id: TabId,
    /// `Some(widget_id)` → Edit mode. `None` → Create mode.
    widget_id: Option<WidgetId>,
    /// Preserved from the source widget so `validate()` does not silently
    /// strip the flag when editing a builtin button (even though the UI
    /// currently prevents opening this modal for builtin buttons).
    is_builtin: bool,
    /// Focus handle for the record button — receives focus while
    /// shortcut recording is active so the next keystroke arrives at
    /// `handle_record_keydown` instead of the shortcut Input.
    record_focus: FocusHandle,
    /// True between "Record" click and the captured keystroke (or
    /// ESC). Drives the button label + visual state and gates the
    /// keydown capture.
    recording_shortcut: bool,
    _input_subscriptions: [Subscription; 4],
}

impl MacroEditModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        tab_id: TabId,
        initial: Option<&ButtonWidget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::macro_placeholder_label())
        });
        let send_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::macro_placeholder_send())
        });
        let icon_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::macro_placeholder_icon())
        });
        let shortcut_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::macro_placeholder_shortcut())
        });

        let (auto_enter, display_as_icon, widget_id, is_builtin) = if let Some(btn) = initial {
            label_input.update(cx, |i, cx_state| {
                i.set_value(btn.label.clone(), window, cx_state)
            });
            send_input.update(cx, |i, cx_state| {
                i.set_value(btn.send.clone(), window, cx_state)
            });
            if let Some(icon) = btn.icon.as_deref() {
                icon_input.update(cx, |i, cx_state| {
                    i.set_value(icon.to_string(), window, cx_state)
                });
            }
            if let Some(sc) = btn.shortcut.as_deref() {
                shortcut_input.update(cx, |i, cx_state| {
                    i.set_value(sc.to_string(), window, cx_state)
                });
            }
            (
                btn.auto_enter,
                matches!(btn.display, ButtonDisplay::Icon),
                Some(btn.id.clone()),
                btn.builtin,
            )
        } else {
            (true, false, None, false)
        };

        let make_sub = |state: &Entity<InputState>, this_cx: &mut Context<Self>| {
            this_cx.subscribe_in(
                state,
                window,
                |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    InputEvent::Change => {
                        if this.error.is_some() {
                            this.error = None;
                            cx.notify();
                        }
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            )
        };
        let _input_subscriptions = [
            make_sub(&label_input, cx),
            make_sub(&send_input, cx),
            make_sub(&icon_input, cx),
            make_sub(&shortcut_input, cx),
        ];

        let record_focus = cx.focus_handle();

        Self {
            label_input,
            send_input,
            icon_input,
            shortcut_input,
            auto_enter,
            display_as_icon,
            error: None,
            workspace,
            tab_id,
            widget_id,
            is_builtin,
            record_focus,
            recording_shortcut: false,
            _input_subscriptions,
        }
    }

    fn toggle_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recording_shortcut = !self.recording_shortcut;
        if self.recording_shortcut {
            // Move focus to the record button so the next keystroke
            // lands in `handle_record_keydown` instead of the shortcut
            // Input.
            self.record_focus.clone().focus(window, cx);
        }
        cx.notify();
    }

    fn handle_record_keydown(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.recording_shortcut {
            return;
        }
        // Modifier-only keystrokes (just Cmd held) report key="" —
        // *don't* stop_propagation on these. macOS's flagsChanged
        // events flow through the same KeyDown channel; if we eat
        // them, the input tracker can lose the chord state and the
        // final "real" keystroke (e.g. cmd-shift-1) never arrives.
        if ev.keystroke.key.is_empty() {
            return;
        }
        // Real key arrived — now block it from reaching keybinding
        // dispatch / other on_key_down handlers, then commit it as
        // the recorded shortcut.
        cx.stop_propagation();
        if ev.keystroke.key.as_str() == "escape" && !has_any_modifier(&ev.keystroke) {
            self.recording_shortcut = false;
            cx.notify();
            return;
        }
        let formatted = format_keystroke_for_keymap(&ev.keystroke);
        self.shortcut_input.update(cx, |inp, cx_state| {
            inp.set_value(formatted, window, cx_state)
        });
        self.recording_shortcut = false;
        if self.error.is_some() {
            self.error = None;
        }
        cx.notify();
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    pub(crate) fn validate(&self, cx: &gpui::App) -> Result<ButtonWidget, String> {
        let label = self.label_input.read(cx).value().to_string();
        let label = label.trim().to_string();
        if label.is_empty() {
            return Err(s::macro_err_label_required());
        }
        let send = self.send_input.read(cx).value().to_string();
        if send.trim().is_empty() {
            return Err(s::macro_err_send_required());
        }

        let display = if self.display_as_icon {
            ButtonDisplay::Icon
        } else {
            ButtonDisplay::Text
        };
        let icon_raw = self.icon_input.read(cx).value().to_string();
        let icon_raw = icon_raw.trim().to_string();
        let icon = if icon_raw.is_empty() {
            None
        } else {
            Some(icon_raw)
        };
        if matches!(display, ButtonDisplay::Icon) && icon.is_none() {
            return Err(s::macro_err_icon_required());
        }

        let shortcut_raw = self.shortcut_input.read(cx).value().to_string();
        let shortcut_raw = shortcut_raw.trim().to_string();
        let shortcut = if shortcut_raw.is_empty() {
            None
        } else {
            Some(shortcut_raw)
        };

        // ID is overwritten on the workspace side (add_widget assigns
        // a fresh ULID; update_widget preserves the existing id).
        // Using new_widget_id() in Create mode is just defensive — the
        // workspace doesn't trust this field anyway.
        let id = self.widget_id.clone().unwrap_or_else(new_widget_id);

        Ok(ButtonWidget {
            id,
            label,
            send,
            auto_enter: self.auto_enter,
            display,
            icon,
            shortcut,
            style: None,
            builtin: self.is_builtin,
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let btn = match self.validate(cx) {
            Ok(b) => b,
            Err(msg) => {
                self.error = Some(msg.into());
                cx.notify();
                return;
            }
        };
        let Some(workspace) = self.workspace.upgrade() else {
            self.dismiss(window, cx);
            return;
        };
        let tab_id = self.tab_id.clone();
        let widget_id = self.widget_id.clone();
        // Dismiss before workspace.update so the modal state is clean
        // before the parent re-renders.
        window.close_dialog(cx);
        workspace.update(cx, |ws, cx| match widget_id {
            Some(wid) => ws.update_widget(tab_id, wid, btn, cx),
            None => ws.add_widget(tab_id, btn, cx),
        });
    }

    fn toggle_auto_enter(&mut self, cx: &mut Context<Self>) {
        self.auto_enter = !self.auto_enter;
        cx.notify();
    }

    /// Build the inline "Record" button. Tracks `record_focus` so it
    /// can receive keystrokes while recording is active. Click toggles
    /// recording. The visible label / color reflects the recording
    /// state.
    fn record_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recording = self.recording_shortcut;
        let label = if recording {
            s::macro_record_recording()
        } else {
            s::macro_record_idle()
        };
        let t = theme::current(cx);
        let widget_bg = t.button_widget_bg;
        let widget_bg_hover = t.button_widget_bg_hover;
        let widget_text = t.text_body;
        let bg = if recording {
            widget_bg_hover
        } else {
            widget_bg
        };

        div()
            .id("macro-edit-record")
            .track_focus(&self.record_focus.clone())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_record_keydown(ev, window, cx);
            }))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(theme::BUTTON_WIDGET_HEIGHT))
            .px(px(theme::BUTTON_WIDGET_PAD_X))
            .rounded(px(theme::BUTTON_WIDGET_RADIUS))
            .bg(bg)
            .text_color(widget_text)
            .text_size(px(theme::BUTTON_WIDGET_FONT_SIZE))
            .cursor_pointer()
            .hover(move |d| d.bg(widget_bg_hover))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.toggle_recording(window, cx);
                }),
            )
            .child(label)
    }

    fn toggle_display_as_icon(&mut self, cx: &mut Context<Self>) {
        self.display_as_icon = !self.display_as_icon;
        // Editing display mode while an error referenced the icon
        // field is now stale — clear it so the user gets fresh
        // feedback on next submit.
        if self.error.is_some() {
            self.error = None;
        }
        cx.notify();
    }
}

impl Focusable for MacroEditModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // Land focus on the label so the user can type immediately.
        self.label_input.focus_handle(cx)
    }
}

impl ModalView for MacroEditModal {}

impl Render for MacroEditModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let submit_label = if self.widget_id.is_some() {
            s::common_button_save()
        } else {
            s::common_button_create()
        };

        let error_text_color = theme::ERROR;

        // Dialog provides outer chrome (panel bg / border / radius /
        // padding / title / backdrop). The modal body is just the
        // form fields + footer.
        let mut body =
            div()
                .key_context("MacroEditModal")
                .tab_group()
                // Safety net: when shortcut recording is active, route the
                // captured keystroke through `handle_record_keydown` even
                // if focus has wandered off the record button itself.
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if this.recording_shortcut {
                        this.handle_record_keydown(ev, window, cx);
                    }
                }))
                .flex()
                .flex_col()
                .gap(px(theme::MODAL_PANEL_GAP))
                .child(field_label(s::macro_field_label(), cx))
                .child(input(&self.label_input, cx, 0))
                .child(field_label(s::macro_field_send(), cx))
                .child(input(&self.send_input, cx, 1))
                .child(
                    checkbox("macro-edit-auto-enter", s::macro_auto_enter(), 2)
                        .checked(self.auto_enter)
                        .on_click(
                            cx.listener(|this, _checked: &bool, _w, cx| this.toggle_auto_enter(cx)),
                        ),
                )
                .child(
                    checkbox("macro-edit-display-icon", s::macro_display_as_icon(), 3)
                        .checked(self.display_as_icon)
                        .on_click(cx.listener(|this, _checked: &bool, _w, cx| {
                            this.toggle_display_as_icon(cx)
                        })),
                )
                .child(field_label(s::macro_field_icon(), cx))
                .child(input(&self.icon_input, cx, 4))
                .child(field_label(s::macro_field_shortcut(), cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(theme::MODAL_FOOTER_GAP))
                        .child(div().flex_1().child(input(&self.shortcut_input, cx, 5)))
                        .child(self.record_button(cx)),
                );

        if let Some(err) = self.error.as_ref() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(error_text_color)
                    .child(err.clone()),
            );
        }

        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("macro-edit-cancel", s::common_button_cancel()).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    },
                )),
            )
            .child(
                button_primary("macro-edit-submit", submit_label).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    },
                )),
            );

        body.child(footer)
    }
}

/// Format a `Keystroke` as the keymap-syntax string daruda's
/// keybinding registrar accepts (e.g. `cmd-shift-1`). Modifier order
/// is fixed (cmd → ctrl → alt → shift → key) so identical keystrokes
/// always produce identical output (PartialEq tab-id behavior with
/// stale bindings).
fn format_keystroke_for_keymap(ks: &Keystroke) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ks.modifiers.platform {
        parts.push("cmd");
    }
    if ks.modifiers.control {
        parts.push("ctrl");
    }
    if ks.modifiers.alt {
        parts.push("alt");
    }
    if ks.modifiers.shift {
        parts.push("shift");
    }
    let key = ks.key.as_str();
    parts.push(key);
    parts.join("-")
}

fn has_any_modifier(ks: &Keystroke) -> bool {
    ks.modifiers.platform || ks.modifiers.control || ks.modifiers.alt || ks.modifiers.shift
}

fn field_label(text: impl Into<SharedString>, cx: &gpui::App) -> impl IntoElement {
    let text = text.into();
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_muted)
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, WindowHandle};

    fn build_modal(
        cx: &mut TestAppContext,
        initial: Option<ButtonWidget>,
    ) -> (WindowHandle<MacroEditModal>, Entity<MacroEditModal>) {
        crate::test_support::init_gpui_component(cx);
        let wh = cx.add_window(|window, cx| {
            MacroEditModal::new(
                WeakEntity::new_invalid(),
                "tab-id".to_string(),
                initial.as_ref(),
                window,
                cx,
            )
        });
        let modal = wh.root(cx).unwrap();
        (wh, modal)
    }

    /// Test-only — write a value into one of the modal's inputs using
    /// the real `InputState::set_value` pipeline. Tests don't carry a
    /// live `Window`, so we re-enter through the modal's window handle.
    fn set_field(
        wh: &WindowHandle<MacroEditModal>,
        modal: &Entity<MacroEditModal>,
        cx: &mut TestAppContext,
        field: fn(&MacroEditModal) -> Entity<InputState>,
        s: &str,
    ) {
        let state = modal.read_with(cx, |m, _| field(m));
        // SILENT-OK: focus restore on possibly-dismissed dialog
        let _ = wh.update(cx, |_root, window, cx| {
            state.update(cx, |i, cx_state| {
                i.set_value(s.to_string(), window, cx_state);
            });
        });
    }

    fn read_btn(modal: &Entity<MacroEditModal>, cx: &mut TestAppContext) -> ButtonWidget {
        cx.update(|cx| {
            modal
                .read(cx)
                .validate(cx)
                .expect("validate should succeed")
        })
    }

    fn read_err(modal: &Entity<MacroEditModal>, cx: &mut TestAppContext) -> String {
        cx.update(|cx| {
            modal
                .read(cx)
                .validate(cx)
                .expect_err("validate should fail")
        })
    }

    #[gpui::test]
    fn create_mode_defaults(cx: &mut TestAppContext) {
        let (_wh, modal) = build_modal(cx, None);
        modal.update(cx, |m, _| {
            assert!(m.widget_id.is_none());
            assert!(m.auto_enter);
            assert!(!m.display_as_icon);
        });
    }

    #[gpui::test]
    fn edit_mode_seeds_fields(cx: &mut TestAppContext) {
        let initial = ButtonWidget {
            id: "w1".to_string(),
            label: "Build".to_string(),
            send: "cargo build".to_string(),
            auto_enter: false,
            display: ButtonDisplay::Icon,
            icon: Some("🔨".to_string()),
            shortcut: Some("cmd-shift-b".to_string()),
            style: None,
            builtin: false,
        };
        let (_wh, modal) = build_modal(cx, Some(initial));
        modal.update(cx, |m, cx| {
            assert_eq!(m.widget_id.as_deref(), Some("w1"));
            assert!(!m.auto_enter);
            assert!(m.display_as_icon);
            assert_eq!(m.label_input.read(cx).value().as_ref(), "Build");
            assert_eq!(m.send_input.read(cx).value().as_ref(), "cargo build");
            assert_eq!(m.icon_input.read(cx).value().as_ref(), "🔨");
            assert_eq!(m.shortcut_input.read(cx).value().as_ref(), "cmd-shift-b");
        });
    }

    #[gpui::test]
    fn validate_rejects_empty_label(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None);
        // Send is required too — set it so we isolate the label check.
        set_field(&wh, &modal, cx, |m| m.send_input.clone(), "ls");
        let err = read_err(&modal, cx);
        assert!(err.contains("Label"));
    }

    #[gpui::test]
    fn validate_rejects_empty_send(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None);
        set_field(&wh, &modal, cx, |m| m.label_input.clone(), "X");
        let err = read_err(&modal, cx);
        assert!(err.contains("Send"));
    }

    #[gpui::test]
    fn validate_rejects_icon_mode_without_icon(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None);
        set_field(&wh, &modal, cx, |m| m.label_input.clone(), "X");
        set_field(&wh, &modal, cx, |m| m.send_input.clone(), "y");
        modal.update(cx, |m, _| m.display_as_icon = true);
        let err = read_err(&modal, cx);
        assert!(err.contains("Icon"));
    }

    #[gpui::test]
    fn validate_succeeds_minimal_text_mode(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None);
        set_field(&wh, &modal, cx, |m| m.label_input.clone(), "Claude");
        set_field(&wh, &modal, cx, |m| m.send_input.clone(), "claude");
        let btn = read_btn(&modal, cx);
        assert_eq!(btn.label, "Claude");
        assert_eq!(btn.send, "claude");
        assert!(btn.auto_enter);
        assert_eq!(btn.display, ButtonDisplay::Text);
        assert!(btn.icon.is_none());
        assert!(btn.shortcut.is_none());
    }

    #[gpui::test]
    fn validate_normalizes_shortcut_blank_to_none(cx: &mut TestAppContext) {
        let (wh, modal) = build_modal(cx, None);
        set_field(&wh, &modal, cx, |m| m.label_input.clone(), "X");
        set_field(&wh, &modal, cx, |m| m.send_input.clone(), "y");
        set_field(&wh, &modal, cx, |m| m.shortcut_input.clone(), "   ");
        let btn = read_btn(&modal, cx);
        assert!(btn.shortcut.is_none());
    }

    #[test]
    fn format_keystroke_simple_letter() {
        let ks = Keystroke::parse("cmd-shift-1").unwrap();
        assert_eq!(format_keystroke_for_keymap(&ks), "cmd-shift-1");
    }

    #[test]
    fn format_keystroke_canonical_modifier_order() {
        // Even if the parser accepts any order, our formatter outputs
        // a canonical sequence: cmd → ctrl → alt → shift → key.
        let ks = Keystroke::parse("shift-ctrl-cmd-alt-a").unwrap();
        assert_eq!(format_keystroke_for_keymap(&ks), "cmd-ctrl-alt-shift-a");
    }

    #[test]
    fn format_keystroke_no_modifier() {
        let ks = Keystroke::parse("f5").unwrap();
        assert_eq!(format_keystroke_for_keymap(&ks), "f5");
    }

    #[test]
    fn format_keystroke_round_trips_through_parse() {
        // Anything we emit must be parseable again — otherwise stored
        // shortcuts wouldn't match the recording.
        let inputs = ["cmd-1", "cmd-shift-d", "ctrl-tab", "cmd-alt-left"];
        for s in inputs {
            let ks = Keystroke::parse(s).unwrap();
            let formatted = format_keystroke_for_keymap(&ks);
            // Re-parse should succeed.
            assert!(
                Keystroke::parse(&formatted).is_ok(),
                "{} -> {}",
                s,
                formatted
            );
        }
    }

    #[gpui::test]
    fn validate_preserves_id_in_edit_mode(cx: &mut TestAppContext) {
        let initial = ButtonWidget {
            id: "stable-id".to_string(),
            label: "X".to_string(),
            send: "y".to_string(),
            auto_enter: true,
            display: ButtonDisplay::Text,
            icon: None,
            shortcut: None,
            style: None,
            builtin: false,
        };
        let (_wh, modal) = build_modal(cx, Some(initial));
        let btn = read_btn(&modal, cx);
        assert_eq!(btn.id, "stable-id");
    }
}
