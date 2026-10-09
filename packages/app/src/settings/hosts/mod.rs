//! Session host registry editing and persistence.

mod reconcile;

use super::*;
pub(in crate::settings) use reconcile::reconcile_session_host_tombstones;
use reconcile::session_host_entry_id;

/// One row of the session host registry editor. Unlike [`AgentCatalogRow`],
/// there is no preset concept here — every row is a plain user-entered
/// `{label, kind, target|container}`.
///
/// `id` is minted once, at construction, and never changes for the row's
/// lifetime: an existing row keeps the [`daruda_config::SessionHostId`] it
/// loaded from config, and a freshly added row mints its own right away
/// so [`SettingsView::validate`] can distinguish a persisted row from a
/// newly-added draft by id membership alone. The id persisted on commit can
/// still differ: a row whose Type changed retires it (see
/// [`session_host_entry_id`]).
#[derive(Clone)]
pub(in crate::settings) struct SessionHostRow {
    pub(in crate::settings) id: daruda_store::project::SessionHostId,
    pub(in crate::settings) label_input: Entity<InputState>,
    /// `"ssh"` / `"docker"` — mirrors [`daruda_config::SessionHostKind`]'s
    /// two variants.
    pub(in crate::settings) kind_select: Entity<SelectState>,
    /// SSH target — only meaningful (and only rendered) when `kind_select`
    /// is `"ssh"`.
    pub(in crate::settings) target_input: Entity<InputState>,
    /// Docker container name — only meaningful (and only rendered) when
    /// `kind_select` is `"docker"`.
    pub(in crate::settings) container_input: Entity<InputState>,
}

impl SessionHostRow {
    /// Whether the row's Type dropdown currently says Docker — the single
    /// read the renderer, the tab cycle and `validate` all branch on, so they
    /// can't disagree about which value field is live.
    pub(in crate::settings) fn is_docker(&self, cx: &gpui::App) -> bool {
        self.kind_select
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.as_ref() == "docker")
    }
}

impl SettingsView {
    pub(in crate::settings) fn subscribe_session_host_input(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.persist_session_hosts(cx);
                }
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::Focus => {}
            },
        )
    }

    /// Build one session-host row from a persisted entry — used both for the
    /// rows seeded at window-open and (via [`Self::add_session_host_row`])
    /// nowhere else, since a freshly added row starts blank rather than
    /// copying an existing entry.
    pub(in crate::settings) fn session_host_row_from_entry(
        entry: &daruda_config::SessionHostEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SessionHostRow {
        let (kind, target, container) = match &entry.kind {
            daruda_config::SessionHostKind::Ssh { target } => {
                ("ssh", target.clone(), String::new())
            }
            daruda_config::SessionHostKind::Docker { container } => {
                ("docker", String::new(), container.clone())
            }
        };
        Self::session_host_row_new(
            entry.id,
            &entry.label,
            kind,
            &target,
            &container,
            window,
            cx,
        )
    }

    /// Shared row constructor for both a loaded entry and a blank "Add Host"
    /// row — keeping one constructor means the two can never wire their
    /// inputs' subscriptions differently.
    pub(in crate::settings) fn session_host_row_new(
        id: daruda_store::project::SessionHostId,
        label: &str,
        kind: &str,
        target: &str,
        container: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SessionHostRow {
        SessionHostRow {
            id,
            label_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::session_host_field_label())
                    .default_value(label.to_string())
            }),
            kind_select: cx.new(|cx| {
                let opts = vec![
                    SelectOption::new("ssh", s::settings::session_host_kind_ssh()),
                    SelectOption::new("docker", s::settings::session_host_kind_docker()),
                ];
                select::state_with_options(opts, Some(&SharedString::from(kind)), window, cx)
            }),
            target_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::session_host_target_placeholder())
                    .default_value(target.to_string())
            }),
            container_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::session_host_container_placeholder())
                    .default_value(container.to_string())
            }),
        }
    }

    /// Wire one session-host row's inputs to the standard submit /
    /// clear-error subscription plus the kind-pick repaint, mirroring
    /// [`Self::subscribe_agent_row`].
    pub(in crate::settings) fn subscribe_session_host_row(
        row: &SessionHostRow,
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) {
        subs.push(Self::subscribe_session_host_input(
            &row.label_input,
            window,
            cx,
        ));
        subs.push(Self::subscribe_session_host_input(
            &row.target_input,
            window,
            cx,
        ));
        subs.push(Self::subscribe_session_host_input(
            &row.container_input,
            window,
            cx,
        ));
        // Re-render on kind pick so the row immediately shows/hides the
        // matching target/container field — same reason as the agent
        // catalog's transport select (see `subscribe_agent_row`).
        subs.push(cx.subscribe_in(
            &row.kind_select,
            window,
            |this, _state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.persist_session_hosts(cx);
                }
            },
        ));
    }

    /// Session-host rows paired with their catalog index — mirrors
    /// [`Self::agent_editable_rows`], minus the non-editable half agents
    /// have (every session-host row is always editable).
    pub(in crate::settings) fn session_host_rows(
        &self,
    ) -> impl Iterator<Item = (usize, &SessionHostRow)> + '_ {
        self.session_host_rows.iter().enumerate()
    }

    /// Test-only: the `ordinal`-th row — production code walks
    /// [`Self::session_host_rows`], which also yields the index every
    /// mutation needs.
    #[cfg(test)]
    pub(in crate::settings) fn session_host_row(&self, ordinal: usize) -> Option<&SessionHostRow> {
        self.session_host_rows.get(ordinal)
    }

    /// Append a blank row the user fills in by hand. A fresh
    /// [`daruda_store::project::SessionHostId`] is minted right away — see
    /// [`SessionHostRow::id`]'s doc for why.
    pub(in crate::settings) fn add_session_host_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = Self::session_host_row_new(
            daruda_store::project::SessionHostId::new(),
            "",
            "ssh",
            "",
            "",
            window,
            cx,
        );
        Self::subscribe_session_host_row(&row, window, cx, &mut self._input_subscriptions);
        self.session_host_rows.push(row);
        self.error = None;
        cx.notify();
    }

    /// Drop the row at `index` and commit the complete valid catalog. The
    /// missing persisted id becomes a tombstone in the same atomic patch.
    /// Mirrors [`Self::remove_agent_catalog_item`].
    pub(in crate::settings) fn remove_session_host_row(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index < self.session_host_rows.len() {
            let removed = self.session_host_rows.remove(index);
            self.error = None;
            if !self.persist_session_hosts(cx) {
                self.session_host_rows.insert(index, removed);
            }
            cx.notify();
        }
    }

    pub(in crate::settings) fn collect_session_hosts(
        &self,
        cx: &gpui::App,
    ) -> Result<Vec<daruda_config::SessionHostEntry>, SharedString> {
        let previous = &crate::settings_store::SettingsStore::global(cx)
            .user()
            .session_hosts;
        self.collect_session_hosts_against(previous, true, cx)
    }

    pub(in crate::settings) fn collect_session_hosts_against(
        &self,
        previous: &[daruda_config::SessionHostEntry],
        skip_blank_new: bool,
        cx: &gpui::App,
    ) -> Result<Vec<daruda_config::SessionHostEntry>, SharedString> {
        let mut entries = Vec::with_capacity(self.session_host_rows.len());
        let mut seen_labels = HashSet::new();
        for (index, row) in self.session_host_rows.iter().enumerate() {
            let label = row.label_input.read(cx).value().trim().to_string();
            let is_new = !previous.iter().any(|entry| entry.id == row.id);
            let target = row.target_input.read(cx).value().trim().to_string();
            let container = row.container_input.read(cx).value().trim().to_string();
            if skip_blank_new
                && is_new
                && label.is_empty()
                && target.is_empty()
                && container.is_empty()
            {
                continue;
            }
            if label.is_empty() {
                return Err(SharedString::from(
                    s::settings::err_session_host_label_empty(index + 1),
                ));
            }
            if !seen_labels.insert(label.to_ascii_lowercase()) {
                return Err(SharedString::from(
                    s::settings::err_session_host_label_duplicate(&label),
                ));
            }
            let kind = if row.is_docker(cx) {
                let container = session_host::checked_bare_word(
                    &container,
                    session_host::SessionHostField::Container,
                )
                .map_err(|error| session_host_validation_message(index, error))?;
                daruda_config::SessionHostKind::Docker { container }
            } else {
                let target = session_host::checked_bare_word(
                    &target,
                    session_host::SessionHostField::Target,
                )
                .map_err(|error| session_host_validation_message(index, error))?;
                daruda_config::SessionHostKind::Ssh { target }
            };
            entries.push(daruda_config::SessionHostEntry {
                id: session_host_entry_id(previous, row.id, &kind),
                label,
                kind,
            });
        }
        Ok(entries)
    }

    pub(in crate::settings) fn persist_session_hosts(&mut self, cx: &mut Context<Self>) -> bool {
        let entries = match self.collect_session_hosts(cx) {
            Ok(entries) => entries,
            Err(message) => {
                self.error = Some(message);
                cx.notify();
                return false;
            }
        };
        let live = crate::settings_store::SettingsStore::global(cx).user();
        let tombstones = reconcile_session_host_tombstones(
            &live.session_hosts,
            &live.session_host_tombstones,
            &entries,
            now_unix(),
        );
        self.apply_settings_patch(
            daruda_config::SettingsPatch::SessionHosts {
                entries,
                tombstones,
            },
            cx,
        )
    }
}
