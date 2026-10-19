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

mod reclaim;
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

/// What the foreground asks of a running worker. The blocking transport
/// session and the retry waits watch `interrupt`; `stop` and `reopen` say why
/// it was raised. Every request stores its reason before the interrupt, and the
/// worker clears the interrupt before reading reasons, so no request is lost.
#[derive(Default)]
struct Signals {
    /// Shared with each `BotSession` the worker publishes, as its liveness.
    stop: Arc<AtomicBool>,
    reopen: AtomicBool,
    interrupt: AtomicBool,
}

pub struct Worker {
    signals: Arc<Signals>,
}

impl Worker {
    /// Retire the inbox writer and open a fresh one under a newly taken
    /// lease, publishing it as a new `Ready`. Done by the worker itself so the
    /// old writer is let go before the lease is asked for again.
    pub(crate) fn reopen_inbox(&self) {
        self.signals.reopen.store(true, Ordering::SeqCst);
        self.signals.interrupt.store(true, Ordering::SeqCst);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.signals.stop.store(true, Ordering::SeqCst);
        self.signals.interrupt.store(true, Ordering::SeqCst);
    }
}

pub fn start(
    config: ChannelConfig,
    generation: uuid::Uuid,
    tx: UnboundedSender<WorkerEvent>,
) -> std::io::Result<Worker> {
    let signals = Arc::new(Signals::default());
    let worker = signals.clone();
    std::thread::Builder::new()
        .name(format!("remote-{}", config.id))
        .spawn(move || {
            let stop = &signals.stop;
            let interrupt = &signals.interrupt;
            let emit = |payload| {
                !stop.load(Ordering::Relaxed)
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
            let mut retiring = None;
            let mut gateway = transport::discord::Gateway::default();
            loop {
                interrupt.store(false, Ordering::SeqCst);
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                if signals.reopen.swap(false, Ordering::SeqCst) {
                    reclaim::retire(&mut serving, &mut retiring);
                }
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
                        reclaim::retire(&mut serving, &mut retiring);
                        if !emit(WorkerPayload::Status(Status::MissingCredentials)) {
                            return;
                        }
                        wait(interrupt);
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
                    // A reused writer's old session is revoked when the new one
                    // replaces it below.
                    let inbox = match inbox {
                        Some(inbox) => inbox,
                        None => {
                            reclaim::retire(&mut serving, &mut retiring);
                            let Some(claim) =
                                reclaim::claim(&lock_root, &credentials.bot, &mut retiring, stop)
                            else {
                                return;
                            };
                            match claim {
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
                                            wait(interrupt);
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
                                    wait(interrupt);
                                    continue;
                                }
                            }
                        }
                    };
                    serving = Some(SessionOwner(Arc::new(BotSession::new(
                        credentials.clone(),
                        inbox,
                        stop.clone(),
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
                        transport::slack::session(&credentials, interrupt, publish, ready)
                    }
                    ChannelKind::Discord => {
                        gateway.session(&credentials, interrupt, publish, ready)
                    }
                };
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                // A requested reopen ended the session, not a connection fault;
                // the gateway keeps its resume state and reconnects at once.
                if signals.reopen.load(Ordering::SeqCst) {
                    continue;
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
                wait(interrupt);
            }
        })?;
    Ok(Worker { signals: worker })
}

fn wait(interrupted: &AtomicBool) {
    let until = std::time::Instant::now() + RETRY_DELAY;
    while !interrupted.load(Ordering::Relaxed) && std::time::Instant::now() < until {
        std::thread::sleep(CANCEL_CHECK);
    }
}

#[cfg(test)]
impl Worker {
    /// A handle with no thread behind it, for foreground tests that only
    /// observe what was asked of the worker.
    pub(crate) fn detached() -> Self {
        Self {
            signals: Arc::default(),
        }
    }

    pub(crate) fn reopen_requested(&self) -> bool {
        self.signals.reopen.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_worker_cancels_all_of_its_loops() {
        let worker = Worker::detached();
        let signals = worker.signals.clone();
        drop(worker);
        assert!(signals.stop.load(Ordering::SeqCst));
        assert!(signals.interrupt.load(Ordering::SeqCst));
    }

    #[test]
    fn a_reopen_interrupts_the_session_without_stopping_the_worker() {
        let worker = Worker::detached();
        worker.reopen_inbox();
        assert!(worker.signals.interrupt.load(Ordering::SeqCst));
        assert!(!worker.signals.stop.load(Ordering::SeqCst));
        assert!(worker.reopen_requested());
    }
}
