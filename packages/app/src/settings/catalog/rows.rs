use super::*;

impl SettingsView {
    /// The catalog item one persisted entry stands for: an editable row when
    /// it resolves, the entry kept verbatim when it does not. The one place a
    /// row learns what the entry says beyond its definition.
    pub(in crate::settings) fn agent_catalog_item(
        entry: &daruda_config::AgentEntry,
        vocabulary: &daruda_store::agent_vocabulary::AgentVocabularyCache,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentCatalogItem {
        match entry.resolve() {
            Some(definition) => {
                let mut row = Self::agent_row_from_definition(
                    &definition,
                    vocabulary,
                    entry.preset_id().map(str::to_string),
                    window,
                    cx,
                );
                row.enabled = entry.enabled;
                AgentCatalogItem::Editable(row)
            }
            None => AgentCatalogItem::Unresolved(entry.clone()),
        }
    }

    pub(in crate::settings) fn agent_row_from_definition(
        definition: &daruda_config::AgentDefinition,
        vocabulary: &daruda_store::agent_vocabulary::AgentVocabularyCache,
        preset: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentCatalogRow {
        let id = definition.id.clone();
        let name = definition.name.clone();
        let (command, transport_kind, host, container) = match &definition.launch {
            daruda_config::AgentLaunch::Raw(command) => {
                (command.clone(), "raw", String::new(), String::new())
            }
            daruda_config::AgentLaunch::Ssh {
                adapter_command,
                host,
            } => (adapter_command.clone(), "ssh", host.clone(), String::new()),
            daruda_config::AgentLaunch::Docker {
                adapter_command,
                container,
            } => (
                adapter_command.clone(),
                "docker",
                String::new(),
                container.clone(),
            ),
        };
        let path_warning = agent_command_path_warning(&command);
        let transcript = sections::agent_transcript::transcript_row(definition, window, cx);
        let transport_kind = SharedString::from(transport_kind);
        let default_mode = SharedString::from(definition.default_mode.clone().unwrap_or_default());
        let default_model =
            SharedString::from(definition.default_model.clone().unwrap_or_default());
        let (mode_options, model_options) =
            sections::agent_vocabulary::agent_row_vocabulary_options(
                vocabulary,
                &id,
                &command,
                &default_mode,
                &default_model,
            );
        let env_field_base = preset
            .as_deref()
            .and_then(daruda_config::AgentDefinition::registry_preset)
            .and_then(|base| base.env);
        let mut row = AgentCatalogRow {
            enabled: true,
            fold: CardFold::default(),
            preset,
            env_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .auto_grow(
                        theme::palette::SETTINGS_AGENT_ENV_ROWS_MIN,
                        theme::palette::SETTINGS_AGENT_ENV_ROWS_MAX,
                    )
                    .placeholder(s::settings::agent_env_placeholder())
                    .default_value(sections::agent_env::env_field_text(
                        definition.env.as_deref(),
                    ))
            }),
            tail_window_loaded: transcript.tail_window_loaded,
            tail_window_calls_loaded: transcript.tail_window_calls_loaded,
            fold_mode: transcript.fold_mode,
            fold_mode_loaded: transcript.fold_mode_loaded,
            fold_editor: FoldEditorState::default(),
            filter_editor: FilterEditorState::default(),
            #[cfg(feature = "screenshot")]
            shot_editor: None,
            display_filter: transcript.display_filter,
            display_filter_loaded: transcript.display_filter_loaded,
            tail_window_select: transcript.tail_window_select,
            tail_window_calls_select: transcript.tail_window_calls_select,
            id_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::agent_id_placeholder())
                    .default_value(id)
            }),
            name_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::agent_name_placeholder())
                    .default_value(name)
            }),
            command_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::agent_command_placeholder())
                    .default_value(command)
            }),
            transport_select: cx.new(|cx| {
                let opts = vec![
                    SelectOption::new("raw", s::settings::agent_transport_raw()),
                    SelectOption::new("ssh", s::settings::agent_transport_ssh()),
                    SelectOption::new("docker", s::settings::agent_transport_docker()),
                ];
                select::state_with_options(opts, Some(&transport_kind), window, cx)
            }),
            host_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::session_host_target_placeholder())
                    .default_value(host)
            }),
            container_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings::session_host_container_placeholder())
                    .default_value(container)
            }),
            default_mode_select: cx.new(|cx| {
                select::state_with_options(mode_options, Some(&default_mode), window, cx)
            }),
            default_model_select: cx.new(|cx| {
                select::state_with_options(model_options, Some(&default_model), window, cx)
            }),
            path_warning,
            env_field_base,
        };
        // A row that already runs something other than its preset opens with
        // the advanced block showing, so the difference is never hidden.
        row.fold.advanced = row.advanced_overridden(cx);
        row
    }

    /// Whether the catalog holds no entries **of either kind**. The single
    /// definition validation and the section's placeholder both read, so a
    /// catalog of only non-editable entries can never be called empty by one
    /// and non-empty by the other.
    pub(in crate::settings) fn agent_catalog_is_empty(&self) -> bool {
        self.agent_catalog.is_empty()
    }

    /// Editable rows paired with their catalog index (the index
    /// [`Self::remove_agent_catalog_item`] takes — not the "Agent N" ordinal).
    pub(in crate::settings) fn agent_editable_rows(
        &self,
    ) -> impl Iterator<Item = (usize, &AgentCatalogRow)> + '_ {
        self.agent_catalog
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                AgentCatalogItem::Editable(row) => Some((index, row)),
                AgentCatalogItem::Unresolved(_) => None,
            })
    }

    /// The editable row at a catalog index, for the ops that write one field of
    /// it. `None` covers both an index past the end and an unresolved entry,
    /// which has no row to edit.
    pub(in crate::settings) fn agent_editable_row_mut(
        &mut self,
        index: usize,
    ) -> Option<&mut AgentCatalogRow> {
        match self.agent_catalog.get_mut(index)? {
            AgentCatalogItem::Editable(row) => Some(row),
            AgentCatalogItem::Unresolved(_) => None,
        }
    }

    /// Non-editable entries paired with their catalog index.
    pub(in crate::settings) fn agent_unresolved_entries(
        &self,
    ) -> impl Iterator<Item = (usize, &daruda_config::AgentEntry)> + '_ {
        self.agent_catalog
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                AgentCatalogItem::Unresolved(entry) => Some((index, entry)),
                AgentCatalogItem::Editable(_) => None,
            })
    }

    /// The `ordinal`-th editable row, counting only editable ones — the number
    /// the section shows as "Agent N" (1-based there, 0-based here). Test-only:
    /// production code walks [`Self::agent_editable_rows`], which also yields
    /// the catalog index every mutation needs.
    #[cfg(test)]
    pub(in crate::settings) fn agent_editable_row(
        &self,
        ordinal: usize,
    ) -> Option<&AgentCatalogRow> {
        self.agent_editable_rows().nth(ordinal).map(|(_, row)| row)
    }

    /// Catalog index of the row whose `command_input` is `entity`, if any. Rows
    /// are looked up by entity identity rather than a captured index because
    /// indices shift on [`Self::remove_agent_catalog_item`], and this closure is
    /// wired once per row without a stable index to close over.
    pub(in crate::settings) fn agent_row_index_by_command(
        &self,
        entity: &Entity<InputState>,
    ) -> Option<usize> {
        self.agent_editable_rows()
            .find(|(_, row)| row.command_input == *entity)
            .map(|(index, _)| index)
    }

    /// Catalog index of the row whose `id_input` is `entity` — same
    /// entity-identity lookup, and same reason, as
    /// [`Self::agent_row_index_by_command`].
    pub(in crate::settings) fn agent_row_index_by_id(
        &self,
        entity: &Entity<InputState>,
    ) -> Option<usize> {
        self.agent_editable_rows()
            .find(|(_, row)| row.id_input == *entity)
            .map(|(index, _)| index)
    }

    /// Re-run the local-PATH check for one row and store the result. The
    /// `which` lookup is I/O, so this runs from the command-change handler
    /// and construction only — never from `render`, which just reads
    /// [`AgentCatalogRow::path_warning`]. Independent of transport: the
    /// ssh/docker exemption is applied at render time instead, since it needs
    /// no I/O (see `agent.rs::render_agent_catalog_row`).
    pub(in crate::settings) fn recompute_agent_row_path_warning(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(AgentCatalogItem::Editable(row)) = self.agent_catalog.get(index) else {
            return;
        };
        let command = row.command_input.read(cx).value().to_string();
        let warning = agent_command_path_warning(&command);
        if let Some(AgentCatalogItem::Editable(row)) = self.agent_catalog.get_mut(index) {
            row.path_warning = warning;
        }
        cx.notify();
    }

    /// Append a catalog row. `preset` names the preset `definition` came from
    /// (one switched on from the catalog), so the saved entry references it
    /// rather than copying its fields; `None` adds a custom row.
    pub(in crate::settings) fn add_agent_row(
        &mut self,
        definition: daruda_config::AgentDefinition,
        preset: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = Self::agent_row_from_definition(
            &definition,
            &self.agent_vocabulary,
            preset,
            window,
            cx,
        );
        Self::subscribe_agent_row(&row, window, cx, &mut self._input_subscriptions);
        self.agent_catalog.push(AgentCatalogItem::Editable(row));
        self.error = None;
        if self.collect_agent_catalog(cx).is_ok() {
            self.persist_agent_catalog(cx);
        }
        cx.notify();
    }

    /// Drop the catalog entry at `index`, whichever kind it is — a non-editable
    /// entry is removable for the same reason an editable one is: the user's
    /// only alternative is hand-editing `config.toml`.
    pub(in crate::settings) fn remove_agent_catalog_item(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index < self.agent_catalog.len() {
            let removed = self.agent_catalog.remove(index);
            self.error = None;
            if !self.persist_agent_catalog(cx) {
                self.agent_catalog.insert(index, removed);
            }
            cx.notify();
        }
    }
}
