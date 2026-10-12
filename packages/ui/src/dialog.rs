//! Dialog re-exports for `window.open_dialog(...)` call sites, plus the
//! confirmation helpers every host shares. The single import site for
//! dialog-construction types so callers stay off `gpui_component::*` directly.
//! Helpers that need the app (its strings, its `Workspace`) stay in the app.

use std::rc::Rc;

use gpui::{App, ClickEvent, Context, ParentElement as _, SharedString, WeakEntity, Window};

pub use gpui_component::WindowExt;
pub use gpui_component::button::ButtonVariant;
pub use gpui_component::dialog::{Dialog, DialogButtonProps};

/// The same client-decoration insets that the dialog uses for its bounds.
pub use gpui_component::window_paddings;

/// A title, a body and an OK / Cancel pair, running `on_ok` on OK. The labels
/// come from the caller, so the dialog says what the caller's locale says.
pub fn open_confirm_dialog<F>(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    ok_label: impl Into<SharedString>,
    ok_variant: ButtonVariant,
    on_ok: F,
    window: &mut Window,
    cx: &mut App,
) where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let title: SharedString = title.into();
    let body: SharedString = body.into();
    let ok_label: SharedString = ok_label.into();
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_ok = on_ok.clone();
        dialog
            .title(title.clone())
            .child(body.clone())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(ok_label.clone())
                    .ok_variant(ok_variant),
            )
            .on_ok(move |ev, window, app_cx| {
                (on_ok)(ev, window, app_cx);
                true
            })
    });
}

/// Ask before a destructive action, then run `on_ok` on `entity`.
///
/// The rule it serves: an action that cannot be undone, or that reaches a
/// system outside daruda (a credential store, a paired chat), confirms first.
/// A reversible value change never does.
pub fn confirm_destructive<T: 'static>(
    entity: WeakEntity<T>,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    ok_label: impl Into<SharedString>,
    on_ok: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_ok = Rc::new(on_ok);
    open_confirm_dialog(
        title,
        body,
        ok_label,
        ButtonVariant::Danger,
        move |_, window, app_cx| {
            let on_ok = on_ok.clone();
            if let Some(entity) = entity.upgrade() {
                entity.update(app_cx, |this, cx| on_ok(this, window, cx));
            }
        },
        window,
        cx,
    );
}
