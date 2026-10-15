//! A Telegram bot owns its shared writer; migration belongs to the profile.

use std::path::Path;
use std::sync::Arc;

use daruda_store::inbox::{InboxStore, InboxStoreError, migrate};

use crate::remote_channel::lock::{self, Claim};
use crate::telegram::keychain::{self, TokenSnapshot};

pub(super) struct ReadyBot {
    pub token: TokenSnapshot,
    pub inbox: Arc<InboxStore>,
    pub generation: uuid::Uuid,
}

pub(super) enum Ownership {
    Idle,
    HeldElsewhere,
    Ready(Arc<ReadyBot>),
}

impl Ownership {
    pub(super) fn serves(&self) -> bool {
        matches!(self, Self::Ready(bot) if bot.token.revision == keychain::revision())
    }

    pub(super) fn ready(&self) -> Option<&Arc<ReadyBot>> {
        match self {
            Self::Ready(bot) => Some(bot),
            Self::Idle | Self::HeldElsewhere => None,
        }
    }
}

pub(super) fn open(
    lock_root: &Path,
    data: &Path,
    token: TokenSnapshot,
) -> Result<Ownership, InboxStoreError> {
    let digest = lock::digest_of(&token.token);
    let seed = migrate::telegram_for_bot(data, &digest, env!("CARGO_PKG_VERSION"))?;
    match lock::claim(lock_root, &token.token) {
        Claim::Ours { _lock: lease, .. } => {
            let inbox = InboxStore::open(lease, env!("CARGO_PKG_VERSION"), || Ok(seed))?;
            Ok(Ownership::Ready(Arc::new(ReadyBot {
                token,
                inbox: Arc::new(inbox),
                generation: uuid::Uuid::new_v4(),
            })))
        }
        Claim::Theirs => Ok(Ownership::HeldElsewhere),
        Claim::Unavailable => Ok(Ownership::Idle),
    }
}

#[cfg(test)]
mod tests;
