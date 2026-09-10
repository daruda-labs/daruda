//! Resolving what an inbound control message refers to, against the live app.
//!
//! Two questions every adapter has to answer before it can act on a message
//! that names nothing: *which pane did they mean*, and *whose command is this*.
//! Both are asked of the whole app rather than one window, both carry a policy
//! about what to do when the app cannot say, and neither is specific to the
//! adapter that asked — the Telegram bridge got here first, but the MCP
//! surface and the orchestrator route by the same rules.
//!
//! The walk itself (`for_each_workspace` with an accumulator) is the idiom
//! `exec.rs` already uses throughout; what lives here is the *policy* folded
//! over it.

use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;
use crate::workspace::SlashClaim;

/// The one agent chat the app is pointing at, across every window.
///
/// Exactly one candidate or none. Two windows each offering their own active
/// lane is the same ambiguity as two chats inside one lane, and gets the same
/// answer — a message put on the wrong agent starts a turn nobody asked for,
/// which is worse than telling the sender to name one.
pub(crate) fn sole_active_agent_chat(cx: &mut gpui::App) -> Option<PaneRef> {
    let mut candidates = Vec::new();
    WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
        candidates.extend(ws.fallback_agent_chat());
    });
    match candidates.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// What the app's agents collectively say about a `/name` daruda does not own.
///
/// The same fold each window runs over its own panes, one level up: a claim in
/// any window settles it for all of them, a disclaimer counts only if nothing
/// claimed, and silence everywhere — no window open, no agent chat, or every
/// session still cold — earns no conclusion in either direction.
pub(crate) fn slash_claim(cx: &mut gpui::App, name: &str) -> SlashClaim {
    let mut claim = SlashClaim::Unsaid;
    WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
        claim = claim.merge(ws.slash_claim(name, cx));
    });
    claim
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::workspace_with_agent_chat;
    use gpui::TestAppContext;

    /// One window with one chat is not a guess, so it resolves.
    #[gpui::test]
    async fn a_single_window_offers_its_lane(cx: &mut TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let expected = fixture.pane_ref(cx);
        cx.update(|cx| assert_eq!(sole_active_agent_chat(cx), Some(expected)));
    }

    /// Two windows each offering their own lane is ambiguous. The caller hears
    /// nothing rather than a coin flip.
    #[gpui::test]
    async fn two_windows_each_offering_a_lane_resolve_to_nothing(cx: &mut TestAppContext) {
        let _windows = [workspace_with_agent_chat(cx), workspace_with_agent_chat(cx)];
        cx.update(|cx| assert_eq!(sole_active_agent_chat(cx), None));
    }

    /// A claim in one window settles the name for every window — otherwise
    /// opening a second window would be enough to answer the first window's
    /// agent command as a typo.
    #[gpui::test]
    async fn one_windows_claim_settles_it_for_all(cx: &mut TestAppContext) {
        let windows = [workspace_with_agent_chat(cx), workspace_with_agent_chat(cx)];
        for (fixture, commands) in windows.iter().zip([["usage", "model"], ["model", "cost"]]) {
            let pane = fixture.pane();
            fixture.workspace.update(cx, |ws, cx| {
                ws.advertise_slash_commands_for_test(pane, &commands, cx);
            });
        }
        cx.update(|cx| {
            assert_eq!(slash_claim(cx, "usage"), SlashClaim::Claims);
            assert!(!slash_claim(cx, "usage").rules_out());
            assert!(
                slash_claim(cx, "lst").rules_out(),
                "no agent anywhere has /lst, so the suggestion is daruda's to give"
            );
        });
    }
}
