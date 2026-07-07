use serde::{Deserialize, Serialize};

/// User attention + notification surfaces driven by terminal escape
/// sequences. Each channel is independently gated so a noisy shell or
/// untrusted process can be silenced without disabling the rest.
///
/// Defaults mirror iTerm2 for consistency with users coming from there.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// `OSC 9 ; <text>` — minimal one-shot notification (Title = app
    /// name, Body = text).
    pub osc9_enabled: bool,
    /// `OSC 777 ; notify ; <title> ; <body>` — rxvt-extended structured
    /// notification.
    pub osc777_enabled: bool,
    /// `OSC 1337 ; RequestAttention=<kind>` — dock bounce / window alert.
    pub attention_enabled: bool,
    /// When true, fire a notification for any command whose elapsed
    /// time between `OSC 133 ; B` (CommandStart) and `OSC 133 ; D`
    /// (CommandFinished) exceeds `long_running_threshold_secs`. Mirrors
    /// iTerm2's "Notify me when a session ends" / "for slow commands"
    /// preference.
    pub long_running_enabled: bool,
    /// Threshold in seconds for the long-running command channel. Has
    /// no effect when `long_running_enabled` is false. iTerm2 default
    /// is 30s.
    pub long_running_threshold_secs: u64,
    /// Suppress notifications for the pane that currently has focus so
    /// foreground work does not bounce its own dock icon. Only applies
    /// when the daruda app itself is the active app — backgrounded
    /// notifications always surface regardless of which pane fired.
    pub skip_focused_pane: bool,
    /// Raise a desktop notification when a Claude Code session emits a
    /// blocking hook `Notification` (permission prompt, idle prompt,
    /// elicitation dialog). These no longer latch the lane indicator
    /// into a persistent attention state — they surface once, here.
    /// Gated by `skip_focused_pane` like the other channels.
    pub hook_notification_enabled: bool,
    /// Raise a desktop notification when an agent-chat turn completes normally.
    pub agent_completion_enabled: bool,
    /// Raise a desktop notification when an agent-chat session enters a
    /// permission / input wait.
    pub agent_waiting_enabled: bool,
}

const LONG_RUNNING_DEFAULT_SECS: u64 = 30;

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            osc9_enabled: true,
            osc777_enabled: true,
            attention_enabled: true,
            long_running_enabled: true,
            long_running_threshold_secs: LONG_RUNNING_DEFAULT_SECS,
            skip_focused_pane: true,
            hook_notification_enabled: true,
            agent_completion_enabled: true,
            agent_waiting_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_channels_default_true() {
        let cfg = NotificationsConfig::default();
        assert!(cfg.agent_completion_enabled);
        assert!(cfg.agent_waiting_enabled);
    }

    #[test]
    fn toml_round_trip_preserves_explicit_false() {
        let toml_src = "\
agent_completion_enabled = false
agent_waiting_enabled = false
";
        let cfg: NotificationsConfig = toml::from_str(toml_src).unwrap();
        assert!(!cfg.agent_completion_enabled);
        assert!(!cfg.agent_waiting_enabled);
        // Unspecified fields fall back to their defaults via `#[serde(default)]`.
        assert!(cfg.hook_notification_enabled);

        let serialized = toml::to_string(&cfg).unwrap();
        let reparsed: NotificationsConfig = toml::from_str(&serialized).unwrap();
        assert!(!reparsed.agent_completion_enabled);
        assert!(!reparsed.agent_waiting_enabled);
    }
}
