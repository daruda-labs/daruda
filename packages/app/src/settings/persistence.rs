//! Draft commits, external changes, and save conflict handling.

use super::*;

impl SettingsView {
    /// Commit every text input holding something other than what the live
    /// config shows, so an exit taken mid-edit does not drop it. Returns
    /// whether the exit may proceed.
    ///
    /// Text settings persist on Enter or Blur only, so an input that still
    /// holds focus has never been written. The `show` comparison is what
    /// keeps an untouched field from re-writing itself —
    /// [`Self::persist_text_setting`] applies unconditionally.
    ///
    /// The two refusals are not the same and are not treated the same. A
    /// value that cannot parse was never usable, and a field that moved
    /// underneath this view resolves to the external value — the same answer
    /// [`Self::sync_external_settings`] picks when there is no local draft —
    /// so both revert and the exit continues. A *write* that failed is
    /// neither: the value is still what the user asked for and the banner
    /// naming the failure is the only place it is said, so the edit stays in
    /// the field and `false` holds the view open around it.
    pub(crate) fn commit_pending_edits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut blocked = false;
        for spec in spec::TEXT_SETTINGS {
            // Re-read per row: a landed patch moves the global underneath us.
            let live = crate::settings_store::SettingsStore::global(cx)
                .user()
                .clone();
            let shown = (spec.show)(&live);
            let input = (spec.field)(self).clone();
            if input.read(cx).value().trim() == shown {
                continue;
            }
            match (spec.parse)(&input, cx) {
                Err(_) => {
                    self.error = None;
                    self.conflict = None;
                    Self::set_input_value(&input, shown, window, cx);
                }
                Ok(patch) => {
                    if self.apply_settings_patch(patch, cx) {
                        continue;
                    }
                    // A refusal is a conflict or a failed write; only the
                    // first one reverts. `apply_settings_patch` sets exactly
                    // one of the two, so this tells them apart.
                    if self.conflict.take().is_some() {
                        self.error = None;
                        Self::set_input_value(&input, shown, window, cx);
                    } else {
                        blocked = true;
                    }
                }
            }
        }
        !blocked
    }

    /// Commit one field's change, refusing it when the same field moved
    /// underneath the window since it opened. Returns whether the write landed.
    pub(super) fn apply_settings_patch(
        &mut self,
        patch: daruda_config::SettingsPatch,
        cx: &mut Context<Self>,
    ) -> bool {
        use gpui::BorrowAppContext as _;

        let baseline = self.base_config.clone();
        let committed_patch = patch.clone();
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.apply_patch_if_unchanged(patch, &baseline)
        });
        match result {
            Ok(()) => {
                self.advance_base_field_from_live(&committed_patch, cx);
                self.error = None;
                self.conflict = None;
                cx.notify();
                true
            }
            Err(daruda_config::SettingsPatchApplyError::Conflict(_)) => {
                self.error = None;
                self.conflict = Some(committed_patch);
                cx.notify();
                false
            }
            Err(daruda_config::SettingsPatchApplyError::Persistence(message)) => {
                self.report_save_failure(
                    committed_patch.field(),
                    &message,
                    |e| s::settings::err_save_settings(e),
                    cx,
                );
                false
            }
        }
    }

    pub(super) fn apply_settings_patch_force(
        &mut self,
        patch: daruda_config::SettingsPatch,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_settings_patch_force_as(patch, |e| s::settings::err_save_settings(e), cx)
    }

    /// [`Self::apply_settings_patch_force`] with the banner sentence chosen by
    /// the caller. A button that means one concrete thing to the user ("Unpair")
    /// says so when it fails, instead of reporting a generic failed save.
    pub(super) fn apply_settings_patch_force_as(
        &mut self,
        patch: daruda_config::SettingsPatch,
        describe: fn(&str) -> String,
        cx: &mut Context<Self>,
    ) -> bool {
        use gpui::BorrowAppContext as _;
        let committed_patch = patch.clone();
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.apply_patch(patch)
        });
        match result {
            Ok(()) => {
                self.advance_base_field_from_live(&committed_patch, cx);
                self.error = None;
                self.conflict = None;
                cx.notify();
                true
            }
            Err(message) => {
                self.report_save_failure(committed_patch.field(), &message, describe, cx);
                false
            }
        }
    }

    /// A settings write that did not land. Banner plus log entry, always —
    /// the banner alone loses the diagnostic, and the log alone is the silent
    /// failure this window kept shipping.
    pub(super) fn report_save_failure(
        &mut self,
        field: daruda_config::SettingsFieldId,
        message: &str,
        describe: fn(&str) -> String,
        cx: &mut Context<Self>,
    ) {
        self.report_section_error(
            describe(message),
            ErrorReport::new("Settings write failed")
                .severity(ErrorSeverity::Warning)
                .message(message)
                .at(file!(), line!())
                .with_context("field", field.path())
                .dedup("settings.write"),
            cx,
        );
    }

    pub(super) fn advance_base_field_from_live(
        &mut self,
        patch: &daruda_config::SettingsPatch,
        cx: &gpui::App,
    ) {
        let live = crate::settings_store::SettingsStore::global(cx).user();
        if let Some(live_patch) = Self::settings_ui_patches(live)
            .into_iter()
            .find(|candidate| candidate.field() == patch.field())
        {
            live_patch.apply_to(&mut self.base_config);
        }
    }

    pub(super) fn overwrite_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(patch) = self.conflict.clone()
            && self.apply_settings_patch_force(patch.clone(), cx)
        {
            let live = crate::settings_store::SettingsStore::global(cx)
                .user()
                .clone();
            self.load_settings_patch(&patch, &live, window, cx);
            cx.notify();
        }
    }

    pub(super) fn reload_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(patch) = self.conflict.take() else {
            return;
        };
        let live = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();
        self.load_settings_patch(&patch, &live, window, cx);
        if let Some(live_patch) = Self::settings_ui_patches(&live)
            .into_iter()
            .find(|candidate| candidate.field() == patch.field())
        {
            live_patch.apply_to(&mut self.base_config);
        }
        self.error = None;
        self.sync_external_settings(window, cx);
        cx.notify();
    }

    /// Refresh the window's widgets after `patch`'s field changed underneath it
    /// (another window saved, or `config.toml` was edited on disk).
    ///
    /// Table-driven for every simple setting: the row that owns the field says
    /// how to read the new value and, for selects, whether the option list has
    /// to be rebuilt with it. Only the editors whose widgets are whole
    /// sub-forms are refreshed by hand below.
    pub(super) fn load_settings_patch(
        &mut self,
        patch: &daruda_config::SettingsPatch,
        live: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = patch.field();

        if let Some(spec) = spec::TEXT_SETTINGS
            .iter()
            .find(|spec| spec::text_field_id(spec) == field)
        {
            Self::set_input_value((spec.field)(self), (spec.show)(live), window, cx);
            return;
        }
        if let Some(spec) = spec::SELECT_SETTINGS
            .iter()
            .find(|spec| spec::select_field_id(spec) == field)
        {
            let select = (spec.field)(self).clone();
            match spec.load {
                spec::SelectLoad::Value => {
                    Self::set_select_value(&select, (spec.show)(live), window, cx);
                }
                spec::SelectLoad::Font => {
                    let value = (spec.show)(live);
                    Self::set_font_select_value(&select, value.clone(), &[&value], window, cx);
                }
                spec::SelectLoad::Rebuild(refresh) => refresh(self, window, cx),
            }
            return;
        }
        if let Some(spec) = spec::BOOL_SETTINGS
            .iter()
            .find(|spec| spec::bool_field_id(spec) == field)
        {
            (spec.set)(self, (spec.show)(live));
            return;
        }

        match patch {
            daruda_config::SettingsPatch::AgentCatalog(_) => {
                self.refresh_orchestrator_agent_select(window, cx);
                self.load_agent_catalog_from_config(live, window, cx);
            }
            daruda_config::SettingsPatch::SessionHosts { .. } => {
                let rows = live
                    .session_hosts
                    .iter()
                    .map(|entry| Self::session_host_row_from_entry(entry, window, cx))
                    .collect::<Vec<_>>();
                for row in &rows {
                    Self::subscribe_session_host_row(
                        row,
                        window,
                        cx,
                        &mut self._input_subscriptions,
                    );
                }
                self.session_host_rows = rows;
                self.section_focus_targets
                    .remove(&BuiltinSection::SessionHosts);
                if let Some(row) = self.session_host_rows.first() {
                    self.section_focus_targets
                        .entry(BuiltinSection::SessionHosts)
                        .or_default()
                        .push(row.label_input.read(cx).focus_handle(cx));
                }
            }
            // No editor row to reload: a toggle is the status bar menu's own
            // gesture (Settings writes the list through `StatusBarHiddenItems`),
            // and the Telegram chat id is owned by pairing —
            // `adopt_external_settings` mirrors that one instead.
            daruda_config::SettingsPatch::ToggleStatusBarItem(_)
            | daruda_config::SettingsPatch::TelegramAuthorizedChatId(_) => {}
            // Every remaining variant is covered by a `spec` row above.
            _ => {}
        }
    }

    /// Every config field this window owns, as patches carrying `config`'s
    /// current values — the list `sync_external_settings` diffs to find what
    /// changed underneath the window.
    ///
    /// Three entries are not `spec` rows because their editors are whole
    /// sub-forms rather than a single widget. `TelegramAuthorizedChatId` is
    /// absent because pairing owns it and there is no editor to reload into;
    /// [`Self::adopt_external_settings`] mirrors it instead.
    pub(super) fn settings_ui_patches(
        config: &daruda_config::Config,
    ) -> Vec<daruda_config::SettingsPatch> {
        let mut patches: Vec<daruda_config::SettingsPatch> = Vec::with_capacity(
            spec::TEXT_SETTINGS.len() + spec::SELECT_SETTINGS.len() + spec::BOOL_SETTINGS.len() + 3,
        );
        patches.extend(
            spec::TEXT_SETTINGS
                .iter()
                .map(|spec| (spec.current)(config)),
        );
        patches.extend(
            spec::SELECT_SETTINGS
                .iter()
                .map(|spec| (spec.current)(config)),
        );
        patches.extend(
            spec::BOOL_SETTINGS
                .iter()
                .map(|spec| (spec.patch)((spec.show)(config))),
        );
        patches.push(daruda_config::SettingsPatch::AgentCatalog(
            config.agents.clone(),
        ));
        patches.push(daruda_config::SettingsPatch::StatusBarHiddenItems(
            config.status_bar.hidden_items.clone(),
        ));
        patches.push(daruda_config::SettingsPatch::SessionHosts {
            entries: config.session_hosts.clone(),
            tombstones: config.session_host_tombstones.clone(),
        });
        patches
    }

    /// Catch up with a settings change made outside this window.
    ///
    /// The pairing is mirrored separately because the field-diff below only
    /// walks [`Self::settings_ui_patches`], which it is not part of.
    pub(super) fn adopt_external_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let live_chat_id = crate::settings_store::SettingsStore::global(cx)
            .user()
            .telegram
            .authorized_chat_id;
        if self.telegram_authorized_chat_id != live_chat_id {
            self.telegram_authorized_chat_id = live_chat_id;
            cx.notify();
        }
        self.sync_external_settings(window, cx);
    }

    pub(super) fn sync_external_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let live = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();
        let changed_patches = Self::settings_ui_patches(&live)
            .iter()
            .filter(|patch| patch.field_changed_between(&self.base_config, &live))
            .cloned()
            .collect::<Vec<_>>();
        if changed_patches.is_empty() {
            return;
        }

        let Ok(draft) = self.validate(cx) else {
            cx.notify();
            return;
        };
        let has_local_draft = Self::settings_ui_patches(&self.base_config)
            .iter()
            .any(|patch| patch.field_changed_between(&self.base_config, &draft));
        if has_local_draft || self.conflict.is_some() {
            cx.notify();
            return;
        }

        // Reuse the per-field loader so structural editors rebuild their
        // subscriptions and focus handles through the same path as the
        // explicit "Use external value" action.
        for patch in changed_patches {
            self.load_settings_patch(&patch, &live, window, cx);
        }
        self.base_config = live;
        self.error = None;
        self.conflict = None;
        cx.notify();
    }

    /// One select's chosen option, persisted. The option→patch mapping lives in
    /// [`spec::SELECT_SETTINGS`]; a value the row does not recognise is ignored
    /// rather than guessed at.
    pub(super) fn persist_select_setting(
        &mut self,
        select: &Entity<SelectState>,
        setting: SelectSetting,
        cx: &mut Context<Self>,
    ) {
        let Some(value) = select
            .read(cx)
            .selected_value()
            .map(|value| value.to_string())
        else {
            return;
        };
        let Some(patch) = (spec::select_spec(setting).read)(&value) else {
            return;
        };
        self.apply_settings_patch(patch, cx);
    }

    /// One switch's new state, persisted. Returns whether the write landed —
    /// the caller mirrors the value onto its own field only then.
    pub(crate) fn persist_bool_setting(
        &mut self,
        setting: BoolSetting,
        value: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_settings_patch((spec::bool_spec(setting).patch)(value), cx)
    }

    /// One text input's contents, persisted. The bounds and the message shown
    /// when they are missed both come from the field's own row in
    /// [`spec::TEXT_SETTINGS`], which is also what `validate` reads — so a live
    /// edit and a whole-config draft cannot disagree about what is valid.
    pub(super) fn persist_text_setting(
        &mut self,
        input: &Entity<InputState>,
        setting: TextSetting,
        cx: &mut Context<Self>,
    ) {
        match (spec::text_spec(setting).parse)(input, cx) {
            Ok(patch) => {
                self.apply_settings_patch(patch, cx);
            }
            Err(message) => {
                self.error = Some(message);
                cx.notify();
            }
        }
    }

    /// The whole form as a `Config`, or the first field-level error.
    ///
    /// Every simple setting goes through its own row in `spec` — the same rows
    /// `persist_*_setting` uses for a live edit — so the draft and the live edit
    /// enforce one set of bounds and produce one set of values. Collected by
    /// hand below: the two sub-form editors, whose validation the row shape
    /// cannot express, and `telegram.authorized_chat_id`, which pairing owns.
    pub(super) fn validate(&self, cx: &gpui::App) -> Result<daruda_config::Config, SharedString> {
        // Start from the snapshot taken at window-open time so fields not
        // exposed in the UI (e.g. [colors], [keybindings]) are preserved.
        let mut config = self.base_config.clone();

        for spec in spec::TEXT_SETTINGS {
            (spec.parse)((spec.field)(self), cx)?.apply_to(&mut config);
        }
        for spec in spec::SELECT_SETTINGS {
            let selected = (spec.field)(self)
                .read(cx)
                .selected_value()
                .and_then(|value| (spec.read)(value));
            // A widget with nothing selected leaves the field as the open-time
            // snapshot had it rather than inventing a value.
            selected
                .unwrap_or_else(|| (spec.current)(&config))
                .apply_to(&mut config);
        }
        for spec in spec::BOOL_SETTINGS {
            (spec.patch)((spec.get)(self)).apply_to(&mut config);
        }
        // `max_fps` is the one select whose accepted range is not the option
        // list: a config edited by hand can carry anything, so re-clamp.
        config.render.clamp();

        config.agents = self.collect_agent_catalog(cx)?;

        // Session host registry: `label` must be unique across the whole
        // catalog (trim + case-insensitive) — two rows saved with the same
        // display label would leave a lane's "which host is this?" picker
        // unable to tell them apart. `target`/`container` go through the
        // exact same bare-word check `SessionHostModal` uses, so a value
        // that would break `wrap`'s shell quoting is rejected here too.
        let session_hosts =
            self.collect_session_hosts_against(&self.base_config.session_hosts, false, cx)?;
        config.session_host_tombstones = reconcile_session_host_tombstones(
            &self.base_config.session_hosts,
            &self.base_config.session_host_tombstones,
            &session_hosts,
            now_unix(),
        );
        config.session_hosts = session_hosts;

        // `authorized_chat_id` is managed asynchronously by pairing/unpairing.
        // Re-read it here so draft detection never treats a completed pairing
        // as a local form edit.
        config.telegram.authorized_chat_id = crate::settings_store::SettingsStore::global(cx)
            .user_arc()
            .telegram
            .authorized_chat_id;
        config.remote = crate::settings_store::SettingsStore::global(cx)
            .user()
            .remote
            .clone();

        Ok(config)
    }
}
