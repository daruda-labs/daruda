//! Provider selection and actionable status above configuration lists.

use crate::{
    agent::configuration::Provider,
    surface::strings,
    ui::theme,
    workspace::layout::{Dock, RightDockSnapshot},
};
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

pub(super) fn header(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let view = &snap.library;
    let choose_provider = view.config.is_none()
        || snap.focused_agent_domain == crate::workspace::main_area::pane::AccountDomain::Any
        || view.local_management;
    if !choose_provider && !view.remote && !view.loading && view.error.is_none() {
        return gpui::div().into_any_element();
    }
    let t = theme::current(cx);
    let mut body = super::right_panel_body()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted);
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
    if choose_provider {
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
    if view.config.is_some() {
        if view.loading {
            body = body.child(strings::common::loading());
        }
        if view.error.is_some() {
            body = body.child(strings::library::read_error());
        }
    }
    body.into_any_element()
}
