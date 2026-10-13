//! `SettingsView`'s host-facing surface: pushes from the host's observers,
//! section focus, read accessors, and the test and capture seams.

#[cfg(feature = "screenshot")]
use crate::CardFold;
#[cfg(feature = "screenshot")]
use crate::strings as s;
use crate::{SettingsEvent, SettingsView, UpdateState, sections};
use daruda_config::BuiltinSection;
use daruda_store::observability::log_writer::LogWriter;
use daruda_ui::theme;
use gpui::{Context, IntoElement, SharedString, Window, div, prelude::*, px};

impl SettingsView {
    /// The accounts changed somewhere — this window, another one, a login —
    /// so the rows and the orchestrator's account picker follow.
    pub fn set_accounts(
        &mut self,
        accounts: daruda_store::accounts::AccountsState,
        login_busy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accounts = accounts;
        self.account_login_busy = login_busy;
        self.refresh_orchestrator_account_select(window, cx);
        cx.notify();
    }

    /// A connection advertised new modes or models; every agent row's pickers
    /// follow, including while Settings stays open.
    pub fn set_agent_vocabulary(
        &mut self,
        vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent_vocabulary = vocabulary;
        for index in 0..self.agent_catalog.len() {
            self.refresh_agent_row_vocabulary(index, window, cx);
        }
    }

    /// The plugin scope was rescanned; the installed-plugin manifest is
    /// reread with it, since an install or removal changes both.
    pub fn set_plugin_skills(
        &mut self,
        skills: Vec<daruda_agent::skills::Skill>,
        cx: &mut Context<Self>,
    ) {
        self.plugin_skills = skills;
        self.plugin_installs = sections::plugin::read_plugin_installs_indexed();
        cx.notify();
    }

    /// The self-updater moved to another step.
    pub fn set_update_state(&mut self, state: UpdateState, cx: &mut Context<Self>) {
        if self.update_state != state {
            self.update_state = state;
            cx.notify();
        }
    }

    /// New sign-in readings. They arrive after Settings opens: every probe is a
    /// background subprocess.
    pub fn set_auth_statuses(
        &mut self,
        statuses: std::collections::HashMap<
            daruda_agent::accounts::LoginTarget,
            daruda_agent::accounts::auth_status::AuthStatus,
        >,
        cx: &mut Context<Self>,
    ) {
        // The Global also changes when a probe merely starts.
        if self.auth_statuses != statuses {
            self.auth_statuses = statuses;
            cx.notify();
        }
    }

    /// The host's answer to whether it has a project to configure.
    pub fn set_project_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.project_open != open {
            self.project_open = open;
            cx.notify();
        }
    }

    /// Go to `section` from a nav row or a search result: the query is done
    /// with, so it clears and the page shows in full.
    pub(crate) fn open_section(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.sidebar_search_input.read(cx).value().is_empty() {
            self.sidebar_search_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
        }
        self.focus_section(section, window, cx);
    }

    /// Switch the active page and land focus on its first visible input.
    /// Also what `Workspace::open_settings` calls when Settings is already up.
    pub fn focus_section(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_section != section {
            self.active_section = section;
            // Reset scroll so the new section starts at the top —
            // otherwise switching from a long page leaves the new page
            // mid-scroll which is confusing.
            self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
        }
        // The page's first visible input in layout order, then whatever the
        // page draws by hand; constructor order knows nothing of either.
        if let Some(fh) = self.first_visible_input(section, cx).or_else(|| {
            self.section_focus_targets
                .get(&section)
                .and_then(|handles| handles.first())
                .cloned()
        }) {
            fh.focus(window, cx);
        } else {
            self.panel_focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn project_open(&self) -> bool {
        self.project_open
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn accounts(&self) -> &daruda_store::accounts::AccountsState {
        &self.accounts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn plugin_skills(&self) -> &[daruda_agent::skills::Skill] {
        &self.plugin_skills
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn auth_statuses(
        &self,
    ) -> &std::collections::HashMap<
        daruda_agent::accounts::LoginTarget,
        daruda_agent::accounts::auth_status::AuthStatus,
    > {
        &self.auth_statuses
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn agent_vocabulary(&self) -> &daruda_store::agent_vocabulary::AgentVocabularyCache {
        &self.agent_vocabulary
    }

    pub fn active_section(&self) -> BuiltinSection {
        self.active_section
    }

    /// Ask the host to take this view down. Committing what is in flight is
    /// the host's job — every exit it knows about (this one, and the window
    /// closing underneath) has to land the same edits, so the funnel lives
    /// there rather than being repeated per gesture.
    pub(crate) fn dismiss(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Close);
    }

    /// The banner text, for the screenshot scenario's test — the one caller
    /// outside this module, which cannot reach a private field.
    #[cfg(all(any(test, feature = "test-support"), feature = "screenshot"))]
    pub fn error_for_test(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Type a fixed query into the sidebar search — the
    /// `--screenshot-scenario settings-search` entry point. `away` lands on a
    /// parent switch, its dependent row and a folded Advanced card's rows.
    #[cfg(feature = "screenshot")]
    pub fn seed_search_for_shot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_search_input.update(cx, |input, cx| {
            input.set_value("away".to_string(), window, cx);
        });
        cx.notify();
    }

    /// Open the first catalog card and its advanced block, narrow the preset
    /// lists with a query, and scroll to them — the `--screenshot-scenario
    /// agent-catalog-expanded` entry point: every part of a card on one screen.
    #[cfg(feature = "screenshot")]
    pub fn seed_agent_catalog_for_shot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.agent_editable_rows().map(|(index, _)| index).next();
        if let Some(index) = first
            && let Some(row) = self.agent_editable_row_mut(index)
        {
            row.fold = CardFold {
                expanded: true,
                advanced: true,
            };
        }
        self.agent_catalog_search.update(cx, |input, cx| {
            input.set_value("gem".to_string(), window, cx);
        });
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    /// Open the first catalog card with one transcript editor showing — the
    /// `--screenshot-scenario agent-catalog-editor:<axis>` entry point. The
    /// fold axis is taken off the built-in in memory only, with its tool
    /// categories open, so the override pin, a hand-edited matrix and the
    /// nested rows are all on screen; nothing is written to the config.
    #[cfg(feature = "screenshot")]
    pub fn seed_agent_catalog_editor_for_shot(
        &mut self,
        axis: sections::agent_transcript::editor::EditorShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_transcript::fold_mode::{BlockRule, FoldBlock, FoldPreset, TurnPosition};
        let first = self.agent_editable_rows().map(|(index, _)| index).next();
        if let Some(index) = first
            && let Some(row) = self.agent_editable_row_mut(index)
        {
            row.fold = CardFold {
                expanded: true,
                advanced: true,
            };
            row.fold_mode = Some(FoldPreset::Summary.mode().with_rule(
                TurnPosition::Last,
                FoldBlock::Thinking,
                BlockRule::Collapsed,
            ));
            row.fold_editor.toggle_tools(TurnPosition::Last);
            row.shot_editor = Some(axis);
        }
        // The same scroll as `agent-catalog-expanded`: it puts the card's
        // transcript fields high on screen, with room below for the panel.
        self.agent_catalog_search.update(cx, |input, cx| {
            input.set_value("gem".to_string(), window, cx);
        });
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    /// Raise the failure banner with a representative message — the
    /// `--screenshot-scenario settings-error` entry point. No action is
    /// actually attempted; a capture must not depend on a write failing.
    #[cfg(feature = "screenshot")]
    pub fn seed_error_for_shot(&mut self, cx: &mut Context<Self>) {
        self.error = Some(SharedString::from(s::settings::err_telegram_unpair(
            "Permission denied (os error 13)",
        )));
        cx.notify();
    }

    /// Put a failed action on screen and in the log. The one place a Settings
    /// action reports a failure the user asked for and did not get.
    pub(crate) fn report_section_error(
        &mut self,
        text: String,
        report: daruda_store::observability::error_report::ErrorReportBuilder,
        cx: &mut Context<Self>,
    ) {
        LogWriter::log(report.build());
        self.error = Some(SharedString::from(text));
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn focus_next_input(
        &self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    }

    pub(crate) fn section_label(
        label: impl Into<gpui::SharedString>,
        cx: &gpui::App,
    ) -> impl IntoElement {
        div()
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme::current(cx).text_primary)
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(label.into())
    }
}
