//! The thin banner above the conversation reporting the session's connection
//! state, and the two gates deciding whether a failure gets a way back.

use daruda_acp::ConnectPhase;
use gpui::{AnyWindowHandle, Context, Hsla, IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::view::{
    AgentChatView, AgentSessionStatus, RuntimePrepPhase,
};
use crate::workspace::main_area::pane_tree::PaneId;

/// Localized banner copy for a runtime-provisioning milestone.
fn runtime_prep_text(phase: RuntimePrepPhase) -> SharedString {
    match phase {
        RuntimePrepPhase::Downloading => s::agent_chat_runtime_downloading(),
        RuntimePrepPhase::Verifying => s::agent_chat_runtime_verifying(),
        RuntimePrepPhase::Extracting => s::agent_chat_runtime_extracting(),
    }
    .into()
}

/// Localized banner copy for a handshake milestone — refines the flat
/// "Connecting…" text once the connection task reports which ACP request is
/// in flight.
fn connect_phase_text(phase: ConnectPhase) -> SharedString {
    match phase {
        ConnectPhase::Handshaking => s::agent_chat_connecting_handshake(),
        ConnectPhase::CreatingSession => s::agent_chat_connecting_creating_session(),
        ConnectPhase::LoadingSession => s::agent_chat_connecting_loading_session(),
        ConnectPhase::ApplyingMode => s::agent_chat_connecting_applying_mode(),
    }
    .into()
}

/// Whether the error banner may offer "Retry", as a pure decision so it can be
/// asserted without building an element.
///
/// Two independent gates, both necessary. `has_cwd` is mechanical: a cwd-less
/// pane cannot reconnect at all, and `retry_agent_chat_connect` no-ops on it.
/// The remedy is semantic: reconnecting after an expired login or an
/// organization-blocked account re-runs the identical handshake with the
/// identical credentials, so the button can only fail the same way — an
/// invitation to click forever while nothing changes.
fn banner_offers_retry(status: &AgentSessionStatus, has_cwd: bool) -> bool {
    match status {
        AgentSessionStatus::Error { remedy, .. } => {
            has_cwd && matches!(remedy, daruda_acp::Remedy::Retry)
        }
        _ => false,
    }
}

/// Whether the error banner may offer a re-login, as a pure decision.
///
/// Deliberately NOT gated on `has_cwd`, unlike [`banner_offers_retry`]: that
/// gate exists because a cwd-less pane cannot *reconnect*, while signing in
/// again writes credentials the pane will read on its next connect regardless.
/// Inheriting it would hide the button on precisely the failures it fixes.
fn banner_offers_reauth(status: &AgentSessionStatus) -> bool {
    match status {
        AgentSessionStatus::Error { remedy, .. } => {
            matches!(remedy, daruda_acp::Remedy::Reauthenticate)
        }
        _ => false,
    }
}

/// The thin top banner — shown while connecting or on error; hidden once the
/// session is live (the conversation itself signals readiness). The `Error`
/// arm carries an inline "Retry" button (`window_handle` + `pane_id` locate
/// the owning `Workspace` op) — otherwise a failed connect has no way back
/// short of closing the pane; see `Workspace::retry_agent_chat_connect`.
pub(in crate::workspace::main_area::agent_chat_pane::render) fn status_banner(
    status: &AgentSessionStatus,
    pane_id: PaneId,
    window_handle: AnyWindowHandle,
    has_cwd: bool,
    t: &theme::DarudaTheme,
    cx: &mut Context<AgentChatView>,
) -> Option<impl IntoElement + use<>> {
    let reauthable = banner_offers_reauth(status);
    let (text, bg, fg, retryable): (SharedString, Hsla, Hsla, bool) = match status {
        AgentSessionStatus::Idle => (
            s::agent_chat_idle().into(),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::PreparingRuntime(phase) => (
            runtime_prep_text(*phase),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::Connecting => (
            s::agent_chat_connecting().into(),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::Handshaking(phase) => (
            connect_phase_text(*phase),
            t.banner_info_bg,
            t.banner_info_text,
            false,
        ),
        AgentSessionStatus::Connected => return None,
        AgentSessionStatus::Error { message, .. } => (
            format!("{} {}", s::agent_chat_error_prefix(), message).into(),
            t.banner_error_bg,
            t.banner_error_text,
            banner_offers_retry(status, has_cwd),
        ),
    };
    let retry_button = retryable.then(|| {
        super::super::blocks::banner_action_button(
            ("agent-chat-retry", pane_id as usize),
            s::agent_chat_retry(),
            t,
            cx,
        )
        .on_click(cx.listener(move |_this, _ev, _window, cx| {
            // `cx.listener` has this AgentChatView leased for the duration of
            // this callback; `retry_agent_chat_connect` reads/updates this
            // same entity (via `Workspace::agent_chat_view`), which would
            // double-lease-panic if called inline (CLAUDE.md Pitfall #5).
            // `cx.defer` runs after the lease is released.
            cx.defer(move |cx| {
                if let Some(workspace) =
                    crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
                {
                    // SILENT-OK: the workspace window may already be closed by the time this deferred callback runs — nothing left to retry
                    let _ = workspace.update(cx, |ws, cx| {
                        ws.retry_agent_chat_connect(pane_id, cx);
                    });
                }
            });
        }))
    });
    let reauth_button = reauthable.then(|| {
        super::super::blocks::banner_action_button(
            ("agent-chat-reauth", pane_id as usize),
            s::agent_chat_sign_in_again(),
            t,
            cx,
        )
        .on_click(cx.listener(move |_this, _ev, _window, cx| {
            // Same lease hazard as the retry button above: the login op
            // reaches this AgentChatView through `Workspace`, which would
            // double-lease-panic inline (CLAUDE.md Pitfall #5).
            cx.defer(move |cx| {
                if let Some(workspace) =
                    crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
                {
                    // SILENT-OK: the workspace window may already be closed by the time this deferred callback runs — nothing left to sign in for
                    let _ = workspace.update(cx, |ws, cx| {
                        ws.reauthenticate_pane_account(pane_id, cx);
                    });
                }
            });
        }))
    });
    Some(
        div()
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .px(px(theme::AGENT_CHAT_PAD_X))
            .py(px(theme::AGENT_CHAT_PAD_Y))
            .bg(bg)
            .text_color(fg)
            .text_size(px(theme::agent_chat_font_size(cx)))
            .child(div().flex_1().min_w_0().child(text))
            .when_some(retry_button, |el, btn| el.child(btn))
            .when_some(reauth_button, |el, btn| el.child(btn)),
    )
}

#[cfg(test)]
mod tests {
    use super::{AgentSessionStatus, banner_offers_reauth, banner_offers_retry};
    use daruda_acp::Remedy;

    fn errored(remedy: Remedy) -> AgentSessionStatus {
        AgentSessionStatus::Error {
            message: "boom".to_owned(),
            remedy,
        }
    }

    /// The whole point of routing the banner through a remedy: an expired
    /// login used to get a Retry button that reconnected with the same
    /// credentials and failed identically.
    #[test]
    fn banner_withholds_retry_from_failures_it_cannot_fix() {
        for remedy in [
            Remedy::Reauthenticate,
            Remedy::ExternalAction,
            Remedy::Configure,
            Remedy::NoneAvailable,
        ] {
            assert!(
                !banner_offers_retry(&errored(remedy), true),
                "{remedy:?} is not fixed by reconnecting"
            );
        }
    }

    /// The cwd gate belongs to Retry alone. Signing in again fixes an expired
    /// login whether or not the pane has a working directory, so inheriting
    /// that gate would hide the button on exactly the failures it can fix.
    #[test]
    fn the_reauth_button_does_not_inherit_the_retry_cwd_gate() {
        assert!(banner_offers_reauth(&errored(Remedy::Reauthenticate)));
        assert!(!banner_offers_retry(&errored(Remedy::Reauthenticate), true));
    }

    /// An organization-blocked account is the case this separation exists for:
    /// signing in again succeeds and changes nothing, so the button must not
    /// appear next to a message that already says to ask an admin.
    #[test]
    fn only_a_reauthenticable_failure_offers_the_button() {
        for remedy in [
            Remedy::Retry,
            Remedy::ExternalAction,
            Remedy::Configure,
            Remedy::NoneAvailable,
        ] {
            assert!(
                !banner_offers_reauth(&errored(remedy)),
                "{remedy:?} is not fixed by signing in again"
            );
        }
    }

    #[test]
    fn no_status_but_error_offers_a_reauth_button() {
        for status in [
            AgentSessionStatus::Idle,
            AgentSessionStatus::Connecting,
            AgentSessionStatus::Connected,
        ] {
            assert!(!banner_offers_reauth(&status), "{status:?}");
        }
    }

    #[test]
    fn banner_offers_retry_only_for_a_transient_failure_on_a_connectable_pane() {
        assert!(banner_offers_retry(&errored(Remedy::Retry), true));
        // Mechanical gate still applies: no cwd, nothing to reconnect to.
        assert!(!banner_offers_retry(&errored(Remedy::Retry), false));
    }

    /// Every non-error status renders its own copy and never a retry button.
    #[test]
    fn banner_offers_no_retry_outside_the_error_status() {
        for status in [
            AgentSessionStatus::Idle,
            AgentSessionStatus::Connecting,
            AgentSessionStatus::Connected,
        ] {
            assert!(!banner_offers_retry(&status, true), "{status:?}");
        }
    }
}
