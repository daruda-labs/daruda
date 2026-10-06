//! Customizable bottom dock panels — data model + persistence.
//!
//! Each tab holds a list of widgets. Currently the only widget kind is
//! `Button` (a clickable macro that sends text to the focused PTY), but
//! the schema is forward-compatible: future widget variants land as
//! `MacroKey::Unknown(value)` in older daruda versions, and the original
//! JSON survives round-trip so user data is never silently dropped.

mod persistence;
mod seed;

#[cfg(test)]
mod tests;

pub use persistence::{load_panels, load_panels_in, panels_path_in, save_panels, save_panels_in};
pub use seed::seed_default;

/// Migrate seed-button state in a loaded `PanelsState`.
///
/// Two cases are handled for every `Button` widget:
///
/// 1. **Already builtin** (`builtin == true`) — if `send` has drifted
///    from the canonical value (e.g. user edited `panels.json` by hand),
///    restore it. The label is trusted as the lookup key because daruda
///    is the only writer of `builtin: true` buttons.
///
/// 2. **Not yet flagged** (`builtin == false`) — if `send` exactly
///    equals a canonical seed payload, set `builtin: true`. This covers
///    files written before the `builtin` field was introduced, where the
///    seed buttons still carry the original (unmodified) send strings.
///    Buttons whose `send` was user-modified are left untouched — their
///    label alone is not enough evidence that they are seed buttons.
///
/// Returns `true` when at least one field was updated. The caller
/// should re-persist so future loads skip this work.
pub fn migrate_builtin_flags(state: &mut PanelsState) -> bool {
    let mut changed = false;
    for tab in &mut state.tabs {
        for widget in &mut tab.widgets {
            if let MacroKey::Button(btn) = widget {
                if btn.builtin {
                    // Restore canonical send if it drifted.
                    if let Some(canonical_send) = seed::SEED_AI_ENTRIES
                        .iter()
                        .find(|(label, _)| *label == btn.label.as_str())
                        .map(|(_, send)| *send)
                        && btn.send != canonical_send
                    {
                        btn.send = canonical_send.to_string();
                        changed = true;
                    }
                } else if seed::SEED_AI_ENTRIES
                    .iter()
                    .any(|(_, send)| *send == btn.send.as_str())
                {
                    // Pre-`builtin` seed file: send is still canonical.
                    btn.builtin = true;
                    changed = true;
                }
            }
        }
    }
    changed
}

use serde::de::Error as DeError;
use serde::ser::Error as SerError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Persistence schema version. Bump when the structural contract changes
/// in a way `MacroKey::Unknown` cannot transparently absorb (e.g. a tab-level
/// rework, not just a new widget variant).
pub const SCHEMA_VERSION: u32 = 1;

/// Stable identifier for a tab. ULID encoded as string — sortable by
/// creation time and globally unique without coordination.
pub type TabId = String;

/// Stable identifier for a widget within a tab.
pub type WidgetId = String;

/// Generate a fresh tab id.
pub fn new_tab_id() -> TabId {
    ulid::Ulid::new().to_string()
}

/// Generate a fresh widget id.
pub fn new_widget_id() -> WidgetId {
    ulid::Ulid::new().to_string()
}

/// Top-level state — the entire `panels.json` file deserializes into this.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PanelsState {
    pub schema_version: u32,
    #[serde(default)]
    pub tabs: Vec<PanelTab>,
    #[serde(default)]
    pub active_tab_id: Option<TabId>,
}

impl Default for PanelsState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            tabs: Vec::new(),
            active_tab_id: None,
        }
    }
}

/// A single tab in the bottom dock.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PanelTab {
    pub id: TabId,
    pub name: String,
    pub order: u32,
    /// `None` = auto-fit content height; `Some(px)` = fixed pixel height.
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub layout: TabLayout,
    #[serde(default)]
    pub widgets: Vec<MacroKey>,
}

/// How the widgets inside a tab are laid out. New algorithms land as new
/// variants; unknown values fall back to `FlexWrap` on load.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TabLayout {
    #[default]
    FlexWrap,
}

/// A widget inside a tab.
///
/// Custom (de)serialize so that:
///   * `Button` deserializes from `{"type": "button", ...}` cleanly.
///   * Any other `type` (or even a missing `type`) is captured as
///     `Unknown(value)` preserving the entire JSON object — when the user
///     opens a panels file authored by a newer daruda version, those
///     widgets survive a save round-trip instead of being dropped.
#[derive(Clone, Debug, PartialEq)]
pub enum MacroKey {
    Button(ButtonWidget),
    /// Captures the raw JSON for any widget type not yet known to this
    /// daruda version. The render layer ignores these.
    Unknown(serde_json::Value),
}

impl MacroKey {
    /// `id` accessor that works for both Button and Unknown (when the
    /// unknown carries an `id` field).
    pub fn id(&self) -> Option<&str> {
        match self {
            MacroKey::Button(b) => Some(&b.id),
            MacroKey::Unknown(v) => v.get("id").and_then(|v| v.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for MacroKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let type_str = value.get("type").and_then(|v| v.as_str());
        match type_str {
            Some("button") => {
                let btn: ButtonWidget = serde_json::from_value(value).map_err(D::Error::custom)?;
                Ok(MacroKey::Button(btn))
            }
            _ => Ok(MacroKey::Unknown(value)),
        }
    }
}

impl Serialize for MacroKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MacroKey::Button(btn) => {
                let mut value = serde_json::to_value(btn).map_err(S::Error::custom)?;
                if let serde_json::Value::Object(ref mut map) = value {
                    map.insert(
                        "type".to_string(),
                        serde_json::Value::String("button".to_string()),
                    );
                }
                value.serialize(serializer)
            }
            MacroKey::Unknown(value) => value.serialize(serializer),
        }
    }
}

/// Click-to-send macro button widget.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ButtonWidget {
    pub id: WidgetId,
    pub label: String,
    pub send: String,
    #[serde(default = "default_auto_enter")]
    pub auto_enter: bool,
    #[serde(default)]
    pub display: ButtonDisplay,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    /// L1 always emits `null` and round-trips any non-null value
    /// untouched. L2 will define a `ButtonStyle` schema and replace
    /// `serde_json::Value` with a typed struct.
    #[serde(default)]
    pub style: Option<serde_json::Value>,
    /// Seed / built-in buttons provided by daruda. Their `send` content
    /// cannot be edited via the UI — only label and shortcut are
    /// mutable. Deletion is still allowed. Defaults to `false` so
    /// existing user-created buttons in `panels.json` are unaffected.
    #[serde(default)]
    pub builtin: bool,
}

fn default_auto_enter() -> bool {
    true
}

/// Whether a button widget renders as a text label or a square icon tile.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ButtonDisplay {
    #[default]
    Text,
    Icon,
}
