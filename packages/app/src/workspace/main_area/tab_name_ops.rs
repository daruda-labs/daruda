//! A name the user gives a tab. It replaces the tab's derived label in the
//! tab strip, and a remote message about an agent running in the tab carries
//! it, so the phone can tell two chats in one worktree apart.

use gpui::{Context, SharedString, Window};

use super::pane_tree::PaneId;
use crate::surface::strings;
use crate::workspace::Workspace;

impl Workspace {
    /// Name tab `tab_id` in the active lane, or clear its name with `None`.
    pub(in crate::workspace) fn rename_tab(
        &mut self,
        tab_id: u64,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .active_runtime_mut()
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        tab.user_label = name.map(SharedString::from);
        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }

    /// A left press on tab `index`: activate it, and a double click also
    /// asks for a name. The orchestrator's tab keeps its own.
    pub(in crate::workspace) fn on_tab_press(
        &mut self,
        index: usize,
        tab_id: u64,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_tab(index, window, cx);
        let renamable = self
            .active_runtime()
            .tabs
            .get(index)
            .is_some_and(|tab| !self.is_orchestrator_tab(tab));
        if click_count == 2 && renamable {
            self.open_rename_tab_dialog(tab_id, window, cx);
        }
    }

    /// Ask for a new name for tab `tab_id`. The field starts from the name
    /// the user gave, not the derived label: confirming an untouched field
    /// must not freeze today's cwd into a name.
    pub(in crate::workspace) fn open_rename_tab_dialog(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_runtime().tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let current = tab.user_label.as_ref().map(|l| l.to_string());
        let placeholder = self.tab_label(tab, cx).unwrap_or_default();
        crate::workspace::dialog_helpers::open_single_field_dialog(
            cx.weak_entity(),
            strings::modal::rename_tab_title(),
            placeholder,
            current.as_deref(),
            move |ws, value, _window, cx| ws.rename_tab(tab_id, value, cx),
            window,
            cx,
        );
    }

    /// The name the user gave the tab holding `pane_id`, in any lane,
    /// flattened and capped like an agent title before it leaves the app.
    pub(in crate::workspace) fn pane_tab_name(&self, pane_id: PaneId) -> Option<String> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.tabs.iter())
            .find(|tab| tab.layout.contains(pane_id))?
            .user_label
            .as_deref()
            .and_then(crate::control::agent_text::sanitize_title)
    }
}
