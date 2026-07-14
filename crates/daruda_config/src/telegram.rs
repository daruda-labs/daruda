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
    /// Hold agent pings (completion / permission / post-turn) instead of
    /// pushing them to Telegram while the user is actively at the daruda
    /// window; deferred pings are delivered once presence drops.
    pub defer_while_active: bool,
    /// While the app is foreground, how many seconds of no system input mark
    /// the user "away" so held pings flush.
    pub active_idle_secs: u64,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            authorized_chat_id: None,
            defer_while_active: true,
            active_idle_secs: 60,
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
    fn defaults_defer_while_active_on_with_60s_idle() {
        let cfg = TelegramConfig::default();
        assert!(cfg.defer_while_active);
        assert_eq!(cfg.active_idle_secs, 60);
    }

    #[test]
    fn toml_round_trip_unspecified_fields_use_defaults() {
        let toml_src = "enabled = true\n";
        let cfg: TelegramConfig = toml::from_str(toml_src).unwrap();
        assert!(cfg.enabled);
        // Unspecified fields fall back to their defaults via `#[serde(default)]`.
        assert_eq!(cfg.authorized_chat_id, None);
        assert!(cfg.defer_while_active);
        assert_eq!(cfg.active_idle_secs, 60);
    }
}
