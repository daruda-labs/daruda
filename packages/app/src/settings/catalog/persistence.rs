use super::*;

impl SettingsView {
    pub(in crate::settings) fn reload_agent_catalog_from_live(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let live = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();
        self.load_agent_catalog_from_config(&live, window, cx);
        cx.notify();
    }

    pub(in crate::settings) fn load_agent_catalog_from_config(
        &mut self,
        live: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folds: HashMap<String, CardFold> = self
            .agent_editable_rows()
            .map(|(_, row)| (row.id_input.read(cx).value().trim().to_string(), row.fold))
            .collect();
        let vocabulary = &self.agent_vocabulary;
        let catalog = live
            .agents
            .iter()
            .map(|entry| {
                let mut item = Self::agent_catalog_item(entry, vocabulary, window, cx);
                if let AgentCatalogItem::Editable(row) = &mut item
                    && let Some(fold) = folds.get(row.id_input.read(cx).value().trim())
                {
                    row.fold = *fold;
                }
                item
            })
            .collect::<Vec<_>>();
        for item in &catalog {
            if let AgentCatalogItem::Editable(row) = item {
                Self::subscribe_agent_row(row, window, cx, &mut self._input_subscriptions);
            }
        }
        self.agent_catalog = catalog;
        self.section_focus_targets.remove(&BuiltinSection::Agent);
        if let Some(row) = self.agent_catalog.iter().find_map(|item| match item {
            AgentCatalogItem::Editable(row) => Some(row),
            AgentCatalogItem::Unresolved(_) => None,
        }) {
            self.section_focus_targets
                .entry(BuiltinSection::Agent)
                .or_default()
                .push(row.id_input.read(cx).focus_handle(cx));
        }
    }

    /// Make each row's preset the one its saved entry references. Saving can
    /// promote a custom row whose command is a preset's, or detach a preset
    /// row onto a remote transport; the row has to say what the file now says,
    /// or the catalog would still offer that preset as unused. `origins` is
    /// one per catalog item, in order — the shape `collect_agent_catalog`
    /// returns.
    pub(in crate::settings) fn adopt_saved_agent_origins(&mut self, origins: Vec<Option<String>>) {
        for (item, origin) in self.agent_catalog.iter_mut().zip(origins) {
            if let AgentCatalogItem::Editable(row) = item {
                row.preset = origin;
            }
        }
    }

    pub(in crate::settings) fn collect_agent_catalog(
        &self,
        cx: &gpui::App,
    ) -> Result<Vec<daruda_config::AgentEntry>, SharedString> {
        let mut agents = Vec::with_capacity(self.agent_catalog.len());
        let mut seen_agent_ids = HashSet::new();
        let mut ordinal = 0usize;
        for item in &self.agent_catalog {
            let row = match item {
                AgentCatalogItem::Unresolved(entry) => {
                    agents.push(entry.clone());
                    continue;
                }
                AgentCatalogItem::Editable(row) => row,
            };
            ordinal += 1;
            let id = row.id_input.read(cx).value().trim().to_string();
            let name = row.name_input.read(cx).value().trim().to_string();
            let command = row.command_input.read(cx).value().trim().to_string();
            if id.is_empty() || name.is_empty() || command.is_empty() {
                return Err(SharedString::from(s::settings::err_agent_catalog_field(
                    ordinal,
                )));
            }
            if !is_valid_agent_id(&id) {
                return Err(SharedString::from(s::settings::err_agent_catalog_id(&id)));
            }
            if !seen_agent_ids.insert(id.clone()) {
                return Err(SharedString::from(
                    s::settings::err_agent_catalog_duplicate(&id),
                ));
            }
            let kind = row
                .transport_select
                .read(cx)
                .selected_value()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "raw".to_string());
            let host = row.host_input.read(cx).value().trim().to_string();
            let container = row.container_input.read(cx).value().trim().to_string();
            if let Some(err) = agent_row_transport_error(&kind, &host, &container) {
                return Err(agent_row_transport_message(ordinal, err));
            }
            let launch = match kind.as_str() {
                "ssh" => daruda_config::AgentLaunch::Ssh {
                    adapter_command: command,
                    host,
                },
                "docker" => daruda_config::AgentLaunch::Docker {
                    adapter_command: command,
                    container,
                },
                _ => daruda_config::AgentLaunch::Raw(command),
            };
            let env = row.stated_env(cx).map_err(|err| {
                SharedString::from(match err {
                    sections::agent_env::EnvFieldError::MalformedLine(line) => {
                        s::settings::err_agent_catalog_env(ordinal, &line)
                    }
                    sections::agent_env::EnvFieldError::UnusableName(name) => {
                        s::settings::err_agent_catalog_env_name(ordinal, &name)
                    }
                })
            })?;
            let entry = daruda_config::AgentEntry::for_definition(
                daruda_config::AgentDefinition {
                    default_mode: row.default_mode(cx),
                    default_model: row.default_model(cx),
                    fold_mode: row.fold_mode(),
                    tail_window: row.tail_window(cx),
                    tail_window_calls: row.tail_window_calls(cx),
                    display_filter: row.display_filter(),
                    env,
                    ..daruda_config::AgentDefinition::new(id, name, launch)
                },
                row.preset.as_deref(),
            );
            agents.push(if row.enabled { entry } else { entry.disabled() });
        }
        if agents.is_empty() {
            return Err(SharedString::from(s::settings::err_agent_catalog_empty()));
        }
        Ok(agents)
    }

    pub(in crate::settings) fn persist_agent_catalog(&mut self, cx: &mut Context<Self>) -> bool {
        match self.collect_agent_catalog(cx) {
            Ok(agents) => {
                let origins: Vec<Option<String>> = agents
                    .iter()
                    .map(|entry| entry.preset_id().map(str::to_string))
                    .collect();
                let saved = self
                    .apply_settings_patch(daruda_config::SettingsPatch::AgentCatalog(agents), cx);
                if saved {
                    self.adopt_saved_agent_origins(origins);
                }
                saved
            }
            Err(message) => {
                self.error = Some(message);
                cx.notify();
                false
            }
        }
    }
}
