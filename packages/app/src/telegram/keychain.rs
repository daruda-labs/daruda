//! Telegram's legacy credential key, preserved across channel migration.

use crate::remote_channel::keychain;
use std::sync::atomic::{AtomicU64, Ordering};

const SERVICE: &str = "daruda-telegram-bot";
const ACCOUNT: &str = "token";
static REVISION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub(crate) struct TokenSnapshot {
    pub token: String,
    pub revision: u64,
}

pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

fn coherent_snapshot(
    mut read: impl FnMut() -> Option<String>,
    version: impl Fn() -> u64,
) -> Option<TokenSnapshot> {
    loop {
        let before = version();
        let token = read();
        let revision = version();
        if before == revision {
            return token.map(|token| TokenSnapshot { token, revision });
        }
    }
}

pub(crate) fn snapshot() -> Option<TokenSnapshot> {
    coherent_snapshot(read_token, revision)
}

pub fn read_token() -> Option<String> {
    keychain::read(&keychain::service(SERVICE), ACCOUNT)
}

pub fn write_token(token: &str) -> std::io::Result<()> {
    keychain::write(&keychain::service(SERVICE), ACCOUNT, token)?;
    REVISION.fetch_add(1, Ordering::Release);
    Ok(())
}

pub fn delete_token() -> std::io::Result<()> {
    keychain::delete(&keychain::service(SERVICE), ACCOUNT)?;
    REVISION.fetch_add(1, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_token_reads_are_hermetic() {
        assert!(super::read_token().is_none());
    }

    #[test]
    fn snapshots_retry_credential_aba_and_failed_writes_do_not_advance() {
        let version = AtomicU64::new(0);
        let mut reads = 0;
        let snapshot = coherent_snapshot(
            || {
                reads += 1;
                if reads == 1 {
                    version.fetch_add(2, Ordering::Release);
                }
                Some("bot-a".into())
            },
            || version.load(Ordering::Acquire),
        )
        .unwrap();
        assert_eq!(reads, 2);
        assert_eq!(snapshot.token, "bot-a");
        assert_eq!(snapshot.revision, 2);
        let before = revision();
        assert!(write_token("not-written").is_err());
        assert!(delete_token().is_err());
        assert_eq!(revision(), before);
    }
}
