//! Execute a routed action once, independently of its delivery transport.

use super::{
    bridge::{BotPermissionOutcome, InboundAction, PaneRef},
    command::{self as commands, CommandState, RenderedReply},
};
use crate::control::{result::ControlError, spec::ParseError};
use crate::surface::strings as s;
use gpui::App;

mod command;
pub(crate) mod target;
pub(crate) use target::Aimed;
use target::{dispatch_to_workspace, permission_feedback};

pub enum Target {
    Telegram,
    Remote(String),
}

impl Target {
    /// `None` once the bridge global is gone or the connection was revoked —
    /// an answer with no table to resolve against is dropped, not panicked on.
    fn state<'a>(&self, cx: &'a mut App) -> Option<&'a mut CommandState> {
        match self {
            Self::Telegram => {
                if !cx.has_global::<crate::telegram::global::TelegramBridge>() {
                    return None;
                }
                Some(
                    cx.global_mut::<crate::telegram::global::TelegramBridge>()
                        .command_state_mut(),
                )
            }
            Self::Remote(id) => {
                if !cx.has_global::<super::global::RemoteChannels>() {
                    return None;
                }
                Some(
                    cx.global_mut::<super::global::RemoteChannels>()
                        .connections
                        .get_mut(id)?
                        .core
                        .command_state_mut(),
                )
            }
        }
    }
}

pub enum Effect {
    None,
    Reply(RenderedReply),
    Feedback { label: String, edit: Edit },
}

#[derive(PartialEq, Eq)]
pub enum Edit {
    KeepButtons,
    ConsumeButtons,
}

pub fn handle(aimed: Aimed, target: &Target, cx: &mut gpui::AsyncApp) -> Effect {
    let Aimed { action, adopted } = aimed;
    match action {
        InboundAction::RunCommand { command: requested } => {
            command::run_command(requested, target, cx).map_or(Effect::None, Effect::Reply)
        }
        InboundAction::ReportParseError { error } => {
            Effect::Reply(commands::render_parse_error(&error))
        }
        InboundAction::NoTarget => cx
            .update(|cx| command::render_outcome(&Err(ControlError::NoTargetSelected), target, cx))
            .map_or(Effect::None, Effect::Reply),
        InboundAction::InjectPrompt { pane, text } => {
            deliver_prompt(pane, text, adopted, target, cx)
        }
        InboundAction::UnknownSlash {
            pane,
            name,
            text,
            suggestion,
        } => {
            let mut agents = true;
            dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                agents = ws.agent_takes_slash_command(pane.pane, &name, cx);
            });
            if agents {
                deliver_prompt(pane, text, adopted, target, cx)
            } else {
                Effect::Reply(commands::render_parse_error(&ParseError::Unknown {
                    input: name,
                    suggestion,
                }))
            }
        }
        InboundAction::UnclaimedSlash { name, suggestion } => {
            let ours = cx.update(|cx| crate::control::resolve::slash_claim(cx, &name).rules_out());
            if ours {
                Effect::Reply(commands::render_parse_error(&ParseError::Unknown {
                    input: name,
                    suggestion,
                }))
            } else {
                cx.update(|cx| {
                    command::render_outcome(&Err(ControlError::NoTargetSelected), target, cx)
                })
                .map_or(Effect::None, Effect::Reply)
            }
        }
        InboundAction::RespondPermission {
            pane,
            perm_id,
            decision,
        } => {
            let mut outcome = BotPermissionOutcome::Gone;
            dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
                outcome = ws.respond_bot_permission(pane.pane, perm_id, decision.clone(), cx);
            });
            Effect::Feedback {
                label: permission_feedback(&decision, outcome),
                edit: Edit::ConsumeButtons,
            }
        }
        InboundAction::SelectTarget { pane } => cx
            .update(|cx| command::select_target(pane, target, cx))
            .map_or(Effect::None, |label| Effect::Feedback {
                label,
                edit: Edit::KeepButtons,
            }),
        InboundAction::ResolveApproval { id, choice } => {
            let applied = cx.update(|cx| crate::control::approval::resolve(id, choice, cx));
            let label = match (applied, choice) {
                (false, _) => s::control_approval_already_answered(),
                (true, crate::control::approval::ApprovalChoice::Approved) => {
                    s::control_approval_allowed()
                }
                (true, crate::control::approval::ApprovalChoice::Refused) => {
                    s::control_approval_refused()
                }
            };
            Effect::Feedback {
                label,
                edit: Edit::KeepButtons,
            }
        }
        InboundAction::StaleListing => Effect::Feedback {
            label: s::control_listing_stale(),
            edit: Edit::KeepButtons,
        },
        InboundAction::Ignore | InboundAction::Unsupported | InboundAction::Paired { .. } => {
            Effect::None
        }
    }
}

/// Put a phone-sent prompt on its pane, and answer for it when the sender
/// cannot see what happened.
///
/// Two of those cases. The pane is gone, so nothing was delivered; or it was
/// delivered to a chat `aim` lent rather than one the sender picked, which is
/// silent in exactly the way that starts a turn on the wrong agent.
fn deliver_prompt(
    pane: PaneRef,
    text: String,
    adopted: Option<PaneRef>,
    target: &Target,
    cx: &mut gpui::AsyncApp,
) -> Effect {
    let mut delivered = false;
    dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
        delivered = ws.inject_bot_reply(pane.pane, text.clone(), cx);
    });
    if !delivered {
        return cx
            .update(|cx| command::report_target_gone(pane, target, cx))
            .map_or(Effect::None, Effect::Reply);
    }
    adopted
        .and_then(|lent| target::lent_target_reply(lent, cx))
        .map_or(Effect::None, Effect::Reply)
}

#[cfg(test)]
mod tests {
    use crate::remote_channel::bridge::{InboundAction, PaneRef, Routed, Unaimed};

    /// A message that named no chat, on a bridge that remembered none, still
    /// reaches an agent — `aim` lends it the app's own lane. The sender is told
    /// which one, because from the phone a lent lane and the one they were
    /// talking to look exactly alike, and only one of them is what they meant.
    #[gpui::test]
    async fn a_lent_target_is_reported_back_to_the_sender(cx: &mut gpui::TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let pane = fixture.pane();
        let expected = fixture
            .workspace
            .read_with(cx, |ws, cx| ws.control_chat_label(pane, cx))
            .expect("the fixture's pane has a label");
        let mut async_cx = cx.to_async();

        let aimed = super::target::aim(
            Routed::NeedsTarget(Unaimed::Text {
                text: "ship it".into(),
            }),
            &mut async_cx,
        );
        let effect = super::handle(aimed, &super::Target::Telegram, &mut async_cx);

        match effect {
            super::Effect::Reply(reply) => assert_eq!(
                reply.text,
                crate::surface::strings::control_target_lent(&expected.path, &expected.agent_name),
            ),
            _ => panic!("a lent target must be reported, not silently used"),
        }
    }

    /// The other half, and the reason the notice is worth having: a message
    /// that named its own chat is delivered without a word. Told every time,
    /// the notice would be chatter the sender learns to scroll past — which is
    /// the one thing it cannot afford to become.
    #[gpui::test]
    async fn a_target_the_sender_named_is_delivered_in_silence(cx: &mut gpui::TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let pane = fixture.pane_ref(cx);
        let mut async_cx = cx.to_async();

        let effect = super::handle(
            super::Aimed {
                action: InboundAction::InjectPrompt {
                    pane,
                    text: "ship it".into(),
                },
                adopted: None,
            },
            &super::Target::Telegram,
            &mut async_cx,
        );

        assert!(
            matches!(effect, super::Effect::None),
            "a chat the sender picked needs no answer about where the message went"
        );
    }

    #[gpui::test]
    async fn unsupported_actions_do_not_require_connection_state(cx: &mut gpui::TestAppContext) {
        let effect = super::handle(
            super::Aimed {
                action: InboundAction::Unsupported,
                adopted: None,
            },
            &super::Target::Remote("missing".into()),
            &mut cx.to_async(),
        );
        assert!(matches!(effect, super::Effect::None));
    }

    /// Every arm that resolves against an ordinal table must survive the
    /// connection being revoked mid-flight: an answer with no table to render
    /// against is dropped, never panicked on.
    #[gpui::test]
    async fn a_revoked_connection_drops_the_answer_instead_of_panicking(
        cx: &mut gpui::TestAppContext,
    ) {
        let pane = PaneRef {
            workspace: Default::default(),
            pane: 1,
        };
        // The bridge installed and running, but this channel revoked: the
        // shape the removed `.expect("current remote connection")` crashed on.
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            crate::remote_channel::global::install(cx);
        });
        for action in [
            InboundAction::NoTarget,
            InboundAction::SelectTarget { pane },
            InboundAction::UnclaimedSlash {
                name: "usage".into(),
                suggestion: None,
            },
        ] {
            let effect = super::handle(
                super::Aimed {
                    action,
                    adopted: None,
                },
                &super::Target::Remote("missing".into()),
                &mut cx.to_async(),
            );
            assert!(matches!(effect, super::Effect::None));
        }
    }
}
