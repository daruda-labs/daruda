use crate::control::resolve as control_resolve;
use crate::remote_channel::bridge::{
    BotPermissionOutcome, InboundAction, PaneRef, PermissionDecision, Routed, Unaimed,
};
use crate::surface::strings as s;
use crate::telegram::trace;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

/// The localized outcome label for a phone-tapped permission decision, shown
/// both as the callback toast and appended to the rewritten message. Applied →
/// the tapped direction (Allow/Reject); otherwise the reason it didn't apply.
pub(crate) fn permission_feedback(
    decision: &PermissionDecision,
    outcome: BotPermissionOutcome,
) -> String {
    match outcome {
        BotPermissionOutcome::Applied => match decision {
            PermissionDecision::Allow(_) => s::telegram_permission_allowed(),
            PermissionDecision::Reject(_) => s::telegram_permission_rejected(),
        },
        BotPermissionOutcome::Stale => s::telegram_permission_stale(),
        BotPermissionOutcome::Gone => s::telegram_permission_gone(),
    }
}

/// Compose the rewritten message body: the original prompt text with the outcome
/// appended on its own line, so the phone keeps the context of what was asked.
/// Falls back to just the label when the original text is unavailable.
pub(crate) fn compose_edit_body(original_text: &str, label: &str) -> String {
    if original_text.is_empty() {
        label.to_string()
    } else {
        format!("{original_text}\n\n— {label}")
    }
}

/// Enter every open `Workspace` window and run `on_match` against the one
/// whose persisted `WorkspaceUuid` equals `workspace` — the shared
/// `WindowRegistry::for_each_workspace` + `ws.uuid() == workspace` scaffolding
/// both [`InboundAction`] dispatch arms above need (a phone-relayed reply /
/// permission decision names its target pane by `PaneRef { workspace, pane }`,
/// but `PaneId` alone is only unique within one open window, so every window
/// must be checked). `pane.pane` itself is *not* checked against anything
/// here — the workspace-side handlers report a stale/gone pane id back to the
/// caller (`inject_bot_reply` returns `false`, `respond_bot_permission`
/// returns `Gone`), which then answers the phone rather than going quiet.
pub(crate) fn dispatch_to_workspace(
    cx: &mut gpui::AsyncApp,
    workspace: daruda_store::project::WorkspaceUuid,
    mut on_match: impl FnMut(&mut Workspace, &mut gpui::Context<Workspace>),
) {
    cx.update(|cx| {
        WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
            if ws.uuid() == workspace {
                on_match(ws, cx);
            }
        });
    });
}

/// What [`aim`] settled: the action to carry out, and whether the target in
/// it was the message's own or one this layer had to supply.
///
/// The second half is not a detail of how the answer was reached. A message
/// the bridge could route and a message that borrowed the app's focused lane
/// look identical from the phone, and only one of them went where the sender
/// meant — so the difference has to survive as far as the layer that can say
/// so out loud.
pub(crate) struct Aimed {
    pub action: InboundAction,
    /// The pane [`aim`] supplied because nothing named one. `None` when the
    /// message, the selection, or a prior ping already did.
    pub adopted: Option<PaneRef>,
}

impl Aimed {
    /// An action that already named its own target.
    fn named(action: InboundAction) -> Self {
        Self {
            action,
            adopted: None,
        }
    }

    /// An action aimed at `pane` only because the app offered it.
    fn borrowed(pane: PaneRef, action: InboundAction) -> Self {
        Self {
            action,
            adopted: Some(pane),
        }
    }
}

/// Name the chat [`aim`] lent a message, for the answer that tells the sender
/// where it went.
///
/// Read from the pane rather than the ordinal table on purpose: a sender who
/// never ran `/list` has no table, and "no chat was picked" is exactly the
/// state this reports. `None` once the pane is gone, which the caller is
/// already answering with a target-gone reply of its own.
pub(crate) fn lent_target_reply(
    pane: PaneRef,
    cx: &mut gpui::AsyncApp,
) -> Option<crate::remote_channel::command::RenderedReply> {
    let mut label = None;
    dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
        label = ws.control_chat_label(pane.pane, cx);
    });
    let label = label?;
    trace::delivery("target.lent", || {
        format!("pane={} agent={}", trace::pane(pane), label.agent)
    });
    Some(crate::remote_channel::command::RenderedReply {
        text: s::control_target_lent(&label.path, &label.agent_name),
        keyboard: None,
    })
}

/// Settle the target question `route` could not, and with it the only thing
/// standing between a routed update and being acted on.
///
/// The bridge's selection and last-pinged pane are in-memory, so a restart
/// leaves the phone unable to reach anything until it is re-aimed by hand —
/// even though the lane it was talking to is right there, restored. This is
/// the last link in the chain `plain_text_target` walks, resolved here rather
/// than in `route` because only this layer can read a workspace.
///
/// The only way to obtain an [`Aimed`] from a [`Routed`], which is what makes
/// skipping this a type error rather than a comment. With a target found,
/// both cases rejoin an arm that already existed — plain text is an
/// `InjectPrompt`, an unowned slash an `UnknownSlash` for that pane to claim
/// — and the pane travels out as `adopted`, because a lane this layer picked
/// is not one the sender did. Without one, both become their terminal answer,
/// and the message body they were carrying is recorded as lost here rather
/// than dropped by whichever arm happened to receive it.
pub(crate) fn aim(routed: Routed, cx: &mut gpui::AsyncApp) -> Aimed {
    let unaimed = match routed {
        Routed::Ready(action) => return Aimed::named(action),
        Routed::NeedsTarget(unaimed) => unaimed,
    };
    match (cx.update(control_resolve::sole_active_agent_chat), unaimed) {
        (Some(pane), Unaimed::Text { text }) => {
            trace::delivery("target.fallback", || {
                format!("pane={} kind=text", trace::pane(pane))
            });
            Aimed::borrowed(pane, InboundAction::InjectPrompt { pane, text })
        }
        (
            Some(pane),
            Unaimed::Slash {
                name,
                text,
                suggestion,
            },
        ) => {
            trace::delivery("target.fallback", || {
                format!("pane={} kind=slash name={name}", trace::pane(pane))
            });
            Aimed::borrowed(
                pane,
                InboundAction::UnknownSlash {
                    pane,
                    name,
                    text,
                    suggestion,
                },
            )
        }
        (None, Unaimed::Text { text }) => {
            trace::delivery("target.none", || {
                format!("kind=text len={}", text.chars().count())
            });
            Aimed::named(InboundAction::NoTarget)
        }
        (
            None,
            Unaimed::Slash {
                name,
                text,
                suggestion,
            },
        ) => {
            trace::delivery("target.none", || {
                format!("kind=slash name={name} len={}", text.chars().count())
            });
            Aimed::named(InboundAction::UnclaimedSlash { name, suggestion })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn compose_edit_body_appends_outcome_and_falls_back_when_empty() {
        assert_eq!(
            compose_edit_body("Allow write to /tmp/x?", "OK"),
            "Allow write to /tmp/x?\n\n— OK"
        );
        // No original text available → just the label, no stray separator.
        assert_eq!(compose_edit_body("", "OK"), "OK");
    }

    #[test]
    fn permission_feedback_distinguishes_direction_and_reason() {
        let allow = PermissionDecision::Allow("opt".into());
        let reject = PermissionDecision::Reject("opt".into());
        let applied_allow = permission_feedback(&allow, BotPermissionOutcome::Applied);
        let applied_reject = permission_feedback(&reject, BotPermissionOutcome::Applied);
        let stale = permission_feedback(&allow, BotPermissionOutcome::Stale);
        let gone = permission_feedback(&allow, BotPermissionOutcome::Gone);

        // Every outcome yields a distinct, non-empty label so the toast/edit is
        // never blank and Allow reads differently from Reject.
        for label in [&applied_allow, &applied_reject, &stale, &gone] {
            assert!(!label.is_empty());
        }
        assert_ne!(applied_allow, applied_reject);
        assert_ne!(applied_allow, stale);
        assert_ne!(stale, gone);
    }

    /// The restart case the whole fallback exists for: the bridge knows of no
    /// target, and the app's own active lane supplies one. Both cases rejoin
    /// the arms that already had a pane, so `/usage` reaches the agent and
    /// plain text reaches the same chat.
    #[gpui::test]
    async fn a_targetless_update_adopts_the_apps_own_lane(cx: &mut TestAppContext) {
        use crate::test_support::workspace_with_agent_chat;

        let fixture = workspace_with_agent_chat(cx);
        let expected = fixture.pane_ref(cx);
        let mut async_cx = cx.to_async();

        assert_eq!(
            aim(
                Routed::NeedsTarget(Unaimed::Text {
                    text: "ship it".into()
                }),
                &mut async_cx,
            )
            .action,
            InboundAction::InjectPrompt {
                pane: expected,
                text: "ship it".into()
            }
        );
        assert_eq!(
            aim(
                Routed::NeedsTarget(Unaimed::Slash {
                    name: "usage".into(),
                    text: "/usage".into(),
                    suggestion: Some("use"),
                }),
                &mut async_cx,
            )
            .action,
            InboundAction::UnknownSlash {
                pane: expected,
                name: "usage".into(),
                text: "/usage".into(),
                suggestion: Some("use"),
            }
        );
    }

    /// Ambiguity has to survive the aim step. Two windows each offering their
    /// own lane resolve to no candidate, and the update must fall to its
    /// terminal answer — picking one of the two would start a turn in
    /// whichever the walk happened to see first.
    #[gpui::test]
    async fn an_ambiguous_app_falls_to_the_terminal_answer(cx: &mut TestAppContext) {
        use crate::test_support::workspace_with_agent_chat;

        let _windows = [workspace_with_agent_chat(cx), workspace_with_agent_chat(cx)];
        let mut async_cx = cx.to_async();

        assert_eq!(
            aim(
                Routed::NeedsTarget(Unaimed::Text {
                    text: "ship it".into()
                }),
                &mut async_cx,
            )
            .action,
            InboundAction::NoTarget,
            "two candidates is not a target"
        );
        assert_eq!(
            aim(
                Routed::NeedsTarget(Unaimed::Slash {
                    name: "lst".into(),
                    text: "/lst".into(),
                    suggestion: Some("list"),
                }),
                &mut async_cx,
            )
            .action,
            InboundAction::UnclaimedSlash {
                name: "lst".into(),
                suggestion: Some("list"),
            },
            "the suggestion still reaches the layer that can use it"
        );
    }

    /// An update that already names its pane is handed straight back — `aim`
    /// resolves the target question, it does not revisit a settled one.
    #[gpui::test]
    async fn an_update_that_already_has_a_target_is_untouched(cx: &mut TestAppContext) {
        use crate::test_support::workspace_with_agent_chat;

        let fixture = workspace_with_agent_chat(cx);
        let pane = fixture.pane_ref(cx);
        let mut async_cx = cx.to_async();

        let action = InboundAction::InjectPrompt {
            pane,
            text: "already aimed".into(),
        };
        assert_eq!(
            aim(Routed::Ready(action.clone()), &mut async_cx).action,
            action
        );
    }
}
