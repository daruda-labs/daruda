//! The orchestrator's lifetime.
//!
//! Lazy on purpose: starting one at launch costs a window, an agent process
//! and a slice of the account's rate limit on every day the user never says
//! `/daruda`. The first request pays for it instead.
//!
//! No session is resumed. `session/load` would carry yesterday's context into
//! today's conversation with no way for the user to see what it remembers, so
//! every run gets a fresh session.
//!
//! There is no separate state machine here. Whether the orchestrator is up is
//! exactly "does `WindowRegistry`'s orchestrator slot hold a window with a
//! chat pane", and a second copy of that answer could only disagree with it —
//! so [`pane`] asks the registry and nothing caches the result.

pub(crate) mod config;
pub(crate) mod window;

use gpui::App;

use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;

/// Why the orchestrator cannot take a prompt. Returned rather than reported:
/// the caller is answering a phone, and each of these needs its own wording
/// there. The internal detail behind `OpenFailed` is logged by
/// [`window::open`]'s caller before this is handed back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnsureError {
    /// `[orchestrator] enabled = false`.
    Disabled,
    /// Switched on, but the settings name no runnable agent.
    Unresolvable,
    OpenFailed(String),
}

/// Where to send a prompt, if the orchestrator is up. The single "is it
/// running?" question, answered from the registry.
pub(crate) fn pane(cx: &App) -> Option<PaneRef> {
    let (_, weak) = WindowRegistry::orchestrator(cx)?;
    weak.upgrade()?.read(cx).orchestrator_chat_pane()
}

/// Bring the orchestrator up if it is not, and answer where to send a prompt.
///
/// The ACP handshake is *not* waited on: it is bounded by `daruda_acp`'s own
/// connect timeout and its outcome arrives as a session event. The caller gets
/// the pane immediately and the prompt queues behind the connect — which is
/// what the phone's "accepted" reply already means.
pub(crate) fn ensure(cx: &mut App) -> Result<PaneRef, EnsureError> {
    // Resolve before reusing a live pane so disabling the feature takes effect
    // on the next request.
    let resolved = config::resolve(cx).ok_or_else(|| refusal(cx))?;
    if let Some(pane) = pane(cx) {
        return Ok(pane);
    }
    match window::open(&resolved, cx) {
        Ok(pane) => Ok(pane),
        Err(error) => {
            window::log_open_failure(&error);
            // Deliberately leaves nothing behind: a transient failure must not
            // wedge the feature, so the next `/daruda` retries from scratch.
            Err(EnsureError::OpenFailed(error.to_string()))
        }
    }
}

/// Which refusal an unresolvable configuration is. Split from [`ensure`] so
/// the two reasons the phone must distinguish — "you never turned this on" and
/// "you turned it on but it names no agent" — are decided in one place.
fn refusal(cx: &App) -> EnsureError {
    let config = crate::settings_store::SettingsStore::global(cx).user_arc();
    if config.orchestrator.enabled {
        EnsureError::Unresolvable
    } else {
        EnsureError::Disabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{BorrowAppContext as _, TestAppContext};

    /// Install a `SettingsStore` holding `config`. `ensure` reads it before
    /// any `Workspace` exists to install one.
    fn with_config(cx: &mut TestAppContext, config: daruda_config::Config) {
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
                store.set_user_for_testing(config);
            });
        });
    }

    fn enabled_naming(agent: Option<&str>) -> daruda_config::Config {
        daruda_config::Config {
            orchestrator: daruda_config::OrchestratorConfig {
                enabled: true,
                agent_id: agent.map(str::to_owned),
                account_id: None,
            },
            ..daruda_config::Config::default()
        }
    }

    #[gpui::test]
    fn nothing_is_up_until_something_brings_it_up(cx: &mut TestAppContext) {
        with_config(cx, daruda_config::Config::default());
        cx.update(|cx| assert!(pane(cx).is_none()));
    }

    #[gpui::test]
    fn switching_the_feature_off_refuses_even_with_one_already_up(cx: &mut TestAppContext) {
        let pane_ref = crate::test_support::register_test_orchestrator(cx);
        with_config(cx, enabled_naming(None));
        cx.update(|cx| assert_eq!(ensure(cx), Ok(pane_ref), "reused while enabled"));
        with_config(cx, daruda_config::Config::default());
        cx.update(|cx| {
            assert_eq!(ensure(cx), Err(EnsureError::Disabled));
            assert!(
                pane(cx).is_some(),
                "refusing does not tear the window down; closing it is the user's call"
            );
        });
    }

    #[gpui::test]
    fn a_disabled_orchestrator_refuses_without_opening_a_window(cx: &mut TestAppContext) {
        with_config(cx, daruda_config::Config::default());
        cx.update(|cx| {
            assert_eq!(ensure(cx), Err(EnsureError::Disabled));
            assert!(WindowRegistry::orchestrator(cx).is_none());
        });
    }

    /// Switched on but naming an agent the catalog does not hold: a different
    /// refusal from "off", because the fix is a different one.
    #[gpui::test]
    fn an_unresolvable_agent_is_its_own_refusal(cx: &mut TestAppContext) {
        with_config(cx, enabled_naming(Some("no-such-agent")));
        cx.update(|cx| {
            assert_eq!(ensure(cx), Err(EnsureError::Unresolvable));
            assert!(WindowRegistry::orchestrator(cx).is_none());
        });
    }
}
