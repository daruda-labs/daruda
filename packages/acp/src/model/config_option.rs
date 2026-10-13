//! Session config options as the host renders them — category, kind, and
//! current value — mapped from the protocol's `SessionConfigOption`.

#[cfg(doc)]
use super::ModeStateView;

/// Semantic category of a session config option (mirror of the protocol's
/// `SessionConfigOptionCategory`). Drives which input chip renders the option;
/// the host groups options by this rather than by id. `Mode` is also surfaced
/// via [`ModeStateView`] (the adapter advertises mode on both paths), so the
/// host renders mode through the existing mode chip and uses config options for
/// `Model` / `ThoughtLevel` / `ModelConfig` / `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOptionCategoryView {
    /// Permission / session mode selector.
    Mode,
    /// Model selector.
    Model,
    /// Model-related parameter (e.g. context window).
    ModelConfig,
    /// Reasoning / thinking effort level.
    ThoughtLevel,
    /// Uncategorized, custom (`_`-prefixed), or a category added by a newer
    /// protocol than this build knows — rendered generically.
    Other,
}

/// The model choice claude-agent-acp offers for "follow the CLI's default";
/// its `description` names the concrete model that default resolves to.
pub const DEFAULT_MODEL_CHOICE: &str = "default";

/// The concrete model a model choice stands for when it is the
/// [`DEFAULT_MODEL_CHOICE`] and the adapter named one in its description.
pub fn resolved_default_model<'a>(value: &str, description: Option<&'a str>) -> Option<&'a str> {
    description.filter(|model| value == DEFAULT_MODEL_CHOICE && !model.is_empty())
}

/// One selectable value of a config option (mirror of the protocol's
/// `SessionConfigSelectOption`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChoiceView {
    /// Stable id submitted via `set_config_option` to pick this value.
    pub value: String,
    /// Human-readable label shown in the dropdown.
    pub name: String,
    /// Optional longer description, shown as secondary text / tooltip.
    pub description: Option<String>,
}

/// What control an option renders as, together with the state that kind needs.
/// One enum rather than a kind tag beside always-present `current_value` /
/// `options` fields, so "a boolean with a choice list" is unrepresentable.
///
/// Boolean carries no labels: they are user-facing text, which must come from
/// the host's i18n layer (`surface/strings/`) and cannot be synthesized in
/// this GPUI-free crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOptionKindView {
    /// Dropdown over a fixed set of value ids.
    Select {
        /// `value` of the currently-selected choice.
        current_value: String,
        /// The selectable choices, flattened across any protocol grouping.
        options: Vec<ConfigChoiceView>,
    },
    /// On/off toggle.
    Boolean {
        /// The current state.
        current_value: bool,
    },
}

/// A value submitted for a config option. Typed so the wire form matches the
/// option's kind — a boolean must go out as `{"type":"boolean","value":…}`, not
/// as the bare value id a select uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValueView {
    /// The id of a select choice.
    Id(String),
    /// A boolean toggle's new state.
    Bool(bool),
}

/// An advertised session config option (mirror of the protocol's
/// `SessionConfigOption`). Advertised at connect time in
/// `NewSessionResponse.config_options` and replaced wholesale by
/// `SetSessionConfigOptionResponse`. Kinds this build does not know are dropped
/// at the mapping boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOptionView {
    /// Stable identifier used to set this option via `set_config_option`.
    pub id: String,
    /// Human-readable label (e.g. "Model", "Effort").
    pub name: String,
    /// Optional longer description, shown in a tooltip.
    pub description: Option<String>,
    /// Semantic category, used to route the option to the right chip.
    pub category: ConfigOptionCategoryView,
    /// The control this option renders as, and its current state.
    pub kind: ConfigOptionKindView,
}

impl ConfigOptionView {
    /// Map a protocol `SessionConfigOption` to a view, or `None` for a kind this
    /// build does not render (forward-compatible with protocol extensions —
    /// `SessionConfigKind` is `#[non_exhaustive]`).
    ///
    /// A boolean only ever arrives when the host advertised
    /// `session.configOptions.boolean` at `initialize`; without that opt-in the
    /// agent degrades the same option to a two-value select.
    pub fn from_protocol(
        o: &agent_client_protocol::schema::v1::SessionConfigOption,
    ) -> Option<Self> {
        use agent_client_protocol::schema::v1::{
            SessionConfigKind, SessionConfigOptionCategory, SessionConfigSelectOptions,
        };
        let kind = match &o.kind {
            SessionConfigKind::Boolean(b) => ConfigOptionKindView::Boolean {
                current_value: b.current_value,
            },
            SessionConfigKind::Select(select) => {
                let options = match &select.options {
                    SessionConfigSelectOptions::Ungrouped(opts) => {
                        opts.iter().map(config_choice).collect()
                    }
                    // Grouped options are flattened (daruda's chip is a flat
                    // dropdown); group headers are dropped. The current adapter
                    // sends ungrouped.
                    SessionConfigSelectOptions::Grouped(groups) => groups
                        .iter()
                        .flat_map(|g| g.options.iter())
                        .map(config_choice)
                        .collect(),
                    // `SessionConfigSelectOptions` is `#[non_exhaustive]`.
                    #[allow(unreachable_patterns)]
                    _ => Vec::new(),
                };
                ConfigOptionKindView::Select {
                    current_value: select.current_value.to_string(),
                    options,
                }
            }
            #[allow(unreachable_patterns)]
            _ => return None,
        };
        let category = match &o.category {
            Some(SessionConfigOptionCategory::Mode) => ConfigOptionCategoryView::Mode,
            Some(SessionConfigOptionCategory::Model) => ConfigOptionCategoryView::Model,
            Some(SessionConfigOptionCategory::ModelConfig) => ConfigOptionCategoryView::ModelConfig,
            Some(SessionConfigOptionCategory::ThoughtLevel) => {
                ConfigOptionCategoryView::ThoughtLevel
            }
            // None, Other(_), or a future category → render generically.
            _ => ConfigOptionCategoryView::Other,
        };
        Some(ConfigOptionView {
            id: o.id.to_string(),
            name: o.name.clone(),
            description: o.description.clone(),
            category,
            kind,
        })
    }
}

fn config_choice(
    o: &agent_client_protocol::schema::v1::SessionConfigSelectOption,
) -> ConfigChoiceView {
    ConfigChoiceView {
        value: o.value.to_string(),
        name: o.name.clone(),
        description: o.description.clone(),
    }
}
