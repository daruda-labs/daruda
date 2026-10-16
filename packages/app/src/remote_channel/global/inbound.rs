use super::RemoteChannels;
use crate::remote_channel::{
    self,
    dispatch::{self, Edit, Effect},
    pairing::pair_code,
    runtime::{BotSession, WorkerEvent, WorkerPayload, channel_key},
    transport::{self, IncomingKind, Message},
};
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use futures::{StreamExt, channel::mpsc::UnboundedReceiver};
use gpui::App;
use std::sync::Arc;

fn live_session(id: &str, generation: uuid::Uuid, bot: &Arc<BotSession>, cx: &App) -> bool {
    bot.is_live()
        && cx
            .global::<RemoteChannels>()
            .live(id, cx)
            .is_some_and(|connection| {
                connection.generation == generation
                    && connection
                        .bot
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, bot))
            })
}

async fn claim_incoming(
    id: &str,
    generation: uuid::Uuid,
    event_id: &str,
    cx: &mut gpui::AsyncApp,
) -> Option<Arc<BotSession>> {
    let (bot, config) = cx.update(|cx| {
        let connection = cx.global::<RemoteChannels>().live(id, cx)?;
        let bot = connection.bot.clone()?;
        (connection.generation == generation && bot.is_live())
            .then(|| (bot, connection.config.clone()))
    })?;
    let claimed = crate::runs::RunsHost::claim_inbound(
        &bot.inbox,
        channel_key(config.kind),
        daruda_store::inbox::InboxId::Opaque(event_id.into()),
    );
    finish_claim(id, generation, bot, config, claimed, cx).await
}

async fn finish_claim(
    id: &str,
    generation: uuid::Uuid,
    bot: Arc<BotSession>,
    config: daruda_config::remote::ChannelConfig,
    claimed: impl Future<
        Output = Result<
            daruda_store::inbox::Committed<daruda_store::inbox::InboxMachine>,
            daruda_store::inbox::CommitError<daruda_store::inbox::InboxRejected>,
        >,
    >,
    cx: &mut gpui::AsyncApp,
) -> Option<Arc<BotSession>> {
    let claimed = claimed.await;
    match claimed {
        Ok(_) => {}
        Err(daruda_store::inbox::CommitError::Rejected(
            daruda_store::inbox::InboxRejected::Duplicate { .. },
        )) => return None,
        Err(error) => {
            remote_channel::log_error("Remote command claim failed", &error, "remote.inbox.claim");
            cx.update(|cx| RemoteChannels::recover_inbox(id, generation, &bot, cx));
            return None;
        }
    }
    cx.update(|cx| {
        (live_session(id, generation, &bot, cx)
            && cx
                .global::<RemoteChannels>()
                .live(id, cx)
                .is_some_and(|connection| connection.config == config))
        .then_some(bot)
    })
}

fn dispatch_claimed(
    id: &str,
    generation: uuid::Uuid,
    bot: &Arc<BotSession>,
    incoming: &transport::Incoming,
    cx: &mut App,
) -> Option<Effect> {
    if !live_session(id, generation, bot, cx) {
        return None;
    }
    let connection = cx.global::<RemoteChannels>().live(id, cx)?;
    if !connection.config.accepts(&incoming.sender) {
        return None;
    }
    let connection = cx.global_mut::<RemoteChannels>().connections.get_mut(id)?;
    let routed = match &incoming.kind {
        IncomingKind::Message { text, reply_to } => {
            let text = text
                .strip_prefix('!')
                .map_or_else(|| text.clone(), |command| format!("/{command}"));
            connection.core.route_text(text, reply_to.clone())
        }
        IncomingKind::Callback { data, .. } => crate::remote_channel::bridge::Routed::Ready(
            connection.core.route_callback(data.clone()),
        ),
    };
    let aimed = dispatch::target::aim_in_app(routed, cx);
    Some(dispatch::handle_in_app(
        aimed,
        &dispatch::Target::Remote(id.into()),
        cx,
    ))
}

pub(super) fn spawn(mut receiver: UnboundedReceiver<WorkerEvent>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(event) = receiver.next().await {
            if !cx.update(|cx| {
                cx.global::<RemoteChannels>()
                    .live(&event.id, cx)
                    .is_some_and(|connection| connection.generation == event.generation)
            }) {
                continue;
            }
            let incoming = match event.payload {
                WorkerPayload::Status(status) => {
                    cx.update(|cx| {
                        if !cx
                            .global::<RemoteChannels>()
                            .live(&event.id, cx)
                            .is_some_and(|connection| connection.generation == event.generation)
                        {
                            return;
                        }
                        if let Some(connection) = cx
                            .global_mut::<RemoteChannels>()
                            .connections
                            .get_mut(&event.id)
                        {
                            connection.status = status;
                            if !status.holds_claim()
                                && let Some(bot) = connection.bot.take()
                            {
                                bot.revoke();
                            }
                        }
                    });
                    continue;
                }
                WorkerPayload::Ready(bot) => {
                    cx.update(|cx| {
                        if !cx
                            .global::<RemoteChannels>()
                            .live(&event.id, cx)
                            .is_some_and(|connection| connection.generation == event.generation)
                        {
                            return;
                        }
                        if let Some(connection) = cx
                            .global_mut::<RemoteChannels>()
                            .connections
                            .get_mut(&event.id)
                        {
                            connection.bot = Some(bot);
                        }
                    });
                    continue;
                }
                WorkerPayload::Incoming(incoming) => incoming,
            };
            let Some(bot) =
                claim_incoming(&event.id, event.generation, &incoming.event_id, cx).await
            else {
                continue;
            };
            let pairing = cx.update(|cx| {
                if !live_session(&event.id, event.generation, &bot, cx) {
                    return None;
                }
                let connection = cx
                    .global_mut::<RemoteChannels>()
                    .connections
                    .get_mut(&event.id)?;
                if connection.config.recipient.is_some() {
                    return None;
                }
                let IncomingKind::Message { text, .. } = &incoming.kind else {
                    return None;
                };
                let matched = connection
                    .pairing
                    .as_mut()
                    .is_some_and(|pairing| pairing.matches(text));
                matched.then_some(connection.config.kind)
            });
            if let Some(kind) = pairing {
                let recipient = incoming.sender.clone();
                let persisted = cx.update(|cx| {
                    if !live_session(&event.id, event.generation, &bot, cx) {
                        return false;
                    }
                    let mut channels = SettingsStore::global(cx).user().remote.channels.clone();
                    let Some(channel) = channels.iter_mut().find(|channel| channel.id == event.id)
                    else {
                        return false;
                    };
                    if channel.recipient.is_some() || !channel.enabled {
                        return false;
                    }
                    channel.recipient = Some(recipient.clone());
                    match cx
                        .global_mut::<SettingsStore>()
                        .apply_patch(daruda_config::SettingsPatch::RemoteChannels(channels))
                    {
                        Ok(()) => {
                            super::reconcile(cx);
                            true
                        }
                        Err(error) => {
                            remote_channel::log_error(
                                "Remote pairing failed to persist",
                                &error,
                                "remote.pair.persist",
                            );
                            false
                        }
                    }
                });
                if persisted {
                    let message = Message::plain(s::remote_channel::pair_success(), None);
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            transport::send(kind, &bot.credentials, &recipient, message)
                        })
                        .await;
                    if let Err(error) = result {
                        remote_channel::log_error(
                            "Remote pairing confirmation failed",
                            &error,
                            "remote.pair.confirm",
                        );
                    }
                }
                continue;
            }
            // Every pairing attempt stops here, matched or not. A gateway
            // redelivery would otherwise reach an already-paired channel's
            // routing path and inject the code into an agent as a prompt.
            if let IncomingKind::Message { text, .. } = &incoming.kind
                && pair_code(text).is_some()
            {
                continue;
            }
            let effect =
                cx.update(|cx| dispatch_claimed(&event.id, event.generation, &bot, &incoming, cx));
            let Some(effect) = effect else {
                continue;
            };
            let effect = effect.settled().await;
            let delivery = cx.update(|cx| {
                if !live_session(&event.id, event.generation, &bot, cx) {
                    return None;
                }
                let connection = cx.global::<RemoteChannels>().live(&event.id, cx)?;
                if connection.generation != event.generation {
                    return None;
                }
                Some((
                    connection.config.kind,
                    bot.clone(),
                    connection.config.recipient.clone()?,
                ))
            });
            let Some((kind, bot, recipient)) = delivery else {
                continue;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    match feedback_for(effect, incoming.kind) {
                        Feedback::Nothing => Ok(()),
                        Feedback::Send(message) => {
                            transport::send(kind, &bot.credentials, &recipient, message).map(|_| ())
                        }
                        Feedback::Edit {
                            message_id,
                            original,
                            label,
                        } => transport::edit(
                            kind,
                            &bot.credentials,
                            &recipient,
                            &message_id,
                            &original,
                            &label,
                        ),
                    }
                })
                .await;
            if let Err(error) = result {
                remote_channel::log_error(
                    "Remote command feedback failed",
                    &error,
                    "remote.feedback.send",
                );
            }
        }
    })
    .detach();
}

/// What one handled effect owes the sender.
#[derive(Debug, PartialEq, Eq)]
enum Feedback {
    Nothing,
    Send(Message),
    Edit {
        message_id: String,
        original: String,
        label: String,
    },
}

/// Split from the send so the edit-vs-new-message choice is decidable without
/// a transport. A tapped button is always answered in place: leaving a
/// consumed keyboard on screen invites a decision that can no longer land.
fn feedback_for(effect: Effect, kind: IncomingKind) -> Feedback {
    let tapped = match kind {
        IncomingKind::Callback {
            message_id,
            original,
            ..
        } => Some((message_id, original)),
        IncomingKind::Message { .. } => None,
    };
    match (effect, tapped) {
        (Effect::Reply(reply), _) => Feedback::Send(Message::plain(reply.text, reply.keyboard)),
        (
            Effect::Feedback {
                label,
                edit: Edit::ConsumeButtons,
            },
            Some((message_id, original)),
        ) => Feedback::Edit {
            message_id,
            original,
            label,
        },
        (Effect::Feedback { label, .. }, _) => Feedback::Send(Message::plain(label, None)),
        // An unknown or already-consumed token: the tap still gets an answer,
        // and the dead buttons still come off.
        (Effect::None, Some((message_id, original))) => Feedback::Edit {
            message_id,
            original,
            label: s::remote_channel::stale_callback(),
        },
        (Effect::None, None) => Feedback::Nothing,
        // The loop settles an effect before asking what it is owed; one that
        // is still waiting has nothing to send yet.
        (Effect::Later(_), _) => Feedback::Nothing,
    }
}

#[cfg(test)]
mod tests;
