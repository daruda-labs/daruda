//! A ready bot retains its writer and revokes command permission separately.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use daruda_config::remote::ChannelKind;
use daruda_store::inbox::{ChannelKey, InboxStore};

use super::super::transport::Credentials;

pub(crate) struct BotSession {
    pub credentials: Arc<Credentials>,
    pub inbox: Arc<InboxStore>,
    cancelled: Arc<AtomicBool>,
    revoked: AtomicBool,
}

impl BotSession {
    pub(crate) fn new(
        credentials: Arc<Credentials>,
        inbox: Arc<InboxStore>,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            credentials,
            inbox,
            cancelled,
            revoked: AtomicBool::new(false),
        }
    }

    pub(crate) fn is_live(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire) && !self.revoked.load(Ordering::Acquire)
    }

    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
}

pub(super) struct SessionOwner(pub Arc<BotSession>);

impl Drop for SessionOwner {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

pub(crate) fn channel_key(kind: ChannelKind) -> ChannelKey {
    ChannelKey::new(match kind {
        ChannelKind::Slack => "slack",
        ChannelKind::Discord => "discord",
    })
}

#[cfg(test)]
mod tests {}
