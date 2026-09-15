//! Telegram's legacy credential key, preserved across channel migration.

use crate::remote_channel::keychain;
const SERVICE: &str = "daruda-telegram-bot";
const ACCOUNT: &str = "token";

pub fn read_token() -> Option<String> {
    keychain::read(&keychain::service(SERVICE), ACCOUNT)
}

pub fn write_token(token: &str) -> std::io::Result<()> {
    keychain::write(&keychain::service(SERVICE), ACCOUNT, token)
}

pub fn delete_token() -> std::io::Result<()> {
    keychain::delete(&keychain::service(SERVICE), ACCOUNT)
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_token_reads_are_hermetic() {
        assert!(super::read_token().is_none());
    }
}
