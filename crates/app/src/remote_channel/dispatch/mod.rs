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

pub fn handle(action: InboundAction, target: &Target, cx: &mut gpui::AsyncApp) -> Effect {
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
        InboundAction::InjectPrompt { pane, text } => deliver_prompt(pane, text, target, cx),
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
                deliver_prompt(pane, text, target, cx)
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

fn deliver_prompt(pane: PaneRef, text: String, target: &Target, cx: &mut gpui::AsyncApp) -> Effect {
    let mut delivered = false;
    dispatch_to_workspace(cx, pane.workspace, |ws, cx| {
        delivered = ws.inject_bot_reply(pane.pane, text.clone(), cx);
    });
    if delivered {
        Effect::None
    } else {
        cx.update(|cx| command::report_target_gone(pane, target, cx))
            .map_or(Effect::None, Effect::Reply)
    }
}

#[cfg(test)]
mod tests {
    use crate::remote_channel::bridge::{InboundAction, PaneRef};

    #[gpui::test]
    async fn unsupported_actions_do_not_require_connection_state(cx: &mut gpui::TestAppContext) {
        let effect = super::handle(
            InboundAction::Unsupported,
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
                action,
                &super::Target::Remote("missing".into()),
                &mut cx.to_async(),
            );
            assert!(matches!(effect, super::Effect::None));
        }
    }
}
