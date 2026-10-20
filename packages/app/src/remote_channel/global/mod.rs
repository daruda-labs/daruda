//! Foreground routing state, with one cancellable gateway worker per connection.

use super::backoff::ErrorBackoff;
use super::bridge::{ApprovalPrompt, BridgePing, MessageTail, Outbound, PaneRef, RoutingCore};
use super::cards::OwnedCards;
use super::pairing::Pairing;
use super::runtime::{self, BotSession, Status, Worker, WorkerEvent};
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use daruda_config::remote::ChannelConfig;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, Global};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

mod inbound;
mod outbound;
pub(crate) use outbound::Pending;

pub struct Connection {
    pub config: ChannelConfig,
    pub generation: uuid::Uuid,
    pub core: RoutingCore<String>,
    cards: RefCell<OwnedCards<String, outbound::Receipt, String>>,
    pub status: Status,
    pub bot: Option<Arc<BotSession>>,
    pairing: Option<Pairing>,
    worker: WorkerState,
    /// Paces inbox reopens while command claims keep failing; reset by the
    /// next claim that commits.
    claim_backoff: ErrorBackoff,
}

enum WorkerState {
    Idle,
    Running { worker: Worker },
    UpdatingCredential { pending: usize },
}

pub struct RemoteChannels {
    pub(crate) connections: HashMap<String, Connection>,
    events: UnboundedSender<WorkerEvent>,
    outbound: UnboundedSender<Pending>,
    applied: Option<Vec<ChannelConfig>>,
}

impl Global for RemoteChannels {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Explicit,
    Presence { away: bool },
}

impl Connection {
    /// Whether a message may go out on this connection.
    ///
    /// Three conditions, in one place because they are asked from three: the
    /// foreground gate that decides whether to queue, the bridge-wide "can
    /// anything be delivered", and `prepare` at the last step before the wire.
    /// The third is what a ping queued while the worker was still connecting
    /// meets once the claim comes back held elsewhere.
    ///
    /// `credentials` doubles as the claim's own gate: the worker publishes
    /// them only after taking the bot, so a connection that has never held it
    /// cannot send on the strength of its starting status.
    pub(crate) fn can_send(&self) -> bool {
        self.bot.as_ref().is_some_and(|bot| bot.is_live())
            && self.config.recipient.is_some()
            && self.status.holds_claim()
    }

    /// Serve with the bot the worker published. The one place `bot` gains a
    /// session, so the card ledger is always bound to the identity that bot
    /// speaks as: the same bot keeps its cards across reconnects and inbox
    /// recovery, and another bot's arrival forgets the old cards' callback
    /// tokens with their records.
    pub(crate) fn publish_bot(&mut self, bot: Arc<BotSession>) {
        if self
            .bot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &bot))
        {
            return;
        }
        if let Some(previous) = self.bot.take() {
            previous.revoke();
        }
        let owner = super::lock::digest_of(&bot.credentials.bot);
        if self.cards.get_mut().owner() != Some(&owner) {
            self.core.clear_settled_callbacks();
        }
        for card in self.cards.get_mut().bind(owner) {
            self.core.forget_card(&card);
        }
        self.bot = Some(bot);
    }

    /// Withdraw the bot without forgetting its cards: the same bot may serve
    /// again, and its messages are still the ones on the person's screen.
    pub(super) fn revoke_bot(&mut self) {
        if let Some(bot) = self.bot.take() {
            bot.revoke();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(bot) = &self.bot {
            bot.revoke();
        }
    }
}

impl RemoteChannels {
    pub fn status(id: &str, cx: &App) -> Status {
        cx.try_global::<Self>()
            .and_then(|b| b.connections.get(id))
            .map_or(Status::Disabled, |c| c.status)
    }

    /// Both the connection generation and live config must still agree.
    pub(crate) fn live<'a>(&'a self, id: &str, cx: &App) -> Option<&'a Connection> {
        let connection = self.connections.get(id)?;
        let config = SettingsStore::global(cx)
            .user()
            .remote
            .channels
            .iter()
            .find(|c| c.id == id)?;
        (connection.config == *config && config.enabled).then_some(connection)
    }

    pub fn has_recipient(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|bridge| {
            bridge
                .connections
                .keys()
                .any(|id| bridge.live(id, cx).is_some_and(Connection::can_send))
        })
    }

    pub fn send_ping(ping: BridgePing, delivery: Delivery, cx: &App) -> bool {
        if !Self::has_recipient(cx) {
            return false;
        }
        let Some(bridge) = cx.try_global::<Self>() else {
            return false;
        };
        let away = match delivery {
            Delivery::Explicit => true,
            Delivery::Presence { away } => away,
        };
        let mut queued = false;
        for id in bridge.connections.keys() {
            let Some(connection) = bridge.live(id, cx) else {
                continue;
            };
            if !away && connection.config.only_when_away {
                continue;
            }
            queued |= bridge.enqueue(connection, Outbound::Ping(ping.clone()));
        }
        queued
    }

    /// Introduce a chat on every live connection. Solicited announcements
    /// also select it as the connection's target.
    pub fn announce_chat(
        pane: PaneRef,
        header: String,
        tail: String,
        delivery: Delivery,
        cx: &mut App,
    ) -> bool {
        if matches!(delivery, Delivery::Presence { .. }) {
            return Self::send_notice_with_delivery(format!("{header}\n{tail}"), delivery, cx);
        }
        if !cx.has_global::<Self>() {
            return false;
        }
        // Same bar as `send_approval`: a live but unpaired channel must not
        // be retargeted at a chat it was never told about.
        let ids: Vec<String> = {
            let bridge = cx.global::<Self>();
            bridge
                .connections
                .keys()
                .filter(|id| bridge.live(id, cx).is_some_and(Connection::can_send))
                .cloned()
                .collect()
        };
        let bridge = cx.global_mut::<Self>();
        for id in ids {
            if let Some(connection) = bridge.connections.get_mut(&id) {
                connection.core.command_state_mut().select(Some(pane));
            }
        }
        Self::send_ping(
            BridgePing {
                pane,
                header,
                tail: MessageTail::Plain(tail),
                permission: None,
            },
            delivery,
            cx,
        )
    }

    pub fn send_notice(text: String, cx: &App) {
        Self::send_notice_with_delivery(text, Delivery::Explicit, cx);
    }

    pub(crate) fn send_flow_ask(
        prompt: super::bridge::FlowAskPrompt,
        delivery: Delivery,
        cx: &App,
    ) -> bool {
        let Some(bridge) = cx.try_global::<Self>() else {
            return false;
        };
        let mut queued = false;
        for id in bridge.connections.keys() {
            if let Some(connection) = bridge.live(id, cx) {
                if matches!(delivery, Delivery::Presence { away: false })
                    && connection.config.only_when_away
                {
                    continue;
                }
                queued |= bridge.enqueue(connection, Outbound::FlowAsk(prompt.clone()));
            }
        }
        queued
    }

    /// Send a pane-less notice under the same explicit/presence policy pings
    /// use. Returns whether at least one live connection accepted it.
    pub fn send_notice_with_delivery(text: String, delivery: Delivery, cx: &App) -> bool {
        let Some(bridge) = cx.try_global::<Self>() else {
            return false;
        };
        let away = match delivery {
            Delivery::Explicit => true,
            Delivery::Presence { away } => away,
        };
        let mut queued = false;
        for id in bridge.connections.keys() {
            if let Some(connection) = bridge.live(id, cx) {
                if !away && connection.config.only_when_away {
                    continue;
                }
                queued |= bridge.enqueue(connection, Outbound::Notice(text.clone()));
            }
        }
        queued
    }

    fn enqueue(&self, connection: &Connection, outbound: Outbound) -> bool {
        if !connection.can_send() {
            return false;
        }
        let Some((bot, recipient)) = connection
            .bot
            .clone()
            .zip(connection.config.recipient.clone())
        else {
            return false;
        };
        let card = match outbound.card_id() {
            Some(id) => match connection.cards.borrow_mut().ledger_mut().claim(id) {
                Some(claim) => Some(claim),
                None => return false,
            },
            None => None,
        };
        let result = self.outbound.unbounded_send(Pending {
            id: connection.config.id.clone(),
            generation: connection.generation,
            bot,
            recipient,
            outbound,
            card: card.clone(),
        });
        if let Err(error) = &result {
            if let Some(claim) = card {
                connection.cards.borrow_mut().ledger_mut().release(&claim);
            }
            super::log_error(
                "Remote outgoing queue closed",
                error,
                "remote.queue.outbound",
            );
        }
        result.is_ok()
    }

    pub fn send_approval(
        id: crate::control::approval::ApprovalId,
        summary: String,
        delivery: Delivery,
        cx: &mut App,
    ) -> bool {
        let Some(bridge) = cx.try_global::<Self>() else {
            return false;
        };
        let ids: Vec<_> = bridge
            .connections
            .keys()
            .filter(|key| {
                bridge.live(key, cx).is_some_and(|connection| {
                    connection.can_send()
                        && (!matches!(delivery, Delivery::Presence { away: false })
                            || !connection.config.only_when_away)
                })
            })
            .cloned()
            .collect();
        let bridge = cx.global_mut::<Self>();
        let mut queued = false;
        for key in ids {
            let Some(connection) = bridge.connections.get_mut(&key) else {
                continue;
            };
            let Some((bot, recipient)) = connection
                .bot
                .clone()
                .zip(connection.config.recipient.clone())
            else {
                continue;
            };
            let Some(card) = connection
                .cards
                .get_mut()
                .ledger_mut()
                .claim(super::cards::CardId::Approval(id))
            else {
                continue;
            };
            let (allow, refuse) = connection.core.record_pending_approval(id);
            let pending = Pending {
                id: key.clone(),
                generation: connection.generation,
                bot,
                recipient,
                outbound: Outbound::Approval(ApprovalPrompt {
                    id,
                    summary: summary.clone(),
                    buttons: [
                        (s::control::approval_allow(), allow),
                        (s::control::approval_refuse(), refuse),
                    ],
                }),
                card: Some(card.clone()),
            };
            match bridge.outbound.unbounded_send(pending) {
                Ok(()) => queued = true,
                Err(error) => {
                    if let Some(connection) = bridge.connections.get_mut(&key) {
                        connection.core.forget_pending_approval(id);
                        connection.cards.get_mut().ledger_mut().release(&card);
                    }
                    super::log_error(
                        "Remote approval queue closed",
                        &error,
                        "remote.queue.approval",
                    );
                }
            }
        }
        queued
    }

    pub fn generate_pair_code(id: &str, cx: &mut App) -> Option<String> {
        reconcile(cx);
        let connection = cx.global_mut::<Self>().connections.get_mut(id)?;
        if !connection.config.enabled || connection.config.recipient.is_some() {
            return None;
        }
        let pairing = Pairing::new();
        let code = pairing.code().to_owned();
        connection.pairing = Some(pairing);
        Some(code)
    }

    pub fn restart(id: &str, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let bridge = cx.global_mut::<Self>();
        if let Some(connection) = bridge.connections.get_mut(id)
            && let WorkerState::UpdatingCredential { pending } = &mut connection.worker
            && *pending > 1
        {
            *pending -= 1;
            return;
        }
        // A credential rewrite usually keeps the bot, and its cards are still on
        // the person's screen: carry their records and callback tokens to the
        // replacement. `publish_bot` drops them if a different bot turns up.
        let carried = bridge.connections.remove(id).map(|mut old| {
            (
                old.config.kind,
                old.config.recipient.clone(),
                std::mem::take(&mut old.core),
                std::mem::take(&mut old.cards),
            )
        });
        bridge.applied = None;
        reconcile(cx);
        if let Some((kind, recipient, core, cards)) = carried
            && let Some(connection) = cx.global_mut::<Self>().connections.get_mut(id)
            && connection.config.kind == kind
            && connection.config.recipient == recipient
        {
            connection.core = core;
            connection.cards = cards;
        }
    }

    /// Take a bot whose inbox writer failed out of service, and after this
    /// connection's claim backoff have its own worker open a fresh writer.
    ///
    /// The bot is revoked at once so nothing still holding it can act. The
    /// generation stays: the same worker keeps serving, and the commands that
    /// failed to claim wait in the inbound backlog for its next `Ready`. A
    /// second worker would race the first for the lease the first still holds.
    fn recover_inbox(id: &str, generation: uuid::Uuid, bot: &Arc<BotSession>, cx: &mut App) {
        bot.revoke();
        let bridge = cx.global_mut::<Self>();
        let Some(connection) = bridge.connections.get_mut(id) else {
            return;
        };
        if connection.generation != generation
            || !connection
                .bot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, bot))
        {
            return;
        }
        connection.bot = None;
        connection.status = Status::Failed;
        let delay = connection.claim_backoff.delay();
        let id = id.to_owned();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            cx.update(|cx| Self::reopen_inbox(&id, generation, cx));
        })
        .detach();
    }

    /// Ask the worker serving `generation` to reopen its inbox. A replaced or
    /// suspended connection has a newer generation and its own writer.
    fn reopen_inbox(id: &str, generation: uuid::Uuid, cx: &mut App) {
        let Some(connection) = cx
            .try_global::<Self>()
            .and_then(|bridge| bridge.connections.get(id))
        else {
            return;
        };
        if connection.generation != generation {
            return;
        }
        match &connection.worker {
            WorkerState::Running { worker } => worker.reopen_inbox(),
            // Only a running worker publishes a bot, so an idle connection
            // has no writer that could have failed.
            WorkerState::Idle | WorkerState::UpdatingCredential { .. } => {}
        }
    }

    pub(crate) fn suspend(config: ChannelConfig, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let bridge = cx.global_mut::<Self>();
        let connection = bridge
            .connections
            .entry(config.id.clone())
            .or_insert_with(|| Connection {
                config: config.clone(),
                generation: uuid::Uuid::new_v4(),
                core: RoutingCore::default(),
                cards: RefCell::default(),
                status: Status::Connecting,
                bot: None,
                pairing: None,
                worker: WorkerState::Idle,
                claim_backoff: ErrorBackoff::new(),
            });
        connection.generation = uuid::Uuid::new_v4();
        connection.config = config;
        connection.revoke_bot();
        let pending = match connection.worker {
            WorkerState::UpdatingCredential { pending } => pending + 1,
            WorkerState::Idle | WorkerState::Running { .. } => 1,
        };
        connection.worker = WorkerState::UpdatingCredential { pending };
        connection.status = Status::Connecting;
    }
}

pub fn install(cx: &mut App) {
    if cx.has_global::<RemoteChannels>() {
        return;
    }
    let (events, incoming) = unbounded();
    let (outbound, outgoing) = unbounded();
    cx.set_global(RemoteChannels {
        connections: HashMap::new(),
        events,
        outbound,
        applied: None,
    });
    inbound::spawn(incoming, cx);
    outbound::spawn(outgoing, cx);
    reconcile(cx);
    // Config reaches us by subscription rather than a timer: `reconcile`
    // returns immediately when the channel list is unchanged, so firing on an
    // unrelated settings write costs one `Vec` comparison.
    cx.observe_global::<SettingsStore>(reconcile).detach();
}

pub(crate) fn reconcile(cx: &mut App) {
    let config = SettingsStore::global(cx).user_arc();
    let bridge = cx.global_mut::<RemoteChannels>();
    if bridge.applied.as_ref() == Some(&config.remote.channels) {
        return;
    }
    bridge.applied = Some(config.remote.channels.clone());
    if let Err(error) = config.remote.validate() {
        bridge.connections.retain(|_, connection| {
            matches!(connection.worker, WorkerState::UpdatingCredential { .. })
        });
        super::log_error(
            "Invalid remote channel configuration",
            &error,
            "remote.config.invalid",
        );
        return;
    }
    bridge.connections.retain(|id, connection| {
        matches!(connection.worker, WorkerState::UpdatingCredential { .. })
            || config.remote.channels.iter().any(|c| c.id == *id)
    });
    for config in &config.remote.channels {
        if let Some(connection) = bridge.connections.get_mut(&config.id)
            && matches!(connection.worker, WorkerState::UpdatingCredential { .. })
        {
            connection.config = config.clone();
            continue;
        }
        if let Some(connection) = bridge.connections.get_mut(&config.id)
            && connection.config.kind == config.kind
            && connection.config.enabled == config.enabled
            && connection.config.recipient == config.recipient
        {
            connection.config = config.clone();
            continue;
        }
        if bridge
            .connections
            .get(&config.id)
            .is_some_and(|c| c.config == *config)
        {
            continue;
        }
        bridge.connections.remove(&config.id);
        let generation = uuid::Uuid::new_v4();
        let worker = start_worker(config, generation, &bridge.events);
        let status = if config.enabled {
            Status::Connecting
        } else {
            Status::Disabled
        };
        bridge.connections.insert(
            config.id.clone(),
            Connection {
                config: config.clone(),
                generation,
                core: RoutingCore::default(),
                cards: RefCell::default(),
                status,
                bot: None,
                pairing: None,
                worker,
                claim_backoff: ErrorBackoff::new(),
            },
        );
    }
}

fn start_worker(
    config: &ChannelConfig,
    generation: uuid::Uuid,
    events: &UnboundedSender<WorkerEvent>,
) -> WorkerState {
    if !config.enabled || cfg!(test) {
        return WorkerState::Idle;
    }
    match runtime::start(config.clone(), generation, events.clone()) {
        Ok(worker) => WorkerState::Running { worker },
        Err(error) => {
            super::log_error(
                "Remote gateway worker could not start",
                &error,
                "remote.worker.start",
            );
            WorkerState::Idle
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
