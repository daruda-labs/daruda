//! Remote commands enter their durable inbox before any application effect.

use daruda_store::inbox::{
    ChannelKey, CommitError, Committed, InboxId, InboxMachine, InboxRejected, InboxStore,
};

use super::RunsHost;

impl RunsHost {
    pub(crate) fn claim_inbound(
        inbox: &InboxStore,
        channel: ChannelKey,
        id: InboxId,
    ) -> impl Future<Output = Result<Committed<InboxMachine>, CommitError<InboxRejected>>> + use<>
    {
        inbox.claim(channel, id)
    }
}

#[cfg(test)]
mod tests {}
