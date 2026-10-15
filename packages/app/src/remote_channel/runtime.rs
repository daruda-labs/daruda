//! Background gateway ownership. Dropping a worker revokes its event generation.

use super::{
    keychain, lock,
    transport::{self, Credentials, Incoming},
};
use daruda_config::remote::{ChannelConfig, ChannelKind};
use futures::channel::mpsc::UnboundedSender;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

mod session;
use session::SessionOwner;
pub(crate) use session::{BotSession, channel_key};

const RETRY_DELAY: Duration = Duration::from_secs(30);
const CANCEL_CHECK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Disabled,
    MissingCredentials,
    Connecting,
    Connected,
    Retrying,
    Failed,
    /// Another daruda holds this bot's claim and is the one serving it. Not a
    /// failure: the user's phone still works, through the other instance.
    HeldElsewhere,
}

impl Status {
    /// Whether reaching this state required the bot's claim, and still implies
    /// holding it. Asked as a positive fact rather than "not `HeldElsewhere`":
    /// a worker that gave up, or never started, holds nothing either, and an
    /// exception list would have to remember every future state.
    ///
    /// The worker emits `Connecting` only just after taking the claim and does
    /// not let go until its retry wait is over, so these three — and only
    /// these — are states in which this process is serving the bot.
    pub(crate) fn holds_claim(self) -> bool {
        matches!(self, Self::Connecting | Self::Connected | Self::Retrying)
    }
}

pub struct WorkerEvent {
    pub id: String,
    pub generation: uuid::Uuid,
    pub payload: WorkerPayload,
}

pub enum WorkerPayload {
    Ready(Arc<BotSession>),
    Status(Status),
    Incoming(Incoming),
}

pub struct Worker {
    stop: Arc<AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(
    config: ChannelConfig,
    generation: uuid::Uuid,
    tx: UnboundedSender<WorkerEvent>,
) -> std::io::Result<Worker> {
    let stop = Arc::new(AtomicBool::new(false));
    let cancelled = stop.clone();
    std::thread::Builder::new()
        .name(format!("remote-{}", config.id))
        .spawn(move || {
            let emit = |payload| {
                !cancelled.load(Ordering::Relaxed)
                    && tx
                        .unbounded_send(WorkerEvent {
                            id: config.id.clone(),
                            generation,
                            payload,
                        })
                        .is_ok()
            };
            let lock_root = daruda_store::persistence::remote_lock_root();
            let data = daruda_store::persistence::default_data_dir();
            let mut serving: Option<SessionOwner> = None;
            let mut gateway = transport::discord::Gateway::default();
            while !cancelled.load(Ordering::Relaxed) {
                let service = keychain::channel_service(&config.id);
                let bot = keychain::read(&service, "bot_token");
                let app = (config.kind == ChannelKind::Slack)
                    .then(|| keychain::read(&service, "app_token"))
                    .flatten();
                let credentials = match bot {
                    Some(bot) if config.kind != ChannelKind::Slack || app.is_some() => {
                        Arc::new(Credentials { bot, app })
                    }
                    _ => {
                        serving = None;
                        if !emit(WorkerPayload::Status(Status::MissingCredentials)) {
                            return;
                        }
                        wait(&cancelled);
                        continue;
                    }
                };
                let unchanged = serving.as_ref().is_some_and(|owner| {
                    owner.0.is_live()
                        && owner.0.credentials.bot == credentials.bot
                        && owner.0.credentials.app == credentials.app
                });
                if !unchanged {
                    let inbox = serving
                        .as_ref()
                        .filter(|owner| owner.0.credentials.bot == credentials.bot)
                        .map(|owner| owner.0.inbox.clone());
                    serving = None;
                    let inbox = match inbox {
                        Some(inbox) => inbox,
                        None => match lock::claim(&lock_root, &credentials.bot) {
                            lock::Claim::Ours { _lock: lease, .. } => {
                                let seed = || {
                                    daruda_store::inbox::migrate::remote(
                                        &data,
                                        &config.id,
                                        channel_key(config.kind),
                                    )
                                };
                                match daruda_store::inbox::InboxStore::open(
                                    lease,
                                    env!("CARGO_PKG_VERSION"),
                                    seed,
                                ) {
                                    Ok(inbox) => Arc::new(inbox),
                                    Err(error) => {
                                        super::log_error(
                                            "Remote inbox could not open",
                                            &error,
                                            "remote.inbox.open",
                                        );
                                        if !emit(WorkerPayload::Status(Status::Failed)) {
                                            return;
                                        }
                                        wait(&cancelled);
                                        continue;
                                    }
                                }
                            }
                            claim => {
                                let status = if matches!(claim, lock::Claim::Theirs) {
                                    Status::HeldElsewhere
                                } else {
                                    Status::Failed
                                };
                                if !emit(WorkerPayload::Status(status)) {
                                    return;
                                }
                                wait(&cancelled);
                                continue;
                            }
                        },
                    };
                    serving = Some(SessionOwner(Arc::new(BotSession::new(
                        credentials.clone(),
                        inbox,
                        cancelled.clone(),
                    ))));
                }
                let session = serving
                    .as_ref()
                    .expect("a ready worker owns its bot")
                    .0
                    .clone();
                if !emit(WorkerPayload::Ready(session))
                    || !emit(WorkerPayload::Status(Status::Connecting))
                {
                    return;
                }
                let publish = |incoming: Incoming| {
                    // Paired connections do not enqueue traffic from other channel members.
                    if config.recipient.is_some() && !config.accepts(&incoming.sender) {
                        return true;
                    }
                    emit(WorkerPayload::Incoming(incoming))
                };
                let ready = || {
                    emit(WorkerPayload::Status(Status::Connected));
                };
                let result = match config.kind {
                    ChannelKind::Slack => {
                        transport::slack::session(&credentials, &cancelled, publish, ready)
                    }
                    ChannelKind::Discord => {
                        gateway.session(&credentials, &cancelled, publish, ready)
                    }
                };
                if cancelled.load(Ordering::Relaxed) {
                    return;
                }
                if let Err(error) = result {
                    if matches!(
                        error,
                        transport::TransportError::Closed(1000 | 1001 | 4007 | 4009)
                    ) {
                        gateway.reset();
                    }
                    super::log_error(
                        "Remote gateway connection interrupted",
                        &error,
                        "remote.gateway.interrupted",
                    );
                    if matches!(
                        error,
                        transport::TransportError::Closed(4004 | 4010 | 4011 | 4012 | 4013 | 4014)
                    ) {
                        emit(WorkerPayload::Status(Status::Failed));
                        return;
                    }
                }
                if !emit(WorkerPayload::Status(Status::Retrying)) {
                    return;
                }
                wait(&cancelled);
            }
        })?;
    Ok(Worker { stop })
}

fn wait(cancelled: &AtomicBool) {
    let until = std::time::Instant::now() + RETRY_DELAY;
    while !cancelled.load(Ordering::Relaxed) && std::time::Instant::now() < until {
        std::thread::sleep(CANCEL_CHECK);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dropping_worker_cancels_all_of_its_loops() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        drop(super::Worker { stop: stop.clone() });
        assert!(stop.load(std::sync::atomic::Ordering::Relaxed));
    }
}
