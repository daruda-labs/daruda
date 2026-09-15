//! Background gateway ownership. Dropping a worker revokes its event generation.

use super::{
    keychain,
    transport::{self, Credentials, Incoming},
};
use daruda_config::remote::{ChannelConfig, ChannelKind};
use futures::channel::mpsc::UnboundedSender;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

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
}

pub struct WorkerEvent {
    pub id: String,
    pub generation: uuid::Uuid,
    pub payload: WorkerPayload,
}

pub enum WorkerPayload {
    Credentials(Arc<Credentials>),
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
                        if !emit(WorkerPayload::Status(Status::MissingCredentials)) {
                            return;
                        }
                        wait(&cancelled);
                        continue;
                    }
                };
                if !emit(WorkerPayload::Credentials(credentials.clone()))
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
