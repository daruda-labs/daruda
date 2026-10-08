//! The keyboard-summoned pickers a window owns. Each keeps its own open
//! state; this only gives them one home on `Workspace`.

use super::flow_picker::FlowPicker;
use super::lane_switcher::LaneSwitcherState;
use super::palette::CommandPaletteState;

#[derive(Default)]
pub(in crate::workspace) struct CommandOverlays {
    /// Command palette state (Cmd+Shift+P).
    pub(in crate::workspace) palette: CommandPaletteState,
    /// Lane switcher state (Cmd+P) — fuzzy quick-switch across every
    /// project's lanes.
    pub(in crate::workspace) lane_switcher: LaneSwitcherState,
    /// Flow picker state — the list opened by `Run Flow…` / `Check Flow…`.
    pub(in crate::workspace) flow_picker: FlowPicker,
}
