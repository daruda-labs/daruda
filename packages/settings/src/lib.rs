//! The settings surface for common daruda config options.
//!
//! Hosted by a `Workspace` in place of its body, one per window; re-opening an
//! already-open one routes through [`SettingsView::focus_section`] instead of
//! building a second. Two windows can therefore show this at once:
//! `SettingsView::sync_external_settings` keeps them coherent by adopting an
//! outside change only when there is no local draft, so a view holding one
//! shows a stale value until that draft clears — and a stale re-commit is
//! refused as a conflict rather than overwriting. Builtin sections pair a `BuiltinSection` variant with
//! nav/header strings and a `render_<section>` method. The view draws no
//! chrome of its own — the host window owns the title bar — and asks to be
//! dismissed by emitting [`SettingsEvent::Close`] rather than acting on the
//! window itself.

rust_i18n::i18n!("locales", fallback = "en");

mod bindings;
mod catalog;
mod confirm;
mod copy;
mod host_api;
mod hosts;
mod initialize;
pub mod labels;
mod layout;
mod navigation;
mod page;
mod persistence;
mod presentation;
mod render;
mod reset;
mod search;
mod sections;
mod setting_kind;
mod spec;
pub mod store;
mod strings;
mod update_state;

pub use update_state::{UpdateCommand, UpdateInstall, UpdateState};

#[cfg(test)]
mod tests;

use catalog::{AgentCatalogRow, CardFold};
use hosts::{SessionHostRow, reconcile_session_host_tombstones};
use setting_kind::{
    BoolSetting, ICON_COLOR, ICON_MONOCHROME, SelectSetting, TextSetting, icon_color_from_value,
    icon_color_value,
};
use std::collections::{HashMap, HashSet};

use daruda_config::BuiltinSection;
use daruda_ui::theme;
#[cfg(test)]
use gpui::px;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable as _, SharedString, Subscription, Task,
    Window, prelude::*,
};

use crate::strings as s;
use daruda_project::lane::session_host;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_transcript::display_filter::DisplayFilter;
use daruda_transcript::editor::state::{FilterEditorState, FoldEditorState};
use daruda_transcript::fold_mode::FoldMode;
use daruda_ui::select::{self, SelectOption, SelectState};
use daruda_ui::{InputEvent, InputState};

fn settings_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> daruda_ui::Button {
    daruda_ui::button(id, label).tab_stop(true)
}

fn settings_button_danger(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> daruda_ui::Button {
    daruda_ui::button_danger(id, label).tab_stop(true)
}

/// "Copied!" feedback for one copy-to-clipboard button: whether the label is
/// currently swapped, and the timer that swaps it back.
///
/// One type rather than a `bool` + `Option<Task<()>>` pair per button — the
/// two fields are always set together, and there are now two buttons that want
/// exactly this behaviour.
#[derive(Default)]
pub(crate) struct CopyFeedback {
    copied: bool,
    /// Re-created on every click, so a rapid second click restarts the window
    /// rather than inheriting the first one's remaining time. Dropping the
    /// previous `Task` cancels it.
    _revert: Option<Task<()>>,
}

impl CopyFeedback {
    /// Whether to show the confirmation label instead of "Copy".
    pub(crate) fn copied(&self) -> bool {
        self.copied
    }
}

/// One Settings string in `locale`, by key — for a host test checking that its
/// own copy names a Settings label in the words Settings uses.
#[cfg(feature = "test-support")]
pub fn translate_for_test(key: &str, locale: &str) -> String {
    rust_i18n::t!(key, locale = locale).into_owned()
}

/// A section's name as the sidebar shows it, for a host naming the page it
/// opens.
pub fn section_label(section: BuiltinSection) -> String {
    navigation::label(section)
}

/// The asset path of a section's sidebar icon, for a host checking that its
/// bundle carries every one.
pub fn section_icon(section: BuiltinSection) -> &'static str {
    navigation::icon(section)
}

/// What the view asks of whichever `Workspace` is showing it. Kept as an
/// event rather than a handle so this module never points back at its host —
/// the dependency runs one way.
pub enum SettingsEvent {
    /// The user is done: Escape, or the sidebar's back button.
    Close,
    /// A login to run. The machinery lives on `Workspace`, beside the panes
    /// the resulting credentials are for, and it owns the process handle and
    /// the Cancel that goes with it.
    Login(LoginRequest),
    /// Open the active project's config file, where `[shell]` can be
    /// overridden per project.
    OpenProjectConfig,
    /// The accounts file was just written with this state; share it with
    /// every window.
    AccountsChanged(daruda_store::accounts::AccountsState),
    /// A plugin was just installed or removed; rescan the plugin scope for
    /// every window, ahead of the file watcher, which lags the CLI's write.
    PluginsChanged,
    /// An About page button: check for, install, or restart into an update.
    Update(UpdateCommand),
}

/// What only the host can answer or do, when the view needs the answer
/// before it can go on. Changes that affect windows go out as
/// [`SettingsEvent`]s instead; state that changes under Settings is pushed in.
pub trait SettingsServices {
    /// How many open panes run under `account`, across every window.
    fn account_usage(
        &self,
        account: daruda_store::accounts::AccountId,
        cx: &mut gpui::App,
    ) -> usize;
    /// Whether a Telegram bot token is in the credential store.
    fn telegram_token_stored(&self) -> bool;
    fn store_telegram_token(&self, token: &str) -> std::io::Result<()>;
    fn forget_telegram_token(&self) -> std::io::Result<()>;
    /// Whether another daruda is the one serving the bot.
    fn telegram_bot_held_elsewhere(&self, cx: &gpui::App) -> bool;
    /// A fresh pairing code, replacing any pending one.
    fn new_telegram_pair_code(&self, cx: &mut gpui::App) -> String;
}

/// The state every window mirrors, as the host hands it to Settings when it
/// opens. Each part is pushed again through its setter whenever it changes.
#[derive(Default)]
pub struct SharedSnapshot {
    pub accounts: daruda_store::accounts::AccountsState,
    pub account_login_busy: bool,
    pub auth_statuses: std::collections::HashMap<
        daruda_agent::accounts::LoginTarget,
        daruda_agent::accounts::auth_status::AuthStatus,
    >,
    pub agent_vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
    pub plugin_skills: Vec<daruda_agent::skills::Skill>,
    pub update_state: UpdateState,
}

/// Which login an account row asked for.
pub enum LoginRequest {
    /// Add a managed account under `recipe`. The login command comes from the
    /// host's agent catalog, which is why the host resolves it.
    AddAccount(daruda_store::accounts::AccountRecipeId),
    /// Re-run the login for an account that already exists.
    Reauthenticate(daruda_store::accounts::AccountId),
    /// Re-run the login for `recipe`'s ambient home — the credentials a pane
    /// with no managed account uses, which has no account id to name.
    ReauthenticateSystem(daruda_store::accounts::AccountRecipeId),
}

impl EventEmitter<SettingsEvent> for SettingsView {}

#[cfg(feature = "screenshot")]
pub use sections::agent_transcript::editor::EditorShot;

pub struct SettingsView {
    panel_focus_handle: FocusHandle,
    /// Last config this window observed after opening or successfully writing.
    /// Used only to detect a same-field external edit before applying a draft.
    base_config: daruda_config::Config,
    /// Section currently rendered by the body.
    active_section: BuiltinSection,
    sidebar_search_input: Entity<InputState>,
    sidebar_focus_handles: HashMap<BuiltinSection, FocusHandle>,
    /// Each page's Advanced header, so Tab reaches it and Enter/Space folds it.
    advanced_focus_handles: HashMap<BuiltinSection, FocusHandle>,
    /// Whether the host window has a project, told by the host: the project
    /// config button has nothing to open without one.
    project_open: bool,
    /// Per-section input focus handles, in tab-cycle order. `focus_section`
    /// jumps to the first handle when entering a section from outside the
    /// window (sidebar click, external open); `focus_next_input` cycles the
    /// full list on Tab. Single source for both — previously a first-handle
    /// map plus a separately hand-maintained match in `focus_next_input`,
    /// which could drift out of sync when a field was added.
    section_focus_targets: HashMap<BuiltinSection, Vec<FocusHandle>>,
    // ---- form fields ----
    // General page.
    language_select: Entity<SelectState>,
    // Theme (rendered inside Appearance & Window).
    // `terminal_preset_select` controls the cell palette (16-color
    // ANSI + fg/bg); `ui_preset_select` controls the chrome palette
    // (workspace, sidebar, modal, status bar). The two axes are
    // independent — see `daruda_config::ThemeConfig`.
    terminal_preset_select: Entity<SelectState>,
    ui_preset_select: Entity<SelectState>,
    // Font
    terminal_font_family_select: Entity<SelectState>,
    terminal_font_size_input: Entity<InputState>,
    terminal_line_height_input: Entity<InputState>,
    terminal_cell_width_input: Entity<InputState>,
    editor_font_family_select: Entity<SelectState>,
    editor_font_size_input: Entity<InputState>,
    editor_line_height_input: Entity<InputState>,
    agent_chat_font_family_select: Entity<SelectState>,
    agent_chat_font_size_input: Entity<InputState>,
    agent_chat_line_height_input: Entity<InputState>,
    // Cursor
    cursor_style_select: Entity<SelectState>,
    /// Whether a fresh agent-chat pane holds its prose to the reading column.
    /// On the General page beside the themes: it is a reading preference, not
    /// an agent one, and applies to every agent.
    agent_use_reading_width: bool,
    /// Whether tool-group summaries spell each category out beside its icon.
    agent_tool_summary_labels: bool,
    // Agent
    /// Filters the catalog's available and needs-install lists by name or id.
    agent_catalog_search: Entity<InputState>,
    agent_use_modifier_to_send: bool,
    /// The agent catalog in `config.toml` order, editable and non-editable
    /// entries in one list. Reach it through [`Self::agent_catalog_is_empty`] /
    /// [`Self::agent_editable_rows`] / [`Self::agent_unresolved_entries`]:
    /// holding the two kinds in separate vectors previously let each operation
    /// decide on its own whether "the catalog" included the non-editable half,
    /// and the two that answered "no" disagreed with the rest.
    agent_catalog: Vec<AgentCatalogItem>,
    /// What each agent last advertised on the mode / model axes, as the host
    /// pushes it ([`Self::set_agent_vocabulary`]) so rows update while Settings
    /// remains open. A row falls back to [`daruda_config::agent_vocabulary_seed`] per
    /// axis when this has nothing for its current id and command.
    pub(crate) agent_vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
    /// The session host registry (`[[session_hosts]]`) in `config.toml`
    /// order — named, reusable SSH/Docker targets a lane's `session_host`
    /// can reference by id instead of repeating the same target/container
    /// as free text on every lane. See [`SessionHostRow`].
    session_host_rows: Vec<SessionHostRow>,
    // The accounts as the host hands them over ([`Self::set_accounts`]).
    // Every write goes through the section's own handlers, which persist
    // immediately and tell the host ([`SettingsEvent::AccountsChanged`]),
    // which shares the new state with every window. See
    // `sections/accounts.rs`'s module doc.
    accounts: daruda_store::accounts::AccountsState,
    account_login_busy: bool,
    services: std::rc::Rc<dyn SettingsServices>,
    /// How each set of credentials was signed in, as the host pushes it
    /// ([`Self::set_auth_statuses`]). A scope absent here has not been read
    /// yet, which the rows show as nothing rather than as a claim.
    auth_statuses: std::collections::HashMap<
        daruda_agent::accounts::LoginTarget,
        daruda_agent::accounts::auth_status::AuthStatus,
    >,
    // Render
    max_fps_select: Entity<SelectState>,
    // Shell
    close_pane_on_exit: bool,
    // Window
    opacity_input: Entity<InputState>,
    window_blur: bool,
    // Terminal
    scrollback_input: Entity<InputState>,
    inset_x_input: Entity<InputState>,
    inset_y_input: Entity<InputState>,
    // Sidebar
    files_show_hidden: bool,
    files_use_gitignore: bool,
    update_auto_check: bool,
    claude_status_stale_input: Entity<InputState>,
    claude_status_ttl_input: Entity<InputState>,
    usage_limits_poll_input: Entity<InputState>,
    usage_status_poll_input: Entity<InputState>,
    ports_poll_input: Entity<InputState>,
    logs_retention_input: Entity<InputState>,
    logs_max_size_input: Entity<InputState>,
    presence_grace_input: Entity<InputState>,
    presence_idle_input: Entity<InputState>,
    presence_idle_foreground_input: Entity<InputState>,
    agent_input_max_rows_input: Entity<InputState>,
    agent_reading_width_input: Entity<InputState>,
    flow_timeout_minutes_input: Entity<InputState>,
    flow_max_node_runs_input: Entity<InputState>,
    flow_max_cost_input: Entity<InputState>,
    flow_cost_currency_input: Entity<InputState>,
    left_collapsed_by_default: bool,
    preview_tab: bool,
    left_default_width_input: Entity<InputState>,
    shell_program_input: Entity<InputState>,
    /// Settings pages whose Advanced card the user has opened this session.
    advanced_open: std::collections::HashSet<BuiltinSection>,
    shell_natural_text_editing: bool,
    notify_osc9: bool,
    notify_osc777: bool,
    notify_attention: bool,
    notify_long_running: bool,
    notify_skip_focused_pane: bool,
    clipboard_copy_on_select: bool,
    git_confirm_commit: bool,
    git_confirm_push: bool,
    git_default_commit_message: bool,
    notify_hook: bool,
    notify_agent_completion: bool,
    notify_agent_waiting: bool,
    telegram_only_when_away: bool,
    notify_long_running_threshold_input: Entity<InputState>,
    // File Viewer
    syntax_theme_select: Entity<SelectState>,
    // Clipboard
    clipboard_streaming_input: Entity<InputState>,
    // External Editor
    editor_select: Entity<SelectState>,
    file_icon_color_select: Entity<SelectState>,
    // Panels (bottom-dock macro grid)
    panels_grid_columns_input: Entity<InputState>,
    // Claude Status
    claude_status_enable: bool,
    // Notifications (Telegram)
    telegram_enabled: bool,
    /// The Slack / Discord channel settings, a view the host builds over its
    /// own channel runtime.
    remote_integrations: gpui::AnyView,
    orchestrator_enabled: bool,
    /// Empty value = follow the catalog's first entry; see
    /// `sections::orchestrator`.
    orchestrator_agent_select: Entity<SelectState>,
    /// Empty value = the system default account.
    orchestrator_account_select: Entity<SelectState>,
    telegram_token_input: Entity<InputState>,
    /// Presence-only cache of whether a token is currently stored in
    /// the Keychain — seeded once at construction, updated by the
    /// Save/Clear button handlers. Never holds the token itself.
    telegram_token_configured: bool,
    /// The paired chat, mirrored from `SettingsStore` for rendering. Pairing is
    /// written by the bridge's poll loop, not by any editor here, so it is not
    /// one of [`Self::settings_ui_patches`] and the window would otherwise
    /// never learn a phone paired while it was open. Single update site:
    /// [`Self::adopt_external_settings`]. [`Self::validate`] deliberately does
    /// not read this — it re-reads the store, which cannot lag an unflushed
    /// observer effect the way this field can.
    telegram_authorized_chat_id: Option<i64>,
    /// Transient UI-only state (never persisted): the pairing code
    /// most recently generated by "Generate Pairing Code", shown with
    /// the `/pair <code>` instructions until the window closes.
    telegram_pair_code: Option<String>,
    /// "Copied!" feedback for the `/pair <code>` button.
    telegram_pair_command_copy: CopyFeedback,
    /// The same, for the BotFather `/setcommands` block.
    telegram_botfather_copy: CopyFeedback,
    scroll_handle: gpui::ScrollHandle,
    sidebar_scroll_handle: gpui::ScrollHandle,
    _input_subscriptions: Vec<Subscription>,
    error: Option<SharedString>,
    conflict: Option<daruda_config::SettingsPatch>,
    /// Plugin ids (`<plugin>@<marketplace>`) with an `install` /
    /// `uninstall` CLI invocation currently spawned on the
    /// `background_executor`. Used by the Plugin section to show a
    /// transient `Installing…` / `Uninstalling…` label and to swallow
    /// duplicate clicks while the request is in flight.
    pub(crate) plugin_ops_in_flight: std::collections::HashSet<String>,
    /// Installed-plugin manifest snapshot. Refreshed with
    /// [`Self::plugin_skills`] so rendering the Plugin section never
    /// performs file I/O.
    pub(crate) plugin_installs:
        std::collections::BTreeMap<String, daruda_agent::skills::plugins::PluginInstall>,
    /// `<plugin>@<marketplace>` of the plugin whose detail pane is on
    /// the right side of the master-detail layout. `None` shows the
    /// "select a plugin" placeholder.
    pub(crate) plugin_selected: Option<String>,
    /// When `Some`, the right pane swaps from the plugin detail to a
    /// SKILL.md viewer. Cleared by the `← Back` button.
    pub(crate) plugin_view_skill: Option<PluginSkillView>,
    /// The plugin-scope skills, as the host pushes them
    /// ([`Self::set_plugin_skills`]) — so the Plugin page reflects install /
    /// uninstall completions (and external `claude plugin` CLI runs) without
    /// polling.
    pub(crate) plugin_skills: Vec<daruda_agent::skills::Skill>,
    /// Window-aware observer for file-watcher and cross-window settings
    /// changes. Clean forms reload immediately; a local draft is preserved
    /// for the same-field conflict flow.
    _settings_global_subscription: Subscription,
    /// The self-updater's state, as the host pushes it
    /// ([`Self::set_update_state`]).
    pub(crate) update_state: UpdateState,
}

/// In-Settings SKILL.md viewer state. The body load is async (disk
/// read on the background executor) so the variant tracks the three
/// observable phases — request in flight, body ready, body failed.
#[derive(Clone)]
pub(crate) struct PluginSkillView {
    /// `display_name_for_invocation(skill)` — the namespaced form
    /// shown in the header (`<plugin>:<skill>`).
    pub(crate) display_name: String,
    /// Absolute path to the SKILL.md file being viewed. Captured at
    /// open time so the async loader knows exactly which file to read
    /// even if the underlying `GlobalSkills` reshuffles mid-load.
    pub(crate) skill_md_path: std::path::PathBuf,
    pub(crate) body: PluginSkillBodyState,
}

#[derive(Clone)]
pub(crate) enum PluginSkillBodyState {
    Loading,
    Loaded(SharedString),
    Error(SharedString),
}

/// One entry of the Settings agent catalog. An entry that resolves gets
/// editable fields; one naming a preset daruda cannot launch has no fields to
/// edit and is carried verbatim, so both kinds live in a single ordered list —
/// position survives a save, and no operation can see one kind without the
/// other being in reach.
///
/// `large_enum_variant` is allowed for the same reason as `PaneContent`'s: the
/// list holds one item per configured agent, so Box-ing the row only adds a
/// heap hop to every render read for negligible savings.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum AgentCatalogItem {
    Editable(AgentCatalogRow),
    Unresolved(daruda_config::AgentEntry),
}

/// Curated syntax palettes. Value = config key resolved by
/// `syntax::SyntaxPalette::from_config_name`; labels are i18n'd via
/// [`syntax_theme_label`]. The recommended Daruda palette is first.
const SYNTAX_THEMES: &[&str] = &[
    "daruda",
    "one-dark",
    "tokyo-night",
    "catppuccin-mocha",
    "dracula",
    "github-dark",
    "material-palenight",
    "monokai",
    "nord",
    "gruvbox-dark",
    "solarized-dark",
    "ayu-mirage",
    "night-owl",
    "darcula",
];

/// Localized display label for a syntax-palette config value.
fn syntax_theme_label(value: &str) -> String {
    match value {
        "one-dark" => s::settings::syntax_theme_one_dark(),
        "tokyo-night" => s::settings::syntax_theme_tokyo_night(),
        "catppuccin-mocha" => s::settings::syntax_theme_catppuccin_mocha(),
        "dracula" => s::settings::syntax_theme_dracula(),
        "github-dark" => s::settings::syntax_theme_github_dark(),
        "material-palenight" => s::settings::syntax_theme_material_palenight(),
        "monokai" => s::settings::syntax_theme_monokai(),
        "nord" => s::settings::syntax_theme_nord(),
        "gruvbox-dark" => s::settings::syntax_theme_gruvbox_dark(),
        "solarized-dark" => s::settings::syntax_theme_solarized_dark(),
        "ayu-mirage" => s::settings::syntax_theme_ayu_mirage(),
        "night-owl" => s::settings::syntax_theme_night_owl(),
        "darcula" => s::settings::syntax_theme_darcula(),
        _ => s::settings::syntax_theme_daruda(),
    }
}

/// Seconds since the Unix epoch, clamped to `0` on a clock error — mirrors
/// `sections::accounts::now_unix`. Duplicated locally rather than shared:
/// both are trivial, single-use timestamp helpers that would gain nothing
/// from a shared home.
fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The registry editor's rows have no natural field label beyond their
/// position, so the validator's own reason is wrapped with the row's 1-based
/// ordinal — a multi-row save can still point at which one failed.
fn session_host_validation_message(
    index: usize,
    err: session_host::SessionHostError,
) -> SharedString {
    SharedString::from(s::settings::err_session_host_field(
        index + 1,
        s::session_host::host_error(err),
    ))
}

/// Return all font family names available on this system, sorted alphabetically.
/// Every configured family remains selectable even if it is no longer installed.
fn all_font_names(cx: &gpui::App, current: &[&str]) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.sort();
    names.dedup();
    let include_system_ui = current.contains(&daruda_config::SYSTEM_UI_FONT_FAMILY);
    names.retain(|name| name != daruda_config::SYSTEM_UI_FONT_FAMILY || include_system_ui);
    for family in current.iter().rev().filter(|family| !family.is_empty()) {
        if !names.iter().any(|name| name == *family) {
            names.insert(0, (*family).to_owned());
        }
    }
    names
}

fn font_select_option(name: &str) -> SelectOption {
    let label = if name == daruda_config::SYSTEM_UI_FONT_FAMILY {
        s::settings::font_system_ui()
    } else {
        name.to_owned()
    };
    SelectOption::new(name.to_owned(), label)
}

fn font_select_options(cx: &gpui::App, current: &[&str]) -> Vec<SelectOption> {
    all_font_names(cx, current)
        .iter()
        .map(|name| font_select_option(name))
        .collect()
}

// `render::*` is just the `impl Render for SettingsView` block —
// no items are re-exported, but keeping the module declaration above
// is what makes the impl visible to external callers.
