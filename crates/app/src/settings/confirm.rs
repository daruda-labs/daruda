//! Confirm-first entry points for the Settings actions that cannot be undone.
//!
//! Each button calls a `request_*` here instead of the action itself; the
//! action runs only from the dialog's OK. See
//! [`crate::workspace::dialog_helpers::confirm_destructive`] for the rule.

use gpui::{Context, Window};

use super::{AgentCatalogItem, SettingsView};
use crate::surface::strings as s;
use crate::workspace::dialog_helpers::confirm_destructive;

impl SettingsView {
    pub(super) fn request_clear_telegram_token(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        confirm_destructive(
            cx.weak_entity(),
            s::settings_confirm_remove_telegram_token_title(),
            s::settings_confirm_remove_token_body(),
            s::settings_confirm_ok_remove_token(),
            |this, _window, cx| this.clear_telegram_token(cx),
            window,
            cx,
        );
    }

    pub(super) fn request_unpair_telegram(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        confirm_destructive(
            cx.weak_entity(),
            s::settings_confirm_unpair_telegram_title(),
            s::settings_confirm_unpair_body(),
            s::settings_confirm_ok_unpair(),
            |this, _window, cx| this.unpair_telegram(cx),
            window,
            cx,
        );
    }

    /// A row the user has never saved goes at once: nothing outside the
    /// window knows it yet, so there is nothing to confirm.
    pub(super) fn request_remove_session_host_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.session_host_rows.get(index) else {
            return;
        };
        let id = row.id;
        let saved = self.base_config.session_hosts.iter().any(|e| e.id == id);
        if !saved {
            self.remove_session_host_by_id(&id, cx);
            return;
        }
        let name = row.label_input.read(cx).value().trim().to_string();
        confirm_destructive(
            cx.weak_entity(),
            named_title(&name, s::settings_confirm_remove_session_host_title),
            s::settings_confirm_remove_from_config_body(),
            s::settings_confirm_ok_remove(),
            // By id, not position: the list can be rebuilt while the dialog
            // is open, and a stale index would remove a different host.
            move |this, _window, cx| this.remove_session_host_by_id(&id, cx),
            window,
            cx,
        );
    }

    pub(super) fn request_remove_agent_catalog_item(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.agent_catalog.get(index) else {
            return;
        };
        let key = CatalogKey::of(item);
        let name = match item {
            AgentCatalogItem::Editable(row) => row.name_input.read(cx).value().trim().to_string(),
            AgentCatalogItem::Unresolved(entry) => {
                entry.preset_id().unwrap_or_default().to_string()
            }
        };
        confirm_destructive(
            cx.weak_entity(),
            named_title(&name, s::settings_confirm_remove_agent_title),
            s::settings_confirm_remove_from_config_body(),
            s::settings_confirm_ok_remove(),
            move |this, _window, cx| {
                if let Some(index) = this.agent_catalog.iter().position(|i| key.matches(i)) {
                    this.remove_agent_catalog_item(index, cx);
                }
            },
            window,
            cx,
        );
    }

    pub(super) fn remove_session_host_by_id(
        &mut self,
        id: &daruda_store::project::SessionHostId,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.session_host_rows.iter().position(|r| &r.id == id) {
            self.remove_session_host_row(index, cx);
        }
    }
}

/// Which catalog entry a confirm dialog was opened for. An editable row is
/// the row itself — its id field is user-editable — and an unresolved one
/// is its persisted entry. A reload replaces both, so the dialog then
/// removes nothing rather than whatever took the old position.
enum CatalogKey {
    Row(gpui::EntityId),
    Unresolved(Box<daruda_config::AgentEntry>),
}

impl CatalogKey {
    fn of(item: &AgentCatalogItem) -> Self {
        match item {
            AgentCatalogItem::Editable(row) => Self::Row(row.id_input.entity_id()),
            AgentCatalogItem::Unresolved(entry) => Self::Unresolved(Box::new(entry.clone())),
        }
    }

    fn matches(&self, item: &AgentCatalogItem) -> bool {
        match (self, item) {
            (Self::Row(id), AgentCatalogItem::Editable(row)) => row.id_input.entity_id() == *id,
            (Self::Unresolved(entry), AgentCatalogItem::Unresolved(other)) => **entry == *other,
            _ => false,
        }
    }
}

/// "Remove “name”?" when the entry has a name, else the generic title.
fn named_title(name: &str, generic: fn() -> String) -> String {
    if name.is_empty() {
        generic()
    } else {
        s::settings_confirm_remove_named(name)
    }
}
