use super::RemoteChannels;
use crate::remote_channel::{
    self,
    dispatch::{self, Edit, Effect},
    pairing::pair_code,
    runtime::{WorkerEvent, WorkerPayload},
    transport::{self, IncomingKind, Message},
};
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use futures::{StreamExt, channel::mpsc::UnboundedReceiver};
use gpui::App;

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
                        if let Some(connection) = cx
                            .global_mut::<RemoteChannels>()
                            .connections
                            .get_mut(&event.id)
                        {
                            connection.status = status;
                        }
                    });
                    continue;
                }
                WorkerPayload::Credentials(credentials) => {
                    cx.update(|cx| {
                        if let Some(connection) = cx
                            .global_mut::<RemoteChannels>()
                            .connections
                            .get_mut(&event.id)
                        {
                            connection.credentials = Some(credentials);
                        }
                    });
                    continue;
                }
                WorkerPayload::Incoming(incoming) => incoming,
            };
            let duplicate = cx.update(|cx| {
                cx.global::<RemoteChannels>()
                    .state
                    .contains(&event.id, &incoming.event_id)
            });
            if duplicate {
                continue;
            }
            let pairing = cx.update(|cx| {
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
                matched
                    .then(|| (connection.config.kind, connection.credentials.clone()))
                    .and_then(|(kind, credentials)| Some((kind, credentials?)))
            });
            if let Some((kind, credentials)) = pairing {
                let recipient = incoming.sender.clone();
                let persisted = cx.update(|cx| {
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
                    let message = Message::plain(s::remote_pair_success(), None);
                    let result = cx
                        .background_executor()
                        .spawn(
                            async move { transport::send(kind, &credentials, &recipient, message) },
                        )
                        .await;
                    if let Err(error) = result {
                        remote_channel::log_error(
                            "Remote pairing confirmation failed",
                            &error,
                            "remote.pair.confirm",
                        );
                    }
                }
                remember(cx, &event.id, incoming.event_id).await;
                continue;
            }
            // Every pairing attempt stops here, matched or not. A gateway
            // redelivery would otherwise reach an already-paired channel's
            // routing path and inject the code into an agent as a prompt.
            if let IncomingKind::Message { text, .. } = &incoming.kind
                && pair_code(text).is_some()
            {
                remember(cx, &event.id, incoming.event_id).await;
                continue;
            }
            let routed = cx.update(|cx| {
                let bridge = cx.global::<RemoteChannels>();
                let connection = bridge.live(&event.id, cx)?;
                if !connection.config.accepts(&incoming.sender) {
                    return None;
                }
                let connection = cx
                    .global_mut::<RemoteChannels>()
                    .connections
                    .get_mut(&event.id)?;
                Some(match &incoming.kind {
                    IncomingKind::Message { text, reply_to } => {
                        let text = text
                            .strip_prefix('!')
                            .map_or_else(|| text.clone(), |command| format!("/{command}"));
                        connection.core.route_text(text, reply_to.clone())
                    }
                    IncomingKind::Callback { data, .. } => {
                        crate::remote_channel::bridge::Routed::Ready(
                            connection.core.route_callback(data.clone()),
                        )
                    }
                })
            });
            let Some(routed) = routed else {
                continue;
            };
            let action = dispatch::target::aim(routed, cx);
            let effect = dispatch::handle(action, &dispatch::Target::Remote(event.id.clone()), cx);
            // Executed effects are remembered before sending their feedback; a
            // failed send must not re-run a command.
            remember(cx, &event.id, incoming.event_id).await;
            let delivery = cx.update(|cx| {
                let connection = cx.global::<RemoteChannels>().live(&event.id, cx)?;
                if connection.generation != event.generation {
                    return None;
                }
                Some((
                    connection.config.kind,
                    connection.credentials.clone()?,
                    connection.config.recipient.clone()?,
                ))
            });
            let Some((kind, credentials, recipient)) = delivery else {
                continue;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    match feedback_for(effect, incoming.kind) {
                        Feedback::Nothing => Ok(()),
                        Feedback::Send(message) => {
                            transport::send(kind, &credentials, &recipient, message).map(|_| ())
                        }
                        Feedback::Edit {
                            message_id,
                            original,
                            label,
                        } => transport::edit(
                            kind,
                            &credentials,
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

/// Remember one handled event so a gateway redelivery cannot run it twice.
async fn remember(cx: &mut gpui::AsyncApp, id: &str, event_id: String) {
    let state = cx.update(|cx| {
        let bridge = cx.global_mut::<RemoteChannels>();
        bridge.state.record(id, event_id);
        bridge.state.clone()
    });
    let persisted = cx
        .background_executor()
        .spawn(async move { state.save_in(&daruda_store::persistence::default_data_dir()) })
        .await;
    if let Err(error) = persisted {
        remote_channel::log_error(
            "Remote event history failed to persist",
            &error,
            "remote.history.persist",
        );
    }
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
            label: s::remote_stale_callback(),
        },
        (Effect::None, None) => Feedback::Nothing,
    }
}

#[cfg(test)]
mod tests;
