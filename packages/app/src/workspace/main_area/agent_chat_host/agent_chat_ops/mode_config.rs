//! Agent mode / config-option switching and the advertised mode and model
//! vocabulary recorded per agent.

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_store::agent_vocabulary::VocabEntry;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::Context;

/// Which vocabulary axes one ACP event carries. `None` means "this event says
/// nothing about that axis", which is distinct from an empty list ("the agent
/// advertised none") — only the latter clears the stored vocabulary.
struct Advertised {
    modes: Option<Vec<VocabEntry>>,
    models: Option<Vec<VocabEntry>>,
}

/// The mode axis as advertised at connect. Empty for an agent without modes.
fn mode_vocabulary(modes: Option<&daruda_acp::ModeStateView>) -> Vec<VocabEntry> {
    modes
        .map(|m| {
            m.available
                .iter()
                .map(|mode| VocabEntry::new(mode.id.clone(), mode.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The model axis as a vocabulary list. Empty for an agent with no model select.
fn model_vocabulary(options: &[daruda_acp::ConfigOptionView]) -> Vec<VocabEntry> {
    daruda_chat::session_config::model_select(options)
        .map(|(_, _, choices)| {
            choices
                .iter()
                .map(|c| {
                    VocabEntry::new(c.value.clone(), c.name.clone())
                        .with_description(c.description.clone())
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Workspace {
    /// Record what this pane's agent just advertised into the persisted
    /// vocabulary cache, so option pickers offer the live agent's own lists.
    /// Called from the event pump BEFORE the event folds into the view.
    ///
    /// `Connected` carries both axes; later model/option changes split them:
    /// `ModeChanged` carries the reconciled mode list and
    /// `ConfigOptionsChanged` carries models. Writes only when a list actually
    /// changed, so a reconnect that advertises the same vocabulary does no I/O.
    pub(in crate::workspace) fn record_agent_vocabulary(
        &mut self,
        pane_id: PaneId,
        event: &daruda_acp::AcpEvent,
        cx: &mut Context<Self>,
    ) {
        let advertised = match event {
            daruda_acp::AcpEvent::Connected {
                modes,
                config_options,
                ..
            } => Advertised {
                modes: Some(mode_vocabulary(modes.as_ref())),
                models: Some(model_vocabulary(config_options)),
            },
            daruda_acp::AcpEvent::ConfigOptionsChanged(options) => Advertised {
                modes: None,
                models: Some(model_vocabulary(options)),
            },
            daruda_acp::AcpEvent::ModeChanged { state } => Advertised {
                modes: Some(mode_vocabulary(Some(state))),
                models: None,
            },
            _ => return,
        };
        let Some((agent_id, frozen_source)) = self.agent_chat_view(pane_id).map(|v| {
            let view = v.read(cx);
            (
                view.agent_id().to_owned(),
                view.agent_vocabulary_source().map(str::to_owned),
            )
        }) else {
            return;
        };
        let source = frozen_source.or_else(|| {
            self.mirrors
                .agents
                .iter()
                .find(|agent| agent.id == agent_id)
                .map(|agent| {
                    crate::lane::session_host::adapter_command(&agent.launch)
                        .trim()
                        .to_string()
                })
        });
        let Some(source) = source else {
            return;
        };

        // A background observation, not a user action: a failed write costs the
        // next launch a stale picker list, so it logs instead of toasting.
        let data_dir = self.data_dir.clone();
        if let Err(e) = crate::workspace::agent_vocabulary_global::record(
            cx,
            &data_dir,
            &agent_id,
            &source,
            advertised.modes,
            advertised.models,
        ) {
            LogWriter::log(
                ErrorReport::new("Failed to save agent_vocabulary.json")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context(
                        "path",
                        daruda_store::observability::system_info::redact_home(
                            daruda_store::agent_vocabulary::agent_vocabulary_path_in(
                                &self.data_dir,
                            ),
                        ),
                    )
                    .with_context("agent_id", agent_id)
                    .with_context("phase", "agent.connect")
                    .dedup("agent_vocabulary.save")
                    .build(),
            );
        }
        self.agent_vocabulary = crate::workspace::agent_vocabulary_global::snapshot(cx, &data_dir);
    }

    /// Switch the active session mode. Shim for the mode chip: routes the
    /// chosen id into the view, which optimistically updates and sends
    /// `session/set_mode`. No-op when `pane_id` isn't an Agent chat pane.
    pub(in crate::workspace) fn set_agent_mode(
        &mut self,
        pane_id: PaneId,
        mode_id: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.set_mode(mode_id, cx));
        }
        // The bottom-input placeholder includes the current mode name;
        // refresh it now that the mode has changed.
        self.refresh_terminal_input_placeholder(cx);
        // Persist the new `picked_mode_id` so the next connect requests it
        // (see that field's doc).
        self.mutate_durable(cx, |_, _| {});
    }

    /// Change a config option (model / effort / a boolean toggle / …) because
    /// the *user* asked for it. Shim for the config chips: routes
    /// `(config_id, value)` into the view, which optimistically updates and
    /// sends `session/set_config_option`. The only path that can move
    /// `picked_model_id`, so it is also the only place that saves it.
    pub(in crate::workspace) fn set_agent_config_option(
        &mut self,
        pane_id: PaneId,
        config_id: String,
        value: daruda_acp::ConfigValueView,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let model_before = view.read(cx).picked_model_id().map(str::to_owned);
        view.update(cx, |v, cx| v.set_config_option(config_id, value, cx));
        // Only a `Model`-category pick moves `picked_model_id`; persist it
        // so the next connect starts on it (see that field's doc).
        if view.read(cx).picked_model_id() != model_before.as_deref() {
            self.mutate_durable(cx, |_, _| {});
        }
    }

    /// Advance to the next advertised session mode, wrapping at the end. Backs
    /// the Shift+Tab shortcut. `false` (no switch) when not an Agent chat pane
    /// or fewer than two modes advertised — the caller then lets it outdent.
    pub(in crate::workspace) fn cycle_agent_mode(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return false;
        };
        let Some(next) = view.read(cx).session_config().next_mode_id() else {
            return false;
        };
        view.update(cx, |v, cx| v.set_mode(next, cx));
        // The bottom-input placeholder includes the current mode name;
        // refresh it now that the mode has cycled.
        self.refresh_terminal_input_placeholder(cx);
        // Persist the new `picked_mode_id` so the next connect requests it
        // (see that field's doc).
        self.mutate_durable(cx, |_, _| {});
        true
    }
}
