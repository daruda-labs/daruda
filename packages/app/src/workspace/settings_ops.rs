//! Opening and closing the Settings view this window hosts.
//!
//! Settings is a body-level surface, not a pane: while it is up it replaces
//! `body_layout` wholesale, so the docks and the tab strip are not on screen
//! and the workspace actions that drive them are not registered (see
//! `render/mod.rs`). The tab and pane entities behind it stay alive untouched,
//! which is why closing needs no record of "where the user came from" — there
//! is only one place to come back to.

use daruda_config::BuiltinSection;
use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};

use crate::agent::skills::global::GlobalSkills;
use crate::window_registry::WindowRegistry;
use crate::workspace::{
    OpenProjectConfig, Workspace, accounts_global, agent_vocabulary_global, auth_status_global,
    settings_update,
};
use daruda_settings::{
    LoginRequest, SettingsEvent, SettingsServices, SettingsView, SharedSnapshot,
};

/// The Settings view a window is showing, with the subscriptions carrying its
/// requests back and the shared state in. All are built and dropped together,
/// so they are one value rather than fields that must agree.
pub(in crate::workspace) struct SettingsHost {
    view: Entity<SettingsView>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// Show Settings on `section`, or move the open one to it.
    pub(in crate::workspace) fn open_settings(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = match self.settings.as_ref() {
            Some(host) => host.view.clone(),
            None => {
                let shared = SharedSnapshot {
                    accounts: accounts_global::snapshot(cx),
                    account_login_busy: accounts_global::login_busy(cx),
                    auth_statuses: auth_status_global::snapshot(cx),
                    agent_vocabulary: agent_vocabulary_global::snapshot(cx, &self.data_dir),
                    plugin_skills: plugin_skills(cx),
                    status_icon_themes: crate::status_icon_themes::current(cx).choices(),
                    update_state: settings_update::update_state(cx),
                };
                let remote_integrations: gpui::AnyView = cx
                    .new(|cx| crate::remote_channel::settings::ChannelSettings::new(window, cx))
                    .into();
                let view = cx.new(|cx| {
                    SettingsView::new_with_section(
                        section,
                        shared,
                        std::rc::Rc::new(HostServices),
                        remote_integrations,
                        window,
                        cx,
                    )
                });
                let events = cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &SettingsEvent, window, cx| match event {
                        SettingsEvent::Close => this.close_settings(window, cx),
                        SettingsEvent::Login(request) => {
                            this.run_settings_login(request, window, cx)
                        }
                        SettingsEvent::OpenProjectConfig => {
                            this.on_open_project_config(&OpenProjectConfig, window, cx)
                        }
                        SettingsEvent::AccountsChanged(state) => {
                            accounts_global::replace(cx, state.clone())
                        }
                        SettingsEvent::OpenStatusIconThemes => {
                            this.open_status_icon_theme_folder(cx)
                        }
                        SettingsEvent::PluginsChanged => rescan_plugins(cx),
                        SettingsEvent::Update(command) => settings_update::run(*command, cx),
                    },
                );
                self.settings = Some(SettingsHost {
                    view: view.clone(),
                    _subscriptions: [
                        events,
                        cx.observe_global_in::<accounts_global::AccountsGlobal>(
                            window,
                            Self::push_settings_accounts,
                        ),
                        cx.observe_global_in::<auth_status_global::AuthStatusGlobal>(
                            window,
                            Self::push_settings_auth_statuses,
                        ),
                        cx.observe_global_in::<agent_vocabulary_global::AgentVocabularyGlobal>(
                            window,
                            Self::push_settings_vocabulary,
                        ),
                        cx.observe_global::<GlobalSkills>(Self::push_settings_plugin_skills),
                        cx.observe_global_in::<crate::status_icon_themes::StatusIconThemes>(
                            window,
                            Self::push_settings_status_icon_themes,
                        ),
                    ]
                    .into_iter()
                    .chain(
                        crate::update::Updater::get(cx)
                            .map(|updater| cx.observe(&updater, Self::push_settings_update)),
                    )
                    .collect(),
                });
                view
            }
        };
        // Both paths land focus, and that is the whole contract of the swap:
        // the pane that had it is no longer rendered, and gpui resolves an
        // unrendered focus id to the root dispatch node — which carries
        // neither the view's key handler nor this window's actions. Skipping
        // it on the build path left Escape and Cmd+W inert until the user
        // clicked something.
        view.update(cx, |view, cx| view.focus_section(section, window, cx));
        self.sync_settings_project(cx);
        cx.notify();
    }

    /// Tell an open Settings whether this window has a project to configure.
    /// Called wherever that answer can change: opening Settings, adding a
    /// project, and falling back to the empty workspace.
    pub(in crate::workspace) fn sync_settings_project(&self, cx: &mut Context<Self>) {
        let Some(host) = self.settings.as_ref() else {
            return;
        };
        let open = self.active_project().is_some();
        host.view
            .update(cx, |view, cx| view.set_project_open(open, cx));
    }

    /// Take Settings down and hand focus back to the workspace.
    ///
    /// Refused when a pending edit could not be written: that value and the
    /// banner naming the failure both live in the view, so taking it down
    /// would be the silent drop [`SettingsView::commit_pending_edits`] exists
    /// to prevent.
    pub(in crate::workspace) fn close_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_settings_edits(window, cx) {
            cx.notify();
            return;
        }
        if self.settings.take().is_none() {
            return;
        }
        // Focus has to land on something this window still renders. A
        // workspace holding no projects has no pane to return to, so the root
        // takes it — leaving the window pointed at the view just dropped kills
        // the keyboard exactly the way an unfocused open did.
        let pane_id = self.active_runtime().focused_pane_id;
        if self.active_runtime().panes.iter().any(|p| p.id == pane_id) {
            self.focus_pane(pane_id, window, cx);
        } else {
            self.window_runtime.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// Land every pending Settings edit, whatever is taking the view away.
    /// The one funnel: the back button, Escape and the window closing
    /// underneath all pass through here, so no exit can be added that skips
    /// the commit. `false` means a write failed and the view must stay up.
    pub(in crate::workspace) fn commit_settings_edits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(host) = self.settings.as_ref() else {
            return true;
        };
        let view = host.view.clone();
        view.update(cx, |view, cx| view.commit_pending_edits(window, cx))
    }

    /// Run a login an account row asked for. The view has no business spawning
    /// one — the login command comes from this window's agent catalog, and this
    /// window keeps the process handle and the Cancel that goes with it.
    fn run_settings_login(
        &mut self,
        request: &LoginRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match *request {
            LoginRequest::AddAccount(recipe) => self.add_managed_account(recipe, window, cx),
            LoginRequest::Reauthenticate(account) => self.reauthenticate_account(account, cx),
            LoginRequest::ReauthenticateSystem(recipe) => self.reauthenticate_system(recipe, cx),
        }
    }

    fn push_settings_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.settings.as_ref() else {
            return;
        };
        let accounts = accounts_global::snapshot(cx);
        let busy = accounts_global::login_busy(cx);
        host.view
            .update(cx, |view, cx| view.set_accounts(accounts, busy, window, cx));
    }

    fn push_settings_auth_statuses(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.settings.as_ref() else {
            return;
        };
        let statuses = auth_status_global::snapshot(cx);
        host.view
            .update(cx, |view, cx| view.set_auth_statuses(statuses, cx));
    }

    fn push_settings_vocabulary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.settings.as_ref() else {
            return;
        };
        let vocabulary = agent_vocabulary_global::snapshot(cx, &self.data_dir);
        host.view.update(cx, |view, cx| {
            view.set_agent_vocabulary(vocabulary, window, cx)
        });
    }

    fn push_settings_update(
        &mut self,
        _: gpui::Entity<crate::update::Updater>,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.settings.as_ref() else {
            return;
        };
        let state = settings_update::update_state(cx);
        host.view
            .update(cx, |view, cx| view.set_update_state(state, cx));
    }

    fn open_status_icon_theme_folder(&mut self, cx: &mut Context<Self>) {
        let path = daruda_store::persistence::status_icon_themes_dir();
        match std::fs::create_dir_all(&path) {
            Ok(()) => self.open_path_with_system_default(path, cx),
            Err(error) => self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::error::status_icon_theme_folder(),
                )
                .from_error(&error)
                .at(file!(), line!())
                .build(),
                cx,
            ),
        }
    }

    fn push_settings_status_icon_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = &self.settings else {
            return;
        };
        let choices = crate::status_icon_themes::current(cx).choices();
        host.view.update(cx, |view, cx| {
            view.set_status_icon_themes(choices, window, cx)
        });
    }

    fn push_settings_plugin_skills(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.settings.as_ref() else {
            return;
        };
        let skills = plugin_skills(cx);
        host.view
            .update(cx, |view, cx| view.set_plugin_skills(skills, cx));
    }

    /// Whether Settings is on screen — for tests; `render` matches on
    /// `settings_view` because it needs the entity too.
    #[cfg(test)]
    pub(in crate::workspace) fn settings_is_open(&self) -> bool {
        self.settings.is_some()
    }

    /// The view on screen, for the two callers that legitimately need the
    /// entity itself: `render`, to embed it, and the screenshot driver, to
    /// seed a state no settled workspace can reach. Everything else goes
    /// through the operations above — reaching in is how `Workspace` grew 82
    /// fields its collaborators could write.
    pub(in crate::workspace) fn settings_view(&self) -> Option<&Entity<SettingsView>> {
        self.settings.as_ref().map(|host| &host.view)
    }
}

/// What Settings asks of the app: window-wide counts and the Telegram
/// credential and bridge, none of which a view can reach on its own.
struct HostServices;

impl SettingsServices for HostServices {
    fn account_usage(&self, account: daruda_store::accounts::AccountId, cx: &mut App) -> usize {
        let mut referencing = 0;
        WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
            referencing += ws.panes_referencing_account(account);
        });
        referencing
    }

    fn telegram_token_stored(&self) -> bool {
        crate::telegram::keychain::read_token().is_some()
    }

    fn store_telegram_token(&self, token: &str) -> std::io::Result<()> {
        crate::telegram::keychain::write_token(token)
    }

    fn forget_telegram_token(&self) -> std::io::Result<()> {
        crate::telegram::keychain::delete_token()
    }

    fn telegram_bot_held_elsewhere(&self, cx: &App) -> bool {
        crate::telegram::global::TelegramBridge::bot_held_elsewhere(cx)
    }

    fn new_telegram_pair_code(&self, cx: &mut App) -> String {
        crate::telegram::global::TelegramBridge::generate_pair_code(cx)
    }
}

fn plugin_skills(cx: &App) -> Vec<crate::agent::skills::Skill> {
    cx.try_global::<GlobalSkills>()
        .map(|skills| skills.plugin.clone())
        .unwrap_or_default()
}

/// Rescan the plugin scope into the app-wide skills; every window's Skills
/// tab and any open Settings follow from the Global.
fn rescan_plugins(cx: &mut App) {
    use gpui::BorrowAppContext as _;
    let personal = crate::agent::skills::skills_personal_dir();
    crate::agent::skills::global::init(cx);
    cx.update_global::<GlobalSkills, _>(|state, _| {
        state.reload_scope(crate::agent::skills::SkillScope::Plugin, None, &personal);
    });
}
