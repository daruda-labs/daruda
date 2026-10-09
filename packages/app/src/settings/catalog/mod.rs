//! Agent catalog draft data, row lifecycle, and persistence.

mod bindings;
mod path_warning;
mod persistence;
mod rows;
mod validation;

use super::*;
use path_warning::agent_command_path_warning;
use validation::{agent_row_transport_error, is_valid_agent_id};

/// Which parts of a catalog card are open. View state only: never persisted,
/// and carried across a catalog reload so a save does not fold the card the
/// user is editing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::settings) struct CardFold {
    /// The detail fields below the card's header.
    pub(in crate::settings) expanded: bool,
    /// The advanced block inside the details — command, transport, env.
    pub(in crate::settings) advanced: bool,
}

#[derive(Clone)]
pub(in crate::settings) struct AgentCatalogRow {
    /// Whether the entry is switched on — written as `enabled = false` when
    /// not, with every other field kept.
    pub(in crate::settings) enabled: bool,
    pub(in crate::settings) fold: CardFold,
    /// The preset this row references, when it has one. Kept so saving
    /// re-derives the row's overrides against that preset instead of writing a
    /// frozen copy — an untouched field keeps tracking the preset.
    pub(in crate::settings) preset: Option<String>,
    pub(in crate::settings) id_input: Entity<InputState>,
    pub(in crate::settings) name_input: Entity<InputState>,
    /// The command that runs the ACP adapter — `Raw`'s full string, or the
    /// `adapter_command` sub-field when `transport_select` is `ssh`/`docker`.
    pub(in crate::settings) command_input: Entity<InputState>,
    /// Transport kind for this row: `"raw"` / `"ssh"` / `"docker"` — mirrors
    /// [`daruda_config::AgentLaunch`]'s three variants.
    pub(in crate::settings) transport_select: Entity<SelectState>,
    /// SSH host — only meaningful (and only rendered) when `transport_select`
    /// is `"ssh"`.
    pub(in crate::settings) host_input: Entity<InputState>,
    /// Docker container name — only meaningful (and only rendered) when
    /// `transport_select` is `"docker"`.
    pub(in crate::settings) container_input: Entity<InputState>,
    /// Optional session mode to request when this agent connects. Options are
    /// the agent's cached vocabulary, falling back to the adapter seed the
    /// command names; the empty value is the "agent default" sentinel that
    /// means no override. Rebuilt whenever the row's id or command changes —
    /// see [`SettingsView::refresh_agent_row_vocabulary`].
    pub(in crate::settings) default_mode_select: Entity<SelectState>,
    /// Optional model to request when this agent connects. Same option
    /// sourcing and same empty sentinel as `default_mode_select`.
    pub(in crate::settings) default_model_select: Entity<SelectState>,
    /// The command's executable name, when [`agent_command_path_warning`]
    /// determined it names a local binary not found on `PATH` — `None` when
    /// no check applies (`npx`/`uvx`/JSON stdio) or the binary was found.
    /// Independent of transport: an ssh/docker row's warning is suppressed at
    /// render time instead (see `sections::agent_catalog::render_agent_catalog_row`),
    /// since that needs no fresh `which` lookup. Recomputed on construction
    /// and whenever `command_input` changes (see
    /// [`SettingsView::recompute_agent_row_path_warning`]); `which::which`
    /// is I/O, so `render` only ever reads this field, never calls it.
    pub(in crate::settings) path_warning: Option<String>,
    /// The environment the Environment field's text was built against: its
    /// preset's for a row loaded as a reference, none for a custom row. An
    /// emptied field clears only what it was shown, so this — not the row's
    /// current preset, which a save can promote it to — decides that.
    pub(in crate::settings) env_field_base: Option<Vec<(String, String)>>,
    /// Fold rules a fresh chat pane under this agent starts on, or `None` to
    /// write no key — which resolves to the built-in. Edited through the same
    /// editor the chat pane opens; see [`sections::agent_transcript`].
    pub(in crate::settings) fold_mode: Option<FoldMode>,
    /// The `fold_mode` tokens this row loaded, kept so an untouched axis is
    /// written back exactly as it was read — see `sections::agent_transcript`.
    pub(in crate::settings) fold_mode_loaded: Option<Vec<String>>,
    /// Where this row's fold editor is looking. Not part of the value: the pane
    /// editing the same agent keeps its own — see [`FoldEditorState`].
    pub(in crate::settings) fold_editor: FoldEditorState,
    /// Which of this row's Visible-items sections are shut — presentation
    /// only, like `fold_editor`.
    pub(in crate::settings) filter_editor: FilterEditorState,
    /// The transcript editor this row renders already open — the
    /// `--screenshot-scenario agent-catalog-editor` seam. Nothing else sets it.
    #[cfg(feature = "screenshot")]
    pub(in crate::settings) shot_editor: Option<sections::agent_transcript::editor::EditorShot>,
    /// Visible row kinds a fresh chat pane starts on. Same `None`-is-built-in
    /// rule as `fold_mode`.
    pub(in crate::settings) display_filter: Option<DisplayFilter>,
    /// The `display_filter` tokens this row loaded — same rule as
    /// `fold_mode_loaded`.
    pub(in crate::settings) display_filter_loaded: Option<Vec<String>>,
    /// Trailing-step window a fresh chat pane starts on, one picker per level
    /// of that axis: this one for a response's work steps, the pair below for
    /// the calls inside one of them. Still dropdowns, and so still able to load
    /// a size they cannot offer.
    pub(in crate::settings) tail_window_select: Entity<SelectState>,
    /// The size this row loaded when the dropdown above cannot state it. It
    /// backs that picker's configured-elsewhere entry and is written back
    /// verbatim while it stays picked, so an unrelated edit cannot flatten a
    /// hand-written value. `None` means the picker holds the whole value.
    pub(in crate::settings) tail_window_loaded: Option<u8>,
    pub(in crate::settings) tail_window_calls_select: Entity<SelectState>,
    pub(in crate::settings) tail_window_calls_loaded: Option<u8>,
    /// The environment the adapter process launches with, as one
    /// `KEY=value` per line. Which of the three
    /// [`daruda_config::AgentDefinition::env`] states an empty field means
    /// depends on the preset behind the row, so read it through
    /// [`AgentCatalogRow::stated_env`] rather than off the text.
    pub(in crate::settings) env_input: Entity<InputState>,
}

/// The localized message for an [`agent_row_transport_error`] on agent row
/// `ordinal` (1-based). `SessionPath` never reaches here — an agent row has
/// no path field — but is matched rather than left to an `unreachable!()`.
pub(in crate::settings) fn agent_row_transport_message(
    ordinal: usize,
    err: session_host::SessionHostError,
) -> SharedString {
    use session_host::{SessionHostError, SessionHostField};
    SharedString::from(match err {
        SessionHostError::Empty(SessionHostField::Target) => {
            s::settings::err_agent_catalog_host(ordinal)
        }
        SessionHostError::Empty(SessionHostField::Container) => {
            s::settings::err_agent_catalog_container(ordinal)
        }
        SessionHostError::Unsafe(SessionHostField::Target) => {
            s::settings::err_agent_catalog_host_unsafe(ordinal)
        }
        SessionHostError::Unsafe(SessionHostField::Container) => {
            s::settings::err_agent_catalog_container_unsafe(ordinal)
        }
        SessionHostError::Empty(SessionHostField::SessionPath)
        | SessionHostError::Unsafe(SessionHostField::SessionPath) => {
            s::settings::err_agent_catalog_host(ordinal)
        }
    })
}
