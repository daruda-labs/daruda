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
use daruda_store::inbox::{
    ChannelKey, CommitError, Committed, InboxId, InboxMachine, InboxRejected, InboxStore,
};
use futures::{StreamExt, channel::mpsc::UnboundedReceiver};
use gpui::App;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

type ClaimResult = Result<Committed<InboxMachine>, CommitError<InboxRejected>>;

/// What claiming one incoming command came to.
enum Claimed {
    /// Claimed now; act on it with this bot.
    Ready(Arc<BotSession>),
    /// Already handled, or no longer this connection's to handle.
    Skip,
    /// The writer failed before the claim was durable. The command is kept and
    /// claimed again once the inbox reopens, as Telegram retries its update:
    /// both gateways acknowledge an event before it reaches here, so neither
    /// Slack nor Discord will send it again.
    Retry,
}

/// Commands whose claim failed, kept in arrival order per connection until its
/// worker publishes a reopened inbox. Later commands queue behind them so none
/// is acted on ahead of one the user sent first.
#[derive(Default)]
struct Backlog(HashMap<String, (uuid::Uuid, VecDeque<transport::Incoming>)>);

impl Backlog {
    /// Queue behind held commands, or hand `incoming` back when none are held.
    fn admit(
        &mut self,
        id: &str,
        generation: uuid::Uuid,
        incoming: transport::Incoming,
    ) -> Option<transport::Incoming> {
        match self.0.get_mut(id) {
            Some((held, queue)) if *held == generation => {
                queue.push_back(incoming);
                None
            }
            _ => {
                self.0.remove(id);
                Some(incoming)
            }
        }
    }

    fn hold(&mut self, id: &str, generation: uuid::Uuid, queue: VecDeque<transport::Incoming>) {
        self.0.insert(id.to_owned(), (generation, queue));
    }

    /// Everything held for this generation; a replaced generation's commands
    /// are dropped with the connection that received them.
    fn take(&mut self, id: &str, generation: uuid::Uuid) -> VecDeque<transport::Incoming> {
        match self.0.remove(id) {
            Some((held, queue)) if held == generation => queue,
            _ => VecDeque::new(),
        }
    }
}

/// Events from a replaced or disabled connection's worker are stale.
fn live_generation(id: &str, generation: uuid::Uuid, cx: &App) -> bool {
    cx.global::<RemoteChannels>()
        .live(id, cx)
        .is_some_and(|connection| connection.generation == generation)
}

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

async fn claim_incoming<F: Future<Output = ClaimResult>>(
    id: &str,
    generation: uuid::Uuid,
    event_id: &str,
    claim: &impl Fn(&InboxStore, ChannelKey, InboxId) -> F,
    cx: &mut gpui::AsyncApp,
) -> Claimed {
    let Some((bot, config)) = cx.update(|cx| {
        let connection = cx.global::<RemoteChannels>().live(id, cx)?;
        let bot = connection.bot.clone()?;
        (connection.generation == generation && bot.is_live())
            .then(|| (bot, connection.config.clone()))
    }) else {
        return Claimed::Skip;
    };
    let claimed = claim(
        &bot.inbox,
        channel_key(config.kind),
        InboxId::Opaque(event_id.into()),
    );
    finish_claim(id, generation, bot, config, claimed, cx).await
}

async fn finish_claim(
    id: &str,
    generation: uuid::Uuid,
    bot: Arc<BotSession>,
    config: daruda_config::remote::ChannelConfig,
    claimed: impl Future<Output = ClaimResult>,
    cx: &mut gpui::AsyncApp,
) -> Claimed {
    match claimed.await {
        Ok(_) => {}
        Err(CommitError::Rejected(InboxRejected::Duplicate { .. })) => return Claimed::Skip,
        Err(error) => {
            remote_channel::log_error("Remote command claim failed", &error, "remote.inbox.claim");
            cx.update(|cx| RemoteChannels::recover_inbox(id, generation, &bot, cx));
            return Claimed::Retry;
        }
    }
    cx.update(|cx| {
        let current = live_session(id, generation, &bot, cx)
            && cx
                .global::<RemoteChannels>()
                .live(id, cx)
                .is_some_and(|connection| connection.config == config);
        if !current {
            return Claimed::Skip;
        }
        if let Some(connection) = cx.global_mut::<RemoteChannels>().connections.get_mut(id) {
            connection.claim_backoff.reset();
        }
        Claimed::Ready(bot)
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
    let effect = dispatch::handle_in_app(aimed, &dispatch::Target::Remote(id.into()), cx);
    if let IncomingKind::Callback {
        data, message_id, ..
    } = &incoming.kind
    {
        let connection = cx.global_mut::<RemoteChannels>().connections.get_mut(id)?;
        if matches!(
            &effect,
            Effect::Feedback {
                edit: Edit::ManagedCard,
                ..
            }
        ) || connection.core.is_settled_callback(data)
        {
            // The callback identifies the message before the send response does.
            connection.cards.get_mut().note_settled(message_id.clone());
        }
    }
    Some(effect)
}

pub(super) fn spawn(receiver: UnboundedReceiver<WorkerEvent>, cx: &mut App) {
    serve_events(receiver, crate::runs::RunsHost::claim_inbound, cx);
}

/// The inbound loop, with the inbox claim passed in so a failing writer can be
/// staged without a failing disk.
fn serve_events<F: Future<Output = ClaimResult> + 'static>(
    mut receiver: UnboundedReceiver<WorkerEvent>,
    claim: impl Fn(&InboxStore, ChannelKey, InboxId) -> F + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let mut backlog = Backlog::default();
        while let Some(event) = receiver.next().await {
            if !cx.update(|cx| live_generation(&event.id, event.generation, cx)) {
                continue;
            }
            let mut ready = match event.payload {
                WorkerPayload::Status(status) => {
                    cx.update(|cx| {
                        if !live_generation(&event.id, event.generation, cx) {
                            return;
                        }
                        if let Some(connection) = cx
                            .global_mut::<RemoteChannels>()
                            .connections
                            .get_mut(&event.id)
                        {
                            connection.status = status;
                            if !status.holds_claim() {
                                connection.revoke_bot();
                            }
                        }
                    });
                    continue;
                }
                WorkerPayload::Ready(bot) => {
                    let installed = cx.update(|cx| {
                        if !live_generation(&event.id, event.generation, cx) {
                            return false;
                        }
                        cx.global_mut::<RemoteChannels>()
                            .connections
                            .get_mut(&event.id)
                            .map(|connection| connection.publish_bot(bot))
                            .is_some()
                    });
                    if !installed {
                        continue;
                    }
                    cx.update(super::outbound::flush_edits);
                    backlog.take(&event.id, event.generation)
                }
                WorkerPayload::Incoming(incoming) => {
                    match backlog.admit(&event.id, event.generation, incoming) {
                        Some(incoming) => VecDeque::from([incoming]),
                        None => continue,
                    }
                }
            };
            while let Some(incoming) = ready.pop_front() {
                if let Served::Retry(incoming) =
                    serve(&event.id, event.generation, incoming, &claim, cx).await
                {
                    ready.push_front(incoming);
                    backlog.hold(&event.id, event.generation, ready);
                    break;
                }
            }
        }
    })
    .detach();
}

/// What serving one command came to.
enum Served {
    Done,
    /// Its claim must be retried on a reopened inbox.
    Retry(transport::Incoming),
}

/// Claim one command and carry it out.
async fn serve<F: Future<Output = ClaimResult>>(
    id: &str,
    generation: uuid::Uuid,
    incoming: transport::Incoming,
    claim: &impl Fn(&InboxStore, ChannelKey, InboxId) -> F,
    cx: &mut gpui::AsyncApp,
) -> Served {
    let bot = match claim_incoming(id, generation, &incoming.event_id, claim, cx).await {
        Claimed::Ready(bot) => bot,
        Claimed::Skip => return Served::Done,
        Claimed::Retry => return Served::Retry(incoming),
    };
    let pairing = cx.update(|cx| {
        if !live_session(id, generation, &bot, cx) {
            return None;
        }
        let connection = cx.global_mut::<RemoteChannels>().connections.get_mut(id)?;
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
            if !live_session(id, generation, &bot, cx) {
                return false;
            }
            let mut channels = SettingsStore::global(cx).user().remote.channels.clone();
            let Some(channel) = channels.iter_mut().find(|channel| channel.id == id) else {
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
                .spawn(async move { transport::send(kind, &bot.credentials, &recipient, message) })
                .await;
            if let Err(error) = result {
                remote_channel::log_error(
                    "Remote pairing confirmation failed",
                    &error,
                    "remote.pair.confirm",
                );
            }
        }
        return Served::Done;
    }
    // Every pairing attempt stops here, matched or not. A gateway
    // redelivery would otherwise reach an already-paired channel's
    // routing path and inject the code into an agent as a prompt.
    if let IncomingKind::Message { text, .. } = &incoming.kind
        && pair_code(text).is_some()
    {
        return Served::Done;
    }
    let effect = cx.update(|cx| dispatch_claimed(id, generation, &bot, &incoming, cx));
    let Some(effect) = effect else {
        return Served::Done;
    };
    let effect = effect.settled().await;
    let delivery = cx.update(|cx| {
        if !live_session(id, generation, &bot, cx) {
            return None;
        }
        let connection = cx.global::<RemoteChannels>().live(id, cx)?;
        if connection.generation != generation {
            return None;
        }
        let tap = match &incoming.kind {
            IncomingKind::Callback { message_id, .. } => {
                if connection.cards.borrow().untracked(message_id) {
                    Tap::Untracked
                } else {
                    Tap::Settled
                }
            }
            IncomingKind::Message { .. } => Tap::Absent,
        };
        Some((
            connection.config.kind,
            bot.clone(),
            connection.config.recipient.clone()?,
            tap,
        ))
    });
    let Some((kind, bot, recipient, tap)) = delivery else {
        return Served::Done;
    };
    let result = cx
        .background_executor()
        .spawn(async move {
            match feedback_for(effect, incoming.kind, tap) {
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
    Served::Done
}

/// What one handled effect owes the sender.
#[derive(Debug, PartialEq, Eq)]
enum Feedback {
    Nothing,
    Send(Message),
    /// Take the dead buttons off the tapped message and say why.
    Edit {
        message_id: String,
        original: String,
        label: String,
    },
}

/// Who rewrites the tapped message, if anyone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tap {
    /// A message, not a button.
    Absent,
    /// A settlement edit owns the message; a tap racing it must not replace
    /// the settled result with a competing label.
    Settled,
    /// No settlement will ever edit the message (a card from an earlier
    /// process, or one whose tokens were dropped), so its buttons are removed
    /// here or they stay tappable forever.
    Untracked,
}

/// Live cards settle through their shared ledger; feedback rewrites only a
/// tapped card the ledger does not own.
fn feedback_for(effect: Effect, kind: IncomingKind, tap: Tap) -> Feedback {
    let stale = |label: String| match (kind, tap) {
        (
            IncomingKind::Callback {
                message_id,
                original,
                ..
            },
            Tap::Untracked,
        ) => Feedback::Edit {
            message_id,
            original,
            label,
        },
        _ => Feedback::Send(Message::plain(label, None)),
    };
    match effect {
        Effect::Reply(reply) => Feedback::Send(Message::plain(reply.text, reply.keyboard)),
        Effect::Feedback {
            edit: Edit::ManagedCard,
            ..
        } => Feedback::Nothing,
        Effect::Feedback {
            label,
            edit: Edit::StaleCard,
        } => stale(label),
        Effect::Feedback {
            label,
            edit: Edit::KeepButtons,
        } => Feedback::Send(Message::plain(label, None)),
        Effect::None if tap != Tap::Absent => stale(s::remote_channel::stale_callback()),
        Effect::None => Feedback::Nothing,
        // The loop settles an effect before asking what it is owed; one that
        // is still waiting has nothing to send yet.
        Effect::Later(_) => Feedback::Nothing,
    }
}

#[cfg(test)]
mod tests;
