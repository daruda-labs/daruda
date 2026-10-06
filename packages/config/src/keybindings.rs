use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User keybinding overrides.
///
/// Keys are GPUI-style key strings (e.g. "cmd-t", "cmd-shift-d").
/// Values are action names (e.g. "new_tab", "split_right") or raw
/// escape sequences prefixed with `\x1b` (e.g. "\\x1b[A").
///
/// These are merged on top of the built-in defaults at startup.
/// The config crate only stores the raw string pairs; mapping to
/// concrete GPUI actions happens in the app crate.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct KeybindingConfig {
    #[serde(flatten)]
    pub bindings: HashMap<String, String>,
}
