//! One daruda at a time per set of bot credentials.
//!
//! Telegram hands `getUpdates` to whichever poller asked last and answers the
//! rest with a 409; Slack picks one of an app's open Socket Mode connections
//! per event and documents no rule for which. Either way a reply can land in a
//! daruda that never sent the message it answers — and since the routing table
//! that would name the target is in memory, that instance has nothing to route
//! by and falls back to whatever chat it happens to be showing. The user
//! watches an answer start a turn on the wrong agent, with nothing said.
//!
//! So the claim is taken on the credential rather than the profile: a debug
//! build and an installed one are different profiles and the same bot, which
//! is exactly the pair that collides. Same reasoning as
//! [`flow_lock_root`](daruda_store::persistence::flow_lock_root) — a mutex on
//! something outside daruda that every profile shares.

use std::path::Path;

/// Hex characters of the credential digest kept for the file name. Half a
/// SHA-256 — long enough that two bots cannot collide in one user's config
/// directory, short enough to read in a directory listing.
const DIGEST_CHARS: usize = 32;

/// What asking for a credential's claim answered.
pub(crate) enum Claim {
    /// Ours, until this is dropped. The lock itself is a drop guard — nothing
    /// reads it, the OS releases it — so it is named like every other such
    /// field in this crate.
    Ours {
        _lock: daruda_store::inbox::InboxLease,
    },
    /// Another daruda holds it.
    Theirs,
    /// The lock itself could not be taken.
    Unavailable,
}

/// Ask for the claim on `credential`, taking it when it is free.
///
/// [`Claim::Unavailable`] is deliberately distinct from [`Claim::Theirs`]: it
/// means ownership could not be established. Neither state may open a writer
/// or execute inbound commands; the unavailable case also reports its error.
pub(crate) fn claim(dir: &Path, credential: &str) -> Claim {
    let digest = digest_of(credential);
    match daruda_store::inbox::InboxLease::try_acquire(dir, &digest) {
        Ok(Some(lease)) => Claim::Ours { _lock: lease },
        Ok(None) => Claim::Theirs,
        Err(error) => {
            unavailable(&error, "remote.lock.acquire");
            Claim::Unavailable
        }
    }
}

/// Report a bot lock this machine could not take at all, so the one state in
/// which two darudas can still collide is in the log rather than inferred.
#[track_caller]
fn unavailable(error: &std::io::Error, dedup: &str) {
    crate::remote_channel::log_error(
        "Bot lock unavailable: remote commands are paused without ownership proof",
        error,
        dedup,
    );
}

/// What one credential is known by here. A SHA-256 prefix, never the
/// credential: this goes into a file name, in a directory shared by every
/// profile, that outlives the session that wrote it.
pub(crate) fn digest_of(credential: &str) -> String {
    use sha2::{Digest as _, Sha256};

    let digest = Sha256::digest(credential.as_bytes());
    digest
        .iter()
        .take(DIGEST_CHARS / 2)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: a second daruda started against the same bot is told
    /// so, instead of quietly opening a second connection Slack and Telegram
    /// will then split the user's replies across.
    #[test]
    fn a_bot_one_daruda_holds_is_refused_to_the_next_one() {
        let dir = tempfile::tempdir().expect("tempdir");

        let held = claim(dir.path(), "bot-token");
        assert!(
            matches!(held, Claim::Ours { .. }),
            "the first daruda to ask must get the bot"
        );
        assert!(
            matches!(claim(dir.path(), "bot-token"), Claim::Theirs),
            "the second must hear that someone else has it"
        );

        // And it comes free again: the user quits the build that held it.
        drop(held);
        let again = crate::test_support::once_released(
            || claim(dir.path(), "bot-token"),
            |c| matches!(c, Claim::Theirs),
        );
        assert!(
            matches!(again, Claim::Ours { .. }),
            "a released bot is the next asker's to take"
        );
    }

    /// Two bots are two conversations. Excluding each other would take the
    /// second channel down with the first for no reason.
    #[test]
    fn two_different_bots_do_not_exclude_each_other() {
        let dir = tempfile::tempdir().expect("tempdir");

        let _first = claim(dir.path(), "bot-one");
        assert!(matches!(claim(dir.path(), "bot-two"), Claim::Ours { .. }));
    }

    /// The lock root is shared by every profile and outlives the session, so
    /// the one thing it must never do is write the token into a file name.
    #[test]
    fn the_lock_file_never_carries_the_token_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        let token = "1234567:AAHs3cr3t-do-not-write-me-down";

        let _held = claim(dir.path(), token);

        for entry in std::fs::read_dir(dir.path()).expect("read lock dir") {
            let name = entry.expect("entry").file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.contains(token),
                "the token is in the file name: {name}"
            );
            assert!(
                !name.contains("AAHs3cr3t"),
                "part of the token is in the file name: {name}"
            );
        }
    }
}
