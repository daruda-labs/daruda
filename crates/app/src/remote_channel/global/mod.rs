//! Foreground routing state, with one cancellable gateway worker per connection.

use super::bridge::{ApprovalPrompt, BridgePing, Outbound, RoutingCore};
use super::pairing::Pairing;
use super::runtime::{self, Status, Worker, WorkerEvent};
use super::transport::Credentials;
use crate::settings_store::SettingsStore;
use crate::surface::strings as s;
use daruda_config::remote::ChannelConfig;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, Global};
use std::collections::HashMap;
use std::sync::Arc;

mod inbound;
mod outbound;
pub(crate) use outbound::Pending;

pub struct Connection {
    pub config: ChannelConfig,
    pub generation: uuid::Uuid,
    pub core: RoutingCore<String>,
    pub status: Status,
    pub credentials: Option<Arc<Credentials>>,
    pairing: Option<Pairing>,
    _worker: Option<Worker>,
}

pub struct RemoteChannels {
    pub(crate) connections: HashMap<String, Connection>,
    events: UnboundedSender<WorkerEvent>,
    outbound: UnboundedSender<Pending>,
    state: daruda_store::remote_channels::RemoteChannelState,
    applied: Option<Vec<ChannelConfig>>,
}

impl Global for RemoteChannels {}

pub enum Delivery {
    Explicit,
    Presence { away: bool },
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
            bridge.connections.keys().any(|id| {
                bridge
                    .live(id, cx)
                    .is_some_and(|c| c.config.recipient.is_some() && c.credentials.is_some())
            })
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

    pub fn send_notice(text: String, cx: &App) {
        let Some(bridge) = cx.try_global::<Self>() else {
            return;
        };
        for id in bridge.connections.keys() {
            if let Some(connection) = bridge.live(id, cx) {
                bridge.enqueue(connection, Outbound::Notice(text.clone()));
            }
        }
    }

    fn enqueue(&self, connection: &Connection, outbound: Outbound) -> bool {
        if connection.credentials.is_none() || connection.config.recipient.is_none() {
            return false;
        }
        let result = self.outbound.unbounded_send(Pending {
            id: connection.config.id.clone(),
            generation: connection.generation,
            outbound,
        });
        if let Err(error) = &result {
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
        cx: &mut App,
    ) -> bool {
        let Some(bridge) = cx.try_global::<Self>() else {
            return false;
        };
        let ids: Vec<_> = bridge
            .connections
            .keys()
            .filter(|key| {
                bridge
                    .live(key, cx)
                    .is_some_and(|c| c.config.recipient.is_some() && c.credentials.is_some())
            })
            .cloned()
            .collect();
        let bridge = cx.global_mut::<Self>();
        let mut queued = false;
        for key in ids {
            let Some(connection) = bridge.connections.get_mut(&key) else {
                continue;
            };
            let (allow, refuse) = connection.core.record_pending_approval(id);
            let pending = Pending {
                id: key,
                generation: connection.generation,
                outbound: Outbound::Approval(ApprovalPrompt {
                    summary: summary.clone(),
                    buttons: [
                        (s::control_approval_allow(), allow),
                        (s::control_approval_refuse(), refuse),
                    ],
                }),
            };
            match bridge.outbound.unbounded_send(pending) {
                Ok(()) => queued = true,
                Err(error) => super::log_error(
                    "Remote approval queue closed",
                    &error,
                    "remote.queue.approval",
                ),
            }
        }
        queued
    }

    pub fn forget_approval(id: crate::control::approval::ApprovalId, cx: &mut App) {
        if cx.has_global::<Self>() {
            for connection in cx.global_mut::<Self>().connections.values_mut() {
                connection.core.forget_pending_approval(id);
            }
        }
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
        bridge.connections.remove(id);
        bridge.applied = None;
        reconcile(cx);
    }
}

pub fn install(cx: &mut App) {
    if cx.has_global::<RemoteChannels>() {
        return;
    }
    let (events, incoming) = unbounded();
    let (outbound, outgoing) = unbounded();
    let state = if cfg!(test) {
        Default::default()
    } else {
        daruda_store::remote_channels::RemoteChannelState::load_in(
            &daruda_store::persistence::default_data_dir(),
        )
    };
    cx.set_global(RemoteChannels {
        connections: HashMap::new(),
        events,
        outbound,
        state,
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
        bridge.connections.clear();
        super::log_error(
            "Invalid remote channel configuration",
            &error,
            "remote.config.invalid",
        );
        return;
    }
    bridge
        .connections
        .retain(|id, _| config.remote.channels.iter().any(|c| c.id == *id));
    bridge.state.retain_channels(
        &config
            .remote
            .channels
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>(),
    );
    for config in &config.remote.channels {
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
        let worker = if config.enabled && !cfg!(test) {
            match runtime::start(config.clone(), generation, bridge.events.clone()) {
                Ok(worker) => Some(worker),
                Err(error) => {
                    super::log_error(
                        "Remote gateway worker could not start",
                        &error,
                        "remote.worker.start",
                    );
                    None
                }
            }
        } else {
            None
        };
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
                status,
                credentials: None,
                pairing: None,
                _worker: worker,
            },
        );
    }
}

#[cfg(test)]
mod tests;
