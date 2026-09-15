use serde::{Deserialize, Serialize};

/// Telegram bot bridge settings. The bot token itself is a secret and
/// never lives here — it is stored in the macOS Keychain (see
/// `daruda`'s `telegram::keychain` module). This struct only holds
/// non-secret configuration: whether the bridge is active, and the
/// chat id captured during pairing.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TelegramConfig {
    /// Master switch for the Telegram bridge. Defaults to false so the
    /// bridge stays fully inert (no polling, no keychain reads) until
    /// the user opts in via Settings.
    pub enabled: bool,
    /// Chat id authorized to receive pings and send replies, captured
    /// during pairing. Not a secret — Telegram chat ids are opaque
    /// numeric identifiers, not credentials.
    pub authorized_chat_id: Option<i64>,
    /// Treat Telegram as the stand-in for being away: an agent ping goes to
    /// the phone only when [`crate::PresenceConfig`] says the user is absent,
    /// and is dropped otherwise (the desktop notification already covered the
    /// present case). False sends every ping regardless of presence.
    ///
    /// The decision is made once, when the ping fires, and never revisited —
    /// nothing is queued, so a ping's send time is its settle time.
    pub only_when_away: bool,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            authorized_chat_id: None,
            only_when_away: true,
        }
    }
}

/// Migrate before defaults erase whether a new key was explicitly stated.
/// `active_idle_secs` contributes only its idle threshold; its delivery
/// quiet window has no successor.
pub(crate) fn migrate_legacy_timing(document: &mut toml::Table) {
    for (legacy, section, key) in [
        ("defer_while_active", "telegram", "only_when_away"),
        ("active_idle_secs", "presence", "away_idle_secs"),
        ("away_grace_secs", "presence", "away_grace_secs"),
    ] {
        let value = document
            .get_mut("telegram")
            .and_then(toml::Value::as_table_mut)
            .and_then(|telegram| telegram.remove(legacy));
        if let Some(value) = value
            && let Some(target) = document
                .entry(section)
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
        {
            target.entry(key).or_insert(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_inert() {
        let cfg = TelegramConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.authorized_chat_id, None);
    }

    #[test]
    fn toml_round_trip_preserves_explicit_false() {
        let toml_src = "\
enabled = false
authorized_chat_id = 123456789
";
        let cfg: TelegramConfig = toml::from_str(toml_src).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.authorized_chat_id, Some(123_456_789));

        let serialized = toml::to_string(&cfg).unwrap();
        let reparsed: TelegramConfig = toml::from_str(&serialized).unwrap();
        assert!(!reparsed.enabled);
        assert_eq!(reparsed.authorized_chat_id, Some(123_456_789));
    }

    #[test]
    fn defaults_restrict_the_phone_to_absence() {
        assert!(TelegramConfig::default().only_when_away);
    }

    #[test]
    fn toml_round_trip_unspecified_fields_use_defaults() {
        let toml_src = "enabled = true\n";
        let cfg: TelegramConfig = toml::from_str(toml_src).unwrap();
        assert!(cfg.enabled);
        // Unspecified fields fall back to their defaults via `#[serde(default)]`.
        assert_eq!(cfg.authorized_chat_id, None);
        assert!(cfg.only_when_away);
    }
}
