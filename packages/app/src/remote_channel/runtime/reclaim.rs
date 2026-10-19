//! Which writer this worker let go of, so taking its bot back waits out the
//! worker's own lease instead of reporting it as another daruda's.

use std::sync::atomic::{AtomicBool, Ordering};

use daruda_store::inbox::InboxReleased;

use super::super::lock;
use super::SessionOwner;

/// The writer this worker gave up, and the bot it served.
pub(super) struct Retiring {
    bot: String,
    released: InboxReleased,
}

/// Stop serving, remembering when the old writer's lease will really be free.
pub(super) fn retire(serving: &mut Option<SessionOwner>, retiring: &mut Option<Retiring>) {
    if let Some(owner) = serving.take() {
        *retiring = Some(Retiring {
            bot: owner.0.credentials.bot.clone(),
            released: owner.0.inbox.released(),
        });
    }
}

/// Claim `bot`, first waiting out this worker's own retired lease on it.
/// `None` when `stop` was raised during the wait.
pub(super) fn claim(
    lock_root: &std::path::Path,
    bot: &str,
    retiring: &mut Option<Retiring>,
    stop: &AtomicBool,
) -> Option<lock::Claim> {
    match retiring.take_if(|retiring| retiring.bot == bot) {
        Some(own) => lock::reclaim(lock_root, bot, &own.released, || {
            !stop.load(Ordering::Relaxed)
        }),
        None => Some(lock::claim(lock_root, bot)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_channel::runtime::BotSession;
    use crate::remote_channel::transport::Credentials;
    use daruda_store::inbox::{InboxSeed, InboxStore};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn serve(root: &std::path::Path, bot: &str) -> SessionOwner {
        let lock::Claim::Ours { _lock: lease } = crate::test_support::once_released(
            || lock::claim(root, bot),
            |claim| matches!(claim, lock::Claim::Theirs),
        ) else {
            panic!("a free bot is ours to take");
        };
        let inbox = InboxStore::open(lease, "test", || Ok(InboxSeed::default())).unwrap();
        SessionOwner(Arc::new(BotSession::new(
            Arc::new(Credentials {
                bot: bot.into(),
                app: None,
            }),
            Arc::new(inbox),
            Default::default(),
        )))
    }

    /// The defect this guards: reopening while an in-flight claim still held
    /// the old writer answered `Theirs`, and the worker reported its own bot
    /// as held by another daruda for a whole retry delay.
    #[test]
    fn a_reopen_waits_for_its_own_retired_writer_instead_of_calling_it_foreign() {
        let root = tempfile::tempdir().unwrap();
        let mut serving = Some(serve(root.path(), "bot"));
        let in_flight = serving.as_ref().unwrap().0.clone();
        let mut retiring = None;
        retire(&mut serving, &mut retiring);
        assert!(
            matches!(lock::claim(root.path(), "bot"), lock::Claim::Theirs),
            "a clone of the retired writer still holds the lease"
        );

        let releasing = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(in_flight);
        });
        let claim = claim(root.path(), "bot", &mut retiring, &AtomicBool::new(false));
        releasing.join().unwrap();
        assert!(matches!(claim, Some(lock::Claim::Ours { .. })));
        assert!(retiring.is_none());
    }

    /// Only this worker's own lease is waited out; another holder is still
    /// reported at once, and a retired writer for a different bot is no reason
    /// to wait.
    #[test]
    fn a_lease_this_worker_never_retired_is_still_someone_elses() {
        let root = tempfile::tempdir().unwrap();
        let _other = serve(root.path(), "bot");
        let mut serving = Some(serve(root.path(), "previous-bot"));
        let _in_flight = serving.as_ref().unwrap().0.clone();
        let mut retiring = None;
        retire(&mut serving, &mut retiring);

        let started = Instant::now();
        let claim = claim(root.path(), "bot", &mut retiring, &AtomicBool::new(false));
        assert!(matches!(claim, Some(lock::Claim::Theirs)));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(retiring.is_some(), "the other bot's release is still owed");
    }

    #[test]
    fn stopping_ends_the_wait_for_a_retired_writer() {
        let root = tempfile::tempdir().unwrap();
        let mut serving = Some(serve(root.path(), "bot"));
        let _in_flight = serving.as_ref().unwrap().0.clone();
        let mut retiring = None;
        retire(&mut serving, &mut retiring);
        assert!(claim(root.path(), "bot", &mut retiring, &AtomicBool::new(true)).is_none());
    }
}
