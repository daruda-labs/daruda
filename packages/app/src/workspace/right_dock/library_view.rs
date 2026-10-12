//! Provider selection and honest on-disk status above configuration lists.

use crate::{
    agent::configuration::{Provider, codex::Trust},
    surface::strings,
    ui::theme,
    workspace::layout::{Dock, RightDockSnapshot},
};
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

pub(super) fn header(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let view = &snap.library;
    let t = theme::current(cx);
    let mut body = super::right_panel_body()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted);
    if let Some(provider) = view.provider {
        body = body.child(match provider {
            Provider::Claude => strings::library::claude(),
            Provider::Codex => strings::library::codex(),
        });
    }
    if view.config.is_none() && view.error.is_some() {
        body = body.child(strings::library::root_invalid());
    }
    if view.remote {
        body = body.child(strings::library::remote());
    } else if view.config.is_none()
        && snap.focused_agent_domain
            == crate::workspace::main_area::pane::AccountDomain::Unsupported
    {
        body = body.child(strings::library::unsupported());
    }
    if view.local_management {
        body = body.child(strings::library::local_management());
        let workspace = snap.workspace.clone();
        body = body.child(
            crate::ui::button("follow-pane", strings::library::follow_pane()).on_click(
                move |_, _, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.right_views.library.manual_pane = None;
                            ws.refresh_library(cx);
                        });
                    }
                },
            ),
        );
    }
    if view.config.is_none()
        || snap.focused_agent_domain == crate::workspace::main_area::pane::AccountDomain::Any
        || view.local_management
    {
        body = body.child(strings::library::choose());
        let mut choices = gpui::div().flex().gap(px(theme::PAD_XS));
        for (provider, label) in [
            (Provider::Claude, strings::library::claude()),
            (Provider::Codex, strings::library::codex()),
        ] {
            let workspace = snap.workspace.clone();
            choices = choices.child(crate::ui::button(label.clone(), label).on_click(
                move |_, _, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.right_views.library.manual = Some(provider);
                            ws.right_views.library.manual_pane =
                                Some(ws.active_runtime().focused_pane_id);
                            ws.refresh_library(cx);
                        });
                    }
                },
            ));
        }
        body = body.child(choices);
    }
    if let Some(config) = &view.config {
        body = body.child(strings::library::account(
            view.account_label
                .clone()
                .unwrap_or_else(strings::library::system_account),
        ));
        body = body.child(strings::library::source(config.user_file.display()));
        if let Some(cwd) = &config.cwd {
            body = body.child(cwd.display().to_string());
        }
        body = body.child(strings::library::configured());
        if view.inventory.managed_exclusive {
            body = body.child(strings::library::managed_exclusive());
        }
        if config.provider == Provider::Claude {
            body = body.child(strings::library::approval_hint());
        }
        if config.provider == Provider::Codex && config.cwd.is_some() {
            body = body.child(match view.inventory.trust {
                Trust::Trusted => strings::library::trusted(),
                Trust::Untrusted => strings::library::untrusted(),
                Trust::Unknown => strings::library::trust_unknown(),
            });
        }
        for (_, path) in &view.inventory.shared_paths {
            body = body.child(strings::library::shared(path.display()));
        }
        if view.loading {
            body = body.child(strings::common::loading());
        }
        if view.error.is_some() {
            body = body.child(strings::library::read_error());
        }
    }
    body.into_any_element()
}
