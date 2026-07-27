//! What the connected agent advertises about how a session runs.
//!
//! Session modes, select config options (model / effort / …), and slash
//! commands share one lifecycle: each is established by `Connected`, replaced
//! wholesale by its own [`daruda_acp::AcpEvent`], and cleared together when the
//! session is torn down. Grouping them gives that lifecycle a single owner —
//! a teardown resets one field instead of remembering three — and gives the
//! derivations the render and ops layers need a home next to the data instead
//! of open-coded at each call site.

use daruda_acp::{ConfigOptionView, ModeStateView, SlashCommand};

use super::agent_chat_helpers::next_mode_id;

/// The agent-advertised configuration of one live session.
#[derive(Default)]
pub(in crate::workspace) struct SessionConfig {
    /// Session modes and which one is active. `None` until the session
    /// connects, or when the agent does not advertise modes. Replaced wholesale
    /// on `ModeChanged` — `daruda_acp` reconciles the protocol's two mode
    /// channels so this is the host's only mode mirror.
    pub(in crate::workspace) modes: Option<ModeStateView>,
    /// Select config options advertised by the agent, replaced wholesale on
    /// `ConfigOptionsChanged`. Never carries a `Mode`-category option:
    /// `daruda_acp` strips it, since [`Self::modes`] already holds that fact.
    pub(in crate::workspace) config_options: Vec<ConfigOptionView>,
    /// Slash commands the agent advertises, replaced wholesale on
    /// `AvailableCommandsChanged`.
    pub(in crate::workspace) available_commands: Vec<SlashCommand>,
}

impl SessionConfig {
    /// The mode state the mode chip renders, or `None` when there is nothing to
    /// show — an agent without modes, or one advertising an empty list (which
    /// can't be displayed or cycled).
    pub(in crate::workspace) fn mode_for_chip(&self) -> Option<&ModeStateView> {
        self.modes.as_ref().filter(|m| !m.available.is_empty())
    }

    /// `id` of the mode the session is in, when the agent advertises modes.
    pub(in crate::workspace) fn current_mode_id(&self) -> Option<&str> {
        self.modes.as_ref().map(|m| m.current.as_str())
    }

    /// Display name of the mode the session is in. Falls back to `None` when
    /// the agent reports a mode absent from its advertised list — it stays
    /// authoritative about `current`, so the id can lead the list by an update.
    pub(in crate::workspace) fn current_mode_name(&self) -> Option<&str> {
        let modes = self.modes.as_ref()?;
        modes
            .available
            .iter()
            .find(|m| m.id == modes.current)
            .map(|m| m.name.as_str())
    }

    /// The mode the Shift+Tab cycle advances to, or `None` when there is
    /// nothing to cycle (fewer than two advertised modes).
    pub(in crate::workspace) fn next_mode_id(&self) -> Option<String> {
        self.modes.as_ref().and_then(next_mode_id)
    }

    /// Point the chip at `mode_id` before the agent confirms, so the selection
    /// reads as immediate; a `ModeChanged` replaces the whole state if the
    /// agent disagrees. No-op for an agent without modes.
    pub(in crate::workspace) fn set_current_mode_optimistically(&mut self, mode_id: String) {
        if let Some(m) = &mut self.modes {
            m.current = mode_id;
        }
    }

    /// The config-option counterpart of [`Self::set_current_mode_optimistically`]:
    /// show the picked value before the agent replies with the updated set.
    /// No-op when the agent advertises no such option.
    pub(in crate::workspace) fn set_option_value_optimistically(
        &mut self,
        config_id: &str,
        value: String,
    ) {
        if let Some(opt) = self.config_options.iter_mut().find(|o| o.id == config_id) {
            opt.current_value = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::SessionModeView;

    fn mode(id: &str, name: &str) -> SessionModeView {
        SessionModeView {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
        }
    }

    fn option(id: &str, current: &str) -> ConfigOptionView {
        ConfigOptionView {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            category: daruda_acp::ConfigOptionCategoryView::Model,
            current_value: current.to_string(),
            options: Vec::new(),
        }
    }

    fn with_modes(current: &str, available: Vec<SessionModeView>) -> SessionConfig {
        SessionConfig {
            modes: Some(ModeStateView {
                available,
                current: current.to_string(),
            }),
            ..SessionConfig::default()
        }
    }

    #[test]
    fn a_modeless_agent_has_no_mode_derivations() {
        let config = SessionConfig::default();
        assert!(config.mode_for_chip().is_none());
        assert_eq!(config.current_mode_id(), None);
        assert_eq!(config.current_mode_name(), None);
        assert_eq!(config.next_mode_id(), None);
    }

    #[test]
    fn an_empty_mode_list_hides_the_chip() {
        let config = with_modes("default", Vec::new());
        assert!(
            config.mode_for_chip().is_none(),
            "an empty selector can't be rendered or cycled"
        );
        assert_eq!(
            config.current_mode_id(),
            Some("default"),
            "the session is still in a mode even with nothing to pick from"
        );
    }

    #[test]
    fn mode_name_resolves_through_the_advertised_list() {
        let config = with_modes(
            "plan",
            vec![mode("default", "Manual"), mode("plan", "Plan")],
        );
        assert_eq!(config.current_mode_name(), Some("Plan"));
        assert!(config.mode_for_chip().is_some());
    }

    #[test]
    fn a_current_mode_ahead_of_the_list_has_no_name_yet() {
        // `ModeChanged` can report a mode id before the matching list lands.
        let config = with_modes("auto", vec![mode("default", "Manual")]);
        assert_eq!(config.current_mode_id(), Some("auto"));
        assert_eq!(config.current_mode_name(), None);
    }

    #[test]
    fn optimistic_writes_touch_only_the_named_entry() {
        let mut config = with_modes(
            "default",
            vec![mode("default", "Manual"), mode("plan", "Plan")],
        );
        config.config_options = vec![option("model", "sonnet"), option("effort", "high")];

        config.set_current_mode_optimistically("plan".to_string());
        config.set_option_value_optimistically("model", "opus".to_string());
        config.set_option_value_optimistically("nonexistent", "x".to_string());

        assert_eq!(config.current_mode_id(), Some("plan"));
        assert_eq!(config.config_options[0].current_value, "opus");
        assert_eq!(
            config.config_options[1].current_value, "high",
            "an unrelated option is untouched"
        );
    }

    #[test]
    fn optimistic_mode_write_is_a_noop_for_a_modeless_agent() {
        let mut config = SessionConfig::default();
        config.set_current_mode_optimistically("plan".to_string());
        assert_eq!(config.current_mode_id(), None);
    }
}
