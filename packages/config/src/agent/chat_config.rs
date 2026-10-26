//! The `[agent]` section (chat input and reading-width settings, legacy keys
//! awaiting migration) and the per-session model / mode preference rule.

use super::AgentDefinition;
use serde::{Deserialize, Serialize};

/// The app-wide transcript keys a pre-catalog `[agent]` section could state.
/// Carried from load to [`crate::Config::clamp`] and nowhere else.
#[derive(Debug, Clone, Default)]
pub(crate) struct LegacyTranscript {
    pub(crate) fold_mode: Option<Vec<String>>,
    pub(crate) tail_window: Option<u8>,
    pub(crate) display_filter: Option<Vec<String>>,
}

impl LegacyTranscript {
    pub(crate) fn is_empty(&self) -> bool {
        self.fold_mode.is_none() && self.tail_window.is_none() && self.display_filter.is_none()
    }
}

/// Agent chat configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Legacy global permission mode from pre per-agent-mode configs.
    /// Deserialized only so [`crate::Config::clamp`] can migrate it into the
    /// agent catalog; never written back.
    #[doc(hidden)]
    #[serde(default, rename = "default_permission_mode", skip_serializing)]
    pub legacy_default_permission_mode: Option<String>,
    /// Legacy app-wide transcript presentation, from before the catalog held
    /// these per agent. Same treatment as
    /// [`Self::legacy_default_permission_mode`]: deserialized only so
    /// [`crate::Config::clamp`] can lift them onto the entries that state
    /// nothing, never written back.
    #[doc(hidden)]
    #[serde(default, rename = "fold_mode", skip_serializing)]
    pub legacy_fold_mode: Option<Vec<String>>,
    #[doc(hidden)]
    #[serde(default, rename = "tail_window", skip_serializing)]
    pub legacy_tail_window: Option<u8>,
    #[doc(hidden)]
    #[serde(default, rename = "display_filter", skip_serializing)]
    pub legacy_display_filter: Option<Vec<String>>,
    /// How the agent chat input submits a message. When `false` (the
    /// default), plain Enter sends and Shift+Enter inserts a newline —
    /// matching Zed's agent panel default. When `true`, Enter inserts a
    /// newline and Cmd+Enter (Ctrl+Enter on Linux/Windows) sends. Only
    /// affects the bottom input while an agent chat pane is focused; the
    /// terminal input always uses Cmd+Enter to send.
    pub use_modifier_to_send: bool,
    /// Maximum number of visible rows before the bottom input scrolls
    /// internally. The input auto-grows from 1 row up to this limit,
    /// then clips and scrolls. Clamped to
    /// [`INPUT_MAX_ROWS_MIN`]..=[`INPUT_MAX_ROWS_MAX`] at load time.
    pub input_max_rows: u8,
    /// Whether a fresh AgentChat pane starts in reading-width mode. Seeds the
    /// pane; the per-pane toggle still wins, and a pane that has used it keeps
    /// its own choice across restarts.
    pub use_reading_width: bool,
    /// Fixed content-column width used when an AgentChat pane is in
    /// reading-width mode — the column [`Self::use_reading_width`] turns on.
    /// Clamped to [`READING_WIDTH_MIN`]..=[`READING_WIDTH_MAX`] at load time.
    pub reading_width: f32,
    /// Whether a tool-group summary spells each category out ("3 commands")
    /// rather than showing its icon and count alone. Off trades the words for
    /// a row that keeps mixed groups on one line.
    pub tool_summary_labels: bool,
    /// Session config options to hide from the input-dock chip row, matched by
    /// the option's advertised `description` (exact string). Presentation-only:
    /// the option stays in the session state and the agent can still change it.
    ///
    /// Exists because the protocol carries no structured "unusable" signal: an
    /// adapter that knows why an option can't be used folds the reason into the
    /// description text, and one that doesn't advertises the same description
    /// as the working state. Matching adapter-authored text is inherently
    /// loose: when an adapter rewords, the entry stops matching and the chip
    /// simply shows again.
    ///
    /// Defaults to [`FAST_MODE_PLAIN_DESCRIPTION`]; set
    /// `hidden_config_option_descriptions = []` to show every advertised option.
    pub hidden_config_option_descriptions: Vec<String>,
}

/// The Claude adapter's Fast mode description in its reason-less form — the
/// text it advertises both while the toggle works and while the CLI declines
/// it without a stated reason. Hidden by default because the toggle does not
/// stick through the ACP adapter path (the CLI reverts it on its next
/// `fast_mode_state` report), leaving a chip that silently flips back. The
/// reason-carrying variant ("… — not available on the free plan" etc.) is a
/// different string, so an adapter that can explain itself keeps its chip.
pub const FAST_MODE_PLAIN_DESCRIPTION: &str = "Faster responses on supported models";

/// Default for [`AgentConfig::hidden_config_option_descriptions`].
fn default_hidden_config_option_descriptions() -> Vec<String> {
    vec![FAST_MODE_PLAIN_DESCRIPTION.to_string()]
}

/// Minimum allowed value for `AgentConfig::input_max_rows`.
pub const INPUT_MAX_ROWS_MIN: u8 = 2;
/// Maximum allowed value for `AgentConfig::input_max_rows`.
pub const INPUT_MAX_ROWS_MAX: u8 = 20;
/// Default maximum rows for the bottom input before it scrolls.
pub const INPUT_MAX_ROWS_DEFAULT: u8 = 3;
/// Minimum readable-content column width for AgentChat.
pub const READING_WIDTH_MIN: f32 = 360.0;
/// Maximum readable-content column width for AgentChat.
pub const READING_WIDTH_MAX: f32 = 2400.0;
/// Default readable-content column width for AgentChat.
pub const READING_WIDTH_DEFAULT: f32 = 700.0;
/// Whether a fresh AgentChat pane starts held to that column.
pub const USE_READING_WIDTH_DEFAULT: bool = true;

/// Sentinel meaning every work step is visible.
pub const TAIL_WINDOW_ALL: u8 = 0;
/// Tail-window sizes offered by the AgentChat chip.
pub const TAIL_WINDOW_CHOICES: [u8; 4] = [1, 3, 5, 10];
pub const TAIL_WINDOW_DEFAULT: u8 = TAIL_WINDOW_ALL;

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            legacy_default_permission_mode: None,
            legacy_fold_mode: None,
            legacy_tail_window: None,
            legacy_display_filter: None,
            use_modifier_to_send: false,
            input_max_rows: INPUT_MAX_ROWS_DEFAULT,
            use_reading_width: USE_READING_WIDTH_DEFAULT,
            reading_width: READING_WIDTH_DEFAULT,
            tool_summary_labels: true,
            hidden_config_option_descriptions: default_hidden_config_option_descriptions(),
        }
    }
}

impl AgentConfig {
    pub(crate) fn take_legacy_default_permission_mode(&mut self) -> Option<String> {
        self.legacy_default_permission_mode
            .take()
            .map(|mode| mode.trim().to_string())
            .filter(|mode| !mode.is_empty())
    }

    /// The legacy app-wide transcript keys, taken so a later save cannot write
    /// them back. All three at once because they are migrated together.
    pub(crate) fn take_legacy_transcript(&mut self) -> LegacyTranscript {
        LegacyTranscript {
            fold_mode: self.legacy_fold_mode.take(),
            tail_window: self.legacy_tail_window.take(),
            display_filter: self.legacy_display_filter.take(),
        }
    }

    /// Clamp numeric fields to their valid ranges.
    pub fn clamp(&mut self) {
        self.input_max_rows = self
            .input_max_rows
            .clamp(INPUT_MAX_ROWS_MIN, INPUT_MAX_ROWS_MAX);
        self.reading_width = crate::clamp_or(
            self.reading_width,
            READING_WIDTH_MIN,
            READING_WIDTH_MAX,
            Self::default().reading_width,
        );
    }
}

/// What a chat pane asks its agent for: the pane's own picks, each ahead of
/// the agent's catalog default. The one rule behind both a connect and a live
/// config edit, so the two cannot disagree on which value wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPreferences {
    /// `None` leaves the adapter's own model standing.
    pub model: Option<String>,
    /// Priority-ordered: the first candidate the agent advertises applies, so
    /// one outside its vocabulary falls through to the next. That is what
    /// makes a per-agent override safe to state as free-form text. Empty
    /// leaves the adapter's own mode standing.
    pub modes: Vec<String>,
}

impl SessionPreferences {
    /// `agent` is `None` for an id no longer in the catalog — only the
    /// pane's picks then apply.
    pub fn resolve(
        agent: Option<&AgentDefinition>,
        picked_model: Option<&str>,
        picked_mode: Option<&str>,
    ) -> Self {
        let model = [picked_model, agent.and_then(|a| a.default_model.as_deref())]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|id| !id.is_empty())
            .map(str::to_string);
        let mut modes: Vec<String> = Vec::new();
        for id in [picked_mode, agent.and_then(|a| a.default_mode.as_deref())]
            .into_iter()
            .flatten()
        {
            let id = id.trim();
            if !id.is_empty() && !modes.iter().any(|m| m == id) {
                modes.push(id.to_string());
            }
        }
        Self { model, modes }
    }
}
