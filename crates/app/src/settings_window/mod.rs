//! Singleton settings window for common daruda config options.
//!
//! Reopening routes through [`SettingsWindow::focus_section`] instead of
//! spawning a duplicate. Builtin sections pair a `BuiltinSection` variant with
//! nav/header strings and a `render_<section>` method.

mod render;
mod sections;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::ops::RangeBounds;

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, IntoElement, SharedString, Subscription, Task,
    Window, WindowBackgroundAppearance, div, prelude::*, px,
};

use crate::lane::session_host;
use crate::surface::strings as s;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::editor::state::FoldEditorState;
use crate::transcript::fold_mode::FoldMode;
use crate::ui::select::{self, SelectOption, SelectState};
use crate::ui::{InputEvent, InputState};
use crate::window_registry::WindowRegistry;

fn settings_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> crate::ui::Button {
    crate::ui::button(id, label).tab_stop(true)
}

fn settings_button_danger(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> crate::ui::Button {
    crate::ui::button_danger(id, label).tab_stop(true)
}

pub struct SettingsWindow {
    panel_focus_handle: FocusHandle,
    /// Last config this window observed after opening or successfully writing.
    /// Used only to detect a same-field external edit before applying a draft.
    base_config: daruda_config::Config,
    /// Section currently rendered by the body.
    active_section: BuiltinSection,
    sidebar_search_input: Entity<InputState>,
    sidebar_focus_handles: HashMap<BuiltinSection, FocusHandle>,
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
    // Theme (rendered inside the General page).
    // `terminal_preset_select` controls the cell palette (16-color
    // ANSI + fg/bg); `ui_preset_select` controls the chrome palette
    // (workspace, sidebar, modal, status bar). The two axes are
    // independent — see `daruda_config::ThemeConfig`.
    terminal_preset_select: Entity<SelectState>,
    ui_preset_select: Entity<SelectState>,
    // Font
    font_family_select: Entity<SelectState>,
    font_size_input: Entity<InputState>,
    editor_font_size_input: Entity<InputState>,
    agent_chat_font_size_input: Entity<InputState>,
    vertical_spacing_input: Entity<InputState>,
    horizontal_spacing_input: Entity<InputState>,
    // Cursor
    cursor_style_select: Entity<SelectState>,
    cursor_blinking: bool,
    // Agent
    agent_preset_select: Entity<SelectState>,
    agent_use_modifier_to_send: bool,
    /// The agent catalog in `config.toml` order, editable and non-editable
    /// entries in one list. Reach it through [`Self::agent_catalog_is_empty`] /
    /// [`Self::agent_editable_rows`] / [`Self::agent_unresolved_entries`]:
    /// holding the two kinds in separate vectors previously let each operation
    /// decide on its own whether "the catalog" included the non-editable half,
    /// and the two that answered "no" disagreed with the rest.
    agent_catalog: Vec<AgentCatalogItem>,
    /// What each agent last advertised on the mode / model axes. Mirrored from
    /// the app-wide vocabulary Global so rows update while Settings remains
    /// open. A row falls back to [`daruda_config::agent_vocabulary_seed`] per
    /// axis when this has nothing for its current id and command.
    pub(super) agent_vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
    /// The session host registry (`[[session_hosts]]`) in `config.toml`
    /// order — named, reusable SSH/Docker targets a lane's `session_host`
    /// can reference by id instead of repeating the same target/container
    /// as free text on every lane. See [`SessionHostRow`].
    session_host_rows: Vec<SessionHostRow>,
    // Accounts (Task 9). Snapshot loaded from `accounts.json` at
    // construction; every write goes through the section's own
    // `set_default_account`/`remove_account` handlers, which persist
    // immediately and broadcast the new state to every open
    // `Workspace` window. See `sections/accounts.rs`'s module doc.
    accounts: daruda_store::accounts::AccountsState,
    account_login_busy: bool,
    /// How each set of credentials was signed in, mirrored from
    /// `auth_status_global`. A scope absent here has not been read yet, which
    /// the rows show as nothing rather than as a claim.
    auth_statuses: std::collections::HashMap<
        crate::workspace::auth_status_global::LoginTarget,
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
    // File Viewer
    syntax_theme_select: Entity<SelectState>,
    // Clipboard
    clipboard_streaming_input: Entity<InputState>,
    // External Editor
    editor_select: Entity<SelectState>,
    // Panels (bottom-dock macro grid)
    panels_grid_columns_input: Entity<InputState>,
    // Claude Status
    claude_status_enable: bool,
    // Notifications (Telegram)
    telegram_enabled: bool,
    telegram_token_input: Entity<InputState>,
    /// Presence-only cache of whether a token is currently stored in
    /// the Keychain — seeded once at construction, updated by the
    /// Save/Clear button handlers. Never holds the token itself.
    telegram_token_configured: bool,
    /// Transient UI-only state (never persisted): the pairing code
    /// most recently generated by "Generate Pairing Code", shown with
    /// the `/pair <code>` instructions until the window closes.
    telegram_pair_code: Option<String>,
    /// Whether the `/pair <code>` command was just copied — drives the
    /// Copy/Copied! label swap, mirrors `ErrorReportModal::copied`.
    telegram_pair_command_copied: bool,
    /// Reverts `telegram_pair_command_copied` after a short delay.
    /// Re-created on every copy click so a rapid second click resets
    /// the window, mirrors `ErrorReportModal::_copied_revert_task`.
    _telegram_pair_copy_revert_task: Option<Task<()>>,
    scroll_handle: gpui::ScrollHandle,
    _input_subscriptions: Vec<Subscription>,
    error: Option<SharedString>,
    conflict: Option<daruda_config::SettingsPatch>,
    /// Plugin ids (`<plugin>@<marketplace>`) with an `install` /
    /// `uninstall` CLI invocation currently spawned on the
    /// `background_executor`. Used by the Plugin section to show a
    /// transient `Installing…` / `Uninstalling…` label and to swallow
    /// duplicate clicks while the request is in flight.
    pub(super) plugin_ops_in_flight: std::collections::HashSet<String>,
    /// Last plugin-op error message — surfaced inline above the plugin
    /// list. `None` clears the banner.
    pub(super) plugin_last_error: Option<SharedString>,
    /// Installed-plugin manifest snapshot. Refreshed when `SkillsState`
    /// changes so rendering the Plugin section never performs file I/O.
    pub(super) plugin_installs:
        std::collections::BTreeMap<String, crate::agent::skills::plugins::PluginInstall>,
    /// `<plugin>@<marketplace>` of the plugin whose detail pane is on
    /// the right side of the master-detail layout. `None` shows the
    /// "select a plugin" placeholder.
    pub(super) plugin_selected: Option<String>,
    /// When `Some`, the right pane swaps from the plugin detail to a
    /// SKILL.md viewer. Cleared by the `← Back` button.
    pub(super) plugin_view_skill: Option<PluginSkillView>,
    /// Subscription that calls `cx.notify()` whenever the app-wide
    /// `SkillsState` Global changes — so the Plugin page reflects
    /// install / uninstall completions (and external `claude plugin`
    /// CLI runs) without polling.
    _skills_global_subscription: Subscription,
    /// Window-aware observer for file-watcher and cross-window settings
    /// changes. Clean forms reload immediately; a local draft is preserved
    /// for the same-field conflict flow.
    _settings_global_subscription: Subscription,
    /// Refreshes every agent row's pickers when a Workspace observes a new
    /// live vocabulary, including while this Settings window stays open.
    _agent_vocabulary_global_subscription: Subscription,
    /// Subscription that refreshes the `accounts` mirror + repaints whenever
    /// the app-wide `AccountsGlobal` changes — so an add/reauth/default/
    /// delete in any Workspace window shows here without a restart.
    _accounts_global_subscription: Subscription,
    /// Held, not dropped: an `observe_global` unsubscribes when its
    /// `Subscription` falls out of scope, and the readings this one waits for
    /// arrive *after* construction — every probe is a background subprocess.
    _auth_status_subscription: Subscription,
    /// Subscription that calls `cx.notify()` whenever the `Updater`
    /// entity changes status — so the About page reflects check /
    /// download / install progress reactively. `None` when the updater
    /// global never registered (unparseable version). Observing the
    /// entity (not the global) is deliberate: the global holder is set
    /// once at init and never replaced, so `observe_global` would never
    /// fire; the entity self-notifies on every status transition.
    _updater_subscription: Option<Subscription>,
}

/// In-Settings SKILL.md viewer state. The body load is async (disk
/// read on the background executor) so the variant tracks the three
/// observable phases — request in flight, body ready, body failed.
#[derive(Clone)]
pub(super) struct PluginSkillView {
    /// `display_name_for_invocation(skill)` — the namespaced form
    /// shown in the header (`<plugin>:<skill>`).
    pub(super) display_name: String,
    /// Absolute path to the SKILL.md file being viewed. Captured at
    /// open time so the async loader knows exactly which file to read
    /// even if the underlying `SkillsState` reshuffles mid-load.
    pub(super) skill_md_path: std::path::PathBuf,
    pub(super) body: PluginSkillBodyState,
}

#[derive(Clone)]
pub(super) enum PluginSkillBodyState {
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
pub(super) enum AgentCatalogItem {
    Editable(AgentCatalogRow),
    Unresolved(daruda_config::AgentEntry),
}

#[derive(Clone, Copy)]
enum TextSetting {
    FontSize,
    EditorFontSize,
    AgentChatFontSize,
    VerticalSpacing,
    HorizontalSpacing,
    WindowOpacity,
    ScrollbackMaxRows,
    TerminalInsetX,
    TerminalInsetY,
    ClipboardStreamingMaxBytes,
    PanelsGridColumns,
}

#[derive(Clone, Copy)]
enum SelectSetting {
    Language,
    TerminalPreset,
    UiPreset,
    FontFamily,
    CursorStyle,
    RenderMaxFps,
    SyntaxTheme,
    PreferredEditor,
}

#[derive(Clone, Copy)]
pub(super) enum BoolSetting {
    CursorBlinking,
    AgentUseModifierToSend,
    ShellClosePaneOnExit,
    WindowBlur,
    FilesShowHidden,
    FilesUseGitignore,
    ClaudeStatusEnabled,
    TelegramEnabled,
}

#[derive(Clone)]
pub(super) struct AgentCatalogRow {
    /// The preset this row references, when it has one. Kept so saving
    /// re-derives the row's overrides against that preset instead of writing a
    /// frozen copy — an untouched field keeps tracking the preset.
    pub(super) preset: Option<String>,
    pub(super) id_input: Entity<InputState>,
    pub(super) name_input: Entity<InputState>,
    /// The command that runs the ACP adapter — `Raw`'s full string, or the
    /// `adapter_command` sub-field when `transport_select` is `ssh`/`docker`.
    pub(super) command_input: Entity<InputState>,
    /// Transport kind for this row: `"raw"` / `"ssh"` / `"docker"` — mirrors
    /// [`daruda_config::AgentLaunch`]'s three variants.
    pub(super) transport_select: Entity<SelectState>,
    /// SSH host — only meaningful (and only rendered) when `transport_select`
    /// is `"ssh"`.
    pub(super) host_input: Entity<InputState>,
    /// Docker container name — only meaningful (and only rendered) when
    /// `transport_select` is `"docker"`.
    pub(super) container_input: Entity<InputState>,
    /// Optional session mode to request when this agent connects. Options are
    /// the agent's cached vocabulary, falling back to the adapter seed the
    /// command names; the empty value is the "agent default" sentinel that
    /// means no override. Rebuilt whenever the row's id or command changes —
    /// see [`SettingsWindow::refresh_agent_row_vocabulary`].
    pub(super) default_mode_select: Entity<SelectState>,
    /// Optional model to request when this agent connects. Same option
    /// sourcing and same empty sentinel as `default_mode_select`.
    pub(super) default_model_select: Entity<SelectState>,
    /// The command's executable name, when [`agent_command_path_warning`]
    /// determined it names a local binary not found on `PATH` — `None` when
    /// no check applies (`npx`/`uvx`/JSON stdio) or the binary was found.
    /// Independent of transport: an ssh/docker row's warning is suppressed at
    /// render time instead (see `sections::agent::render_agent_catalog_row`),
    /// since that needs no fresh `which` lookup. Recomputed on construction
    /// and whenever `command_input` changes (see
    /// [`SettingsWindow::recompute_agent_row_path_warning`]); `which::which`
    /// is I/O, so `render` only ever reads this field, never calls it.
    pub(super) path_warning: Option<String>,
    /// Fold rules a fresh chat pane under this agent starts on, or `None` to
    /// write no key — which resolves to the built-in. Edited through the same
    /// editor the chat pane opens; see [`sections::agent_transcript`].
    pub(super) fold_mode: Option<FoldMode>,
    /// Where this row's fold editor is looking. Not part of the value: the pane
    /// editing the same agent keeps its own — see [`FoldEditorState`].
    pub(super) fold_editor: FoldEditorState,
    /// Visible row kinds a fresh chat pane starts on. Same `None`-is-built-in
    /// rule as `fold_mode`.
    pub(super) display_filter: Option<DisplayFilter>,
    /// Trailing-step window a fresh chat pane starts on. Still a dropdown, and
    /// so still able to load a size it cannot offer.
    pub(super) tail_window_select: Entity<SelectState>,
    /// The one transcript key this row loaded that its dropdown cannot state.
    /// It backs that picker's configured-elsewhere entry and is written back
    /// verbatim while it stays picked, so an unrelated edit cannot flatten a
    /// hand-written value.
    pub(super) transcript: AgentRowTranscript,
}

/// The per-agent transcript-presentation values an [`AgentCatalogRow`] carries
/// verbatim because no picker can state them. `None` means the picker holds the
/// whole value. See [`daruda_config::AgentDefinition`] for what the key means.
#[derive(Clone, Default)]
pub(super) struct AgentRowTranscript {
    pub(super) tail_window: Option<u8>,
}

/// One row of the session host registry editor. Unlike [`AgentCatalogRow`],
/// there is no preset concept here — every row is a plain user-entered
/// `{label, kind, target|container}`.
///
/// `id` is minted once, at construction, and never changes for the row's
/// lifetime: an existing row keeps the [`daruda_config::SessionHostId`] it
/// loaded from config, and a freshly added row mints its own right away
/// so [`SettingsWindow::validate`] can distinguish a persisted row from a
/// newly-added draft by id membership alone. The id persisted on commit can
/// still differ: a row whose Type changed retires it (see
/// [`session_host_entry_id`]).
#[derive(Clone)]
pub(super) struct SessionHostRow {
    pub(super) id: daruda_store::project::SessionHostId,
    pub(super) label_input: Entity<InputState>,
    /// `"ssh"` / `"docker"` — mirrors [`daruda_config::SessionHostKind`]'s
    /// two variants.
    pub(super) kind_select: Entity<SelectState>,
    /// SSH target — only meaningful (and only rendered) when `kind_select`
    /// is `"ssh"`.
    pub(super) target_input: Entity<InputState>,
    /// Docker container name — only meaningful (and only rendered) when
    /// `kind_select` is `"docker"`.
    pub(super) container_input: Entity<InputState>,
}

impl SessionHostRow {
    /// Whether the row's Type dropdown currently says Docker — the single
    /// read the renderer, the tab cycle and `validate` all branch on, so they
    /// can't disagree about which value field is live.
    fn is_docker(&self, cx: &gpui::App) -> bool {
        self.kind_select
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.as_ref() == "docker")
    }
}

impl SettingsWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_section(BuiltinSection::default(), window, cx)
    }

    fn subscribe_draft_input(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
            },
        )
    }

    fn subscribe_sidebar_search(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(state, window, |_this, _, ev: &InputEvent, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
    }

    fn subscribe_text_setting(
        state: &Entity<InputState>,
        setting: TextSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            move |this, state, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.persist_text_setting(state, setting, cx);
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

    fn subscribe_select_setting(
        state: &Entity<SelectState>,
        setting: SelectSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            move |this, state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.persist_select_setting(state, setting, cx);
                }
            },
        )
    }

    fn subscribe_agent_input(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.persist_agent_catalog(cx);
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

    fn subscribe_session_host_input(
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

    /// Build a text-input field, wire it to the standard submit /
    /// clear-error subscription, and capture its focus handle — one call
    /// site instead of three separately-located steps (construct, then
    /// subscribe, then extract the handle) for every bounded text field.
    fn new_text_field(
        placeholder: &str,
        default_value: String,
        setting: TextSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, FocusHandle) {
        let state = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(placeholder)
                .default_value(default_value)
        });
        subs.push(Self::subscribe_text_setting(&state, setting, window, cx));
        let fh = state.read(cx).focus_handle(cx);
        (state, fh)
    }

    fn agent_row_from_definition(
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
        AgentCatalogRow {
            preset,
            transcript: transcript.preserved,
            fold_mode: transcript.fold_mode,
            fold_editor: FoldEditorState::default(),
            display_filter: transcript.display_filter,
            tail_window_select: transcript.tail_window_select,
            id_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("agent-id")
                    .default_value(id)
            }),
            name_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder(s::settings_agent_name_placeholder())
                    .default_value(name)
            }),
            command_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("command --acp")
                    .default_value(command)
            }),
            transport_select: cx.new(|cx| {
                let opts = vec![
                    SelectOption::new("raw", s::settings_agent_transport_raw()),
                    SelectOption::new("ssh", s::settings_agent_transport_ssh()),
                    SelectOption::new("docker", s::settings_agent_transport_docker()),
                ];
                select::state_with_options(opts, Some(&transport_kind), window, cx)
            }),
            host_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("user@host")
                    .default_value(host)
            }),
            container_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("container-name")
                    .default_value(container)
            }),
            default_mode_select: cx.new(|cx| {
                select::state_with_options(mode_options, Some(&default_mode), window, cx)
            }),
            default_model_select: cx.new(|cx| {
                select::state_with_options(model_options, Some(&default_model), window, cx)
            }),
            path_warning,
        }
    }

    /// Wire one agent-catalog row's inputs to the standard submit /
    /// clear-error subscription plus the transport-pick repaint, pushing
    /// each into `subs`. An associated fn (not `&mut self`) so both the
    /// constructor's initial rows (loaded from config) and rows added at
    /// runtime via [`Self::add_agent_row`] go through one wiring site — two
    /// copies would let a row's inputs drift out of sync over which ones are
    /// actually subscribed.
    fn subscribe_agent_row(
        row: &AgentCatalogRow,
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) {
        subs.push(Self::subscribe_agent_input(&row.id_input, window, cx));
        subs.push(Self::subscribe_agent_input(&row.name_input, window, cx));
        subs.push(Self::subscribe_agent_input(&row.command_input, window, cx));
        subs.push(Self::subscribe_agent_input(&row.host_input, window, cx));
        subs.push(Self::subscribe_agent_input(
            &row.container_input,
            window,
            cx,
        ));
        // Every picker persists on pick, like the transport one below. The
        // mode/model option lists are rebuilt in place by the id/command
        // handlers further down, which reuse these same entities so this
        // wiring stays valid.
        for state in [&row.default_mode_select, &row.default_model_select] {
            subs.push(cx.subscribe_in(
                state,
                window,
                |this, _state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        this.persist_agent_catalog(cx);
                    }
                },
            ));
        }
        // The tail picker may shed a "Custom (from config)" entry by choosing
        // one of the stated values. Rebuild the catalog from the committed
        // config after a successful save so the hidden preserved value cannot
        // be picked again later in the same Settings window.
        subs.push(cx.subscribe_in(
            &row.tail_window_select,
            window,
            |this, _state, ev: &select::ConfirmEvent, window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) && this.persist_agent_catalog(cx) {
                    this.reload_agent_catalog_from_live(window, cx);
                }
            },
        ));
        // The row's id keys the cached vocabulary, so retyping it switches
        // both pickers to that agent's option lists.
        subs.push(cx.subscribe_in(
            &row.id_input,
            window,
            |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change)
                    && let Some(index) = this.agent_row_index_by_id(state)
                {
                    this.refresh_agent_row_vocabulary(index, window, cx);
                }
            },
        ));
        // Re-render on transport pick so the row immediately shows/hides the
        // matching host/container field (rows are added/removed at runtime,
        // unlike the fixed global dropdowns wired in `new_with_section`), and
        // so an ssh/docker pick immediately hides a stale PATH warning — that
        // suppression is transport-dependent but needs no fresh `which` call
        // (see `agent.rs::render_agent_catalog_row`), so a plain repaint
        // suffices here.
        subs.push(cx.subscribe_in(
            &row.transport_select,
            window,
            |this, _state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.persist_agent_catalog(cx);
                }
            },
        ));
        // Recompute the local-PATH warning whenever the command text changes,
        // and re-source the pickers: the command names the adapter whose seed
        // fills them before any connect recorded a real vocabulary. Separate
        // from the standard submit/clear-error subscription above, which is
        // shared by every input field and doesn't know which row changed.
        subs.push(cx.subscribe_in(
            &row.command_input,
            window,
            |this, state, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change)
                    && let Some(index) = this.agent_row_index_by_command(state)
                {
                    this.recompute_agent_row_path_warning(index, cx);
                    this.refresh_agent_row_vocabulary(index, window, cx);
                }
            },
        ));
    }

    /// Whether the catalog holds no entries **of either kind**. The single
    /// definition validation and the section's placeholder both read, so a
    /// catalog of only non-editable entries can never be called empty by one
    /// and non-empty by the other.
    pub(super) fn agent_catalog_is_empty(&self) -> bool {
        self.agent_catalog.is_empty()
    }

    /// Editable rows paired with their catalog index (the index
    /// [`Self::remove_agent_catalog_item`] takes — not the "Agent N" ordinal).
    pub(super) fn agent_editable_rows(
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
    pub(super) fn agent_editable_row_mut(&mut self, index: usize) -> Option<&mut AgentCatalogRow> {
        match self.agent_catalog.get_mut(index)? {
            AgentCatalogItem::Editable(row) => Some(row),
            AgentCatalogItem::Unresolved(_) => None,
        }
    }

    /// Non-editable entries paired with their catalog index.
    pub(super) fn agent_unresolved_entries(
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
    pub(super) fn agent_editable_row(&self, ordinal: usize) -> Option<&AgentCatalogRow> {
        self.agent_editable_rows().nth(ordinal).map(|(_, row)| row)
    }

    /// Catalog index of the row whose `command_input` is `entity`, if any. Rows
    /// are looked up by entity identity rather than a captured index because
    /// indices shift on [`Self::remove_agent_catalog_item`], and this closure is
    /// wired once per row without a stable index to close over.
    fn agent_row_index_by_command(&self, entity: &Entity<InputState>) -> Option<usize> {
        self.agent_editable_rows()
            .find(|(_, row)| row.command_input == *entity)
            .map(|(index, _)| index)
    }

    /// Catalog index of the row whose `id_input` is `entity` — same
    /// entity-identity lookup, and same reason, as
    /// [`Self::agent_row_index_by_command`].
    fn agent_row_index_by_id(&self, entity: &Entity<InputState>) -> Option<usize> {
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
    fn recompute_agent_row_path_warning(&mut self, index: usize, cx: &mut Context<Self>) {
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
    /// (the "Add Preset" button), so the saved entry references it rather than
    /// copying its fields; `None` adds a custom row.
    pub(super) fn add_agent_row(
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
    pub(super) fn remove_agent_catalog_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.agent_catalog.len() {
            let removed = self.agent_catalog.remove(index);
            self.error = None;
            if !self.persist_agent_catalog(cx) {
                self.agent_catalog.insert(index, removed);
            }
            cx.notify();
        }
    }

    /// Build one session-host row from a persisted entry — used both for the
    /// rows seeded at window-open and (via [`Self::add_session_host_row`])
    /// nowhere else, since a freshly added row starts blank rather than
    /// copying an existing entry.
    fn session_host_row_from_entry(
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
    fn session_host_row_new(
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
                    .placeholder(s::settings_session_host_field_label())
                    .default_value(label.to_string())
            }),
            kind_select: cx.new(|cx| {
                let opts = vec![
                    SelectOption::new("ssh", s::settings_session_host_kind_ssh()),
                    SelectOption::new("docker", s::settings_session_host_kind_docker()),
                ];
                select::state_with_options(opts, Some(&SharedString::from(kind)), window, cx)
            }),
            target_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("user@host")
                    .default_value(target.to_string())
            }),
            container_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("container-name")
                    .default_value(container.to_string())
            }),
        }
    }

    /// Wire one session-host row's inputs to the standard submit /
    /// clear-error subscription plus the kind-pick repaint, mirroring
    /// [`Self::subscribe_agent_row`].
    fn subscribe_session_host_row(
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
    pub(super) fn session_host_rows(&self) -> impl Iterator<Item = (usize, &SessionHostRow)> + '_ {
        self.session_host_rows.iter().enumerate()
    }

    /// Test-only: the `ordinal`-th row — production code walks
    /// [`Self::session_host_rows`], which also yields the index every
    /// mutation needs.
    #[cfg(test)]
    pub(super) fn session_host_row(&self, ordinal: usize) -> Option<&SessionHostRow> {
        self.session_host_rows.get(ordinal)
    }

    /// Append a blank row the user fills in by hand. A fresh
    /// [`daruda_store::project::SessionHostId`] is minted right away — see
    /// [`SessionHostRow::id`]'s doc for why.
    pub(super) fn add_session_host_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn remove_session_host_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.session_host_rows.len() {
            let removed = self.session_host_rows.remove(index);
            self.error = None;
            if !self.persist_session_hosts(cx) {
                self.session_host_rows.insert(index, removed);
            }
            cx.notify();
        }
    }

    pub fn new_with_section(
        active: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Defensive idempotent init for test fixtures that open a
        // Settings window without going through `globals::init_all`.
        crate::settings_store::SettingsStore::init(cx);
        let config = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();

        let sidebar_search_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::settings_search_placeholder())
        });
        let sidebar_focus_handles = BuiltinSection::ALL
            .iter()
            .copied()
            .map(|section| (section, cx.focus_handle().tab_stop(true)))
            .collect();

        // Language select — options driven by the canonical locale list so
        // adding a new locale only requires updating SUPPORTED_LOCALES.
        let lang = SharedString::from(config.general.language.clone());
        let language_select = cx.new(|cx| {
            let opts: Vec<select::SelectOption> = daruda_config::SUPPORTED_LOCALES
                .iter()
                .map(|&slug| {
                    let label = match slug {
                        "auto" => s::settings_language_auto(),
                        "en" => s::settings_language_en(),
                        "ko" => s::settings_language_ko(),
                        other => other.to_owned(),
                    };
                    select::SelectOption::new(slug, label)
                })
                .collect();
            select::state_with_options(opts, Some(&lang), window, cx)
        });

        // Terminal preset select — cell palette (fg/bg + ANSI 16).
        let terminal_preset = SharedString::from(config.theme.terminal_preset.clone());
        let terminal_preset_select = cx.new(|cx| {
            let opts = daruda_config::THEME_PRESETS
                .iter()
                .map(|p| SelectOption::new(p.name, p.display_name))
                .collect();
            select::state_with_options(opts, Some(&terminal_preset), window, cx)
        });

        // UI preset select — chrome palette (workspace, modal, status bar, …).
        let ui_preset = SharedString::from(config.theme.ui_preset.clone());
        let ui_preset_select = cx.new(|cx| {
            let opts = daruda_config::UI_THEME_PRESETS
                .iter()
                .map(|p| SelectOption::new(p.name, p.display_name))
                .collect();
            select::state_with_options(opts, Some(&ui_preset), window, cx)
        });

        let font_names = all_font_names(cx, &config.font.family);
        let font_family = SharedString::from(config.font.family.clone());
        let font_family_select = cx.new(|cx| {
            let opts = font_names
                .iter()
                .map(|n| SelectOption::simple(n.clone()))
                .collect();
            select::state_with_options(opts, Some(&font_family), window, cx)
        });

        // Collected as each field below is built, instead of assembled in one
        // block after the fact — keeps "construct → subscribe → focus handle
        // → section jump target" together per field. `section_focus_targets`
        // is the single source for both `focus_section` (first handle) and
        // `focus_next_input` (full per-section cycle order).
        let mut input_subscriptions: Vec<Subscription> = Vec::new();
        input_subscriptions.push(Self::subscribe_sidebar_search(
            &sidebar_search_input,
            window,
            cx,
        ));
        let mut section_focus_targets: HashMap<BuiltinSection, Vec<FocusHandle>> = HashMap::new();

        let (font_size_input, font_size_fh) = Self::new_text_field(
            "e.g. 13",
            format!("{}", config.font.size),
            TextSetting::FontSize,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Font)
            .or_default()
            .push(font_size_fh);
        // Not wired into the Font tab cycle (kept out of
        // `section_focus_targets`) — only font_size/vertical/horizontal
        // spacing are, matching the pre-existing cycle order.
        let (editor_font_size_input, _) = Self::new_text_field(
            "e.g. 13",
            format!("{}", config.font.editor_size),
            TextSetting::EditorFontSize,
            window,
            cx,
            &mut input_subscriptions,
        );
        let (agent_chat_font_size_input, _) = Self::new_text_field(
            "e.g. 13",
            format!("{}", config.font.agent_chat_size),
            TextSetting::AgentChatFontSize,
            window,
            cx,
            &mut input_subscriptions,
        );
        let (vertical_spacing_input, vertical_spacing_fh) = Self::new_text_field(
            "e.g. 1.0",
            format!("{}", config.font.vertical_spacing),
            TextSetting::VerticalSpacing,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Font)
            .or_default()
            .push(vertical_spacing_fh);
        let (horizontal_spacing_input, horizontal_spacing_fh) = Self::new_text_field(
            "e.g. 1.0",
            format!("{}", config.font.horizontal_spacing),
            TextSetting::HorizontalSpacing,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Font)
            .or_default()
            .push(horizontal_spacing_fh);
        let (opacity_input, opacity_fh) = Self::new_text_field(
            "0.1 – 1.0",
            format!("{}", config.window.opacity),
            TextSetting::WindowOpacity,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Window)
            .or_default()
            .push(opacity_fh);
        let (scrollback_input, scrollback_fh) = Self::new_text_field(
            "e.g. 10000",
            format!("{}", config.scrollback.max_rows),
            TextSetting::ScrollbackMaxRows,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Terminal)
            .or_default()
            .push(scrollback_fh);
        let (inset_x_input, inset_x_fh) = Self::new_text_field(
            "e.g. 4",
            format!("{}", config.font.inset_x),
            TextSetting::TerminalInsetX,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Terminal)
            .or_default()
            .push(inset_x_fh);
        let (inset_y_input, inset_y_fh) = Self::new_text_field(
            "e.g. 2",
            format!("{}", config.font.inset_y),
            TextSetting::TerminalInsetY,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Terminal)
            .or_default()
            .push(inset_y_fh);
        let (clipboard_streaming_input, clipboard_streaming_fh) = Self::new_text_field(
            "e.g. 10485760",
            format!("{}", config.clipboard.streaming_max_bytes),
            TextSetting::ClipboardStreamingMaxBytes,
            window,
            cx,
            &mut input_subscriptions,
        );
        section_focus_targets
            .entry(BuiltinSection::Clipboard)
            .or_default()
            .push(clipboard_streaming_fh);
        // External editor select — "" (empty, the config default) means the
        // OS default handler; every other value is a `daruda_config::editor`
        // preset name.
        let preferred_editor = SharedString::from(config.editor.preferred.clone());
        let editor_select = cx.new(|cx| {
            let mut opts = vec![select::SelectOption::new(
                "",
                s::settings_editor_system_default(),
            )];
            opts.extend(
                daruda_config::EXTERNAL_EDITOR_PRESETS
                    .iter()
                    .map(|p| select::SelectOption::new(p.name, p.display_name)),
            );
            select::state_with_options(opts, Some(&preferred_editor), window, cx)
        });
        let (panels_grid_columns_input, panels_grid_columns_fh) = Self::new_text_field(
            "1 – 16",
            format!("{}", config.panels.grid_columns),
            TextSetting::PanelsGridColumns,
            window,
            cx,
            &mut input_subscriptions,
        );
        // Second (and last) text input on the merged Dock page — after
        // the Sidebar subsection's checkboxes (no text input) and before
        // the Bottom Dock subsection's own fields, so it's simply
        // appended to the same section's tab-cycle list.
        section_focus_targets
            .entry(BuiltinSection::Dock)
            .or_default()
            .push(panels_grid_columns_fh);
        // Never pre-filled with the real token (`default_value`) — a stored
        // secret is never re-displayed in a text field, so this field can't
        // go through `new_text_field` (which always sets a default value).
        // The "Token configured" status line covers presence instead.
        let telegram_token_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::settings_telegram_token_placeholder())
                .masked(true)
        });
        input_subscriptions.push(Self::subscribe_draft_input(
            &telegram_token_input,
            window,
            cx,
        ));
        section_focus_targets
            .entry(BuiltinSection::Notifications)
            .or_default()
            .push(telegram_token_input.read(cx).focus_handle(cx));
        let telegram_token_configured = crate::telegram::keychain::read_token().is_some();

        let cursor_style_str: SharedString = match config.cursor.style {
            daruda_config::CursorStyle::Block => "block".into(),
            daruda_config::CursorStyle::Underline => "underline".into(),
            daruda_config::CursorStyle::Bar => "bar".into(),
        };
        let cursor_style_select = cx.new(|cx| {
            select::state_with_options(
                vec![
                    SelectOption::new("block", s::settings_cursor_block()),
                    SelectOption::new("underline", s::settings_cursor_underline()),
                    SelectOption::new("bar", s::settings_cursor_bar()),
                ],
                Some(&cursor_style_str),
                window,
                cx,
            )
        });

        let agent_preset = SharedString::from("codex-acp");
        let agent_preset_select = cx.new(|cx| {
            // Every built-in preset, launchable or not. A preset that needs a
            // manual install cannot become a row, so its label says so and the
            // section swaps the Add button for install instructions — hiding it
            // instead left the user with no sign the agent exists at all.
            let opts = daruda_config::agent_presets()
                .map(|preset| {
                    let label = match preset.launchability {
                        daruda_config::PresetLaunchability::Runnable { .. } => {
                            s::settings_agent_preset_option(preset.name, preset.id)
                        }
                        daruda_config::PresetLaunchability::NeedsManualInstall { .. } => {
                            s::settings_agent_preset_option_needs_install(preset.name, preset.id)
                        }
                    };
                    SelectOption::new(preset.id, label)
                })
                .collect();
            select::state_with_options(opts, Some(&agent_preset), window, cx)
        });

        // Settings has no `data_dir` field of its own, but vocabulary is shared
        // app-wide so every open window sees a connection's advertisement.
        let agent_vocabulary_data_dir = daruda_store::persistence::default_data_dir();
        crate::workspace::agent_vocabulary_global::install_path(cx, &agent_vocabulary_data_dir);
        let agent_vocabulary =
            crate::workspace::agent_vocabulary_global::snapshot(cx, &agent_vocabulary_data_dir);

        // Entries that resolve get an editable row; entries that don't (a preset
        // id daruda no longer knows, or one that needs a manual install) have no
        // fields to edit and are kept verbatim — a config the editor cannot
        // represent must not be a config the editor deletes. Both kinds stay in
        // one list at their config position.
        let agent_catalog: Vec<AgentCatalogItem> = config
            .agents
            .iter()
            .map(|entry| match entry.resolve() {
                Some(definition) => AgentCatalogItem::Editable(Self::agent_row_from_definition(
                    &definition,
                    &agent_vocabulary,
                    entry.preset_id().map(str::to_string),
                    window,
                    cx,
                )),
                None => AgentCatalogItem::Unresolved(entry.clone()),
            })
            .collect();

        let session_host_rows: Vec<SessionHostRow> = config
            .session_hosts
            .iter()
            .map(|entry| Self::session_host_row_from_entry(entry, window, cx))
            .collect();

        let max_fps_str: SharedString = config.render.max_fps.to_string().into();
        let max_fps_select = cx.new(|cx| {
            let opts = daruda_config::ALLOWED_MAX_FPS
                .iter()
                .map(|fps| {
                    SelectOption::new(
                        SharedString::from(fps.to_string()),
                        s::settings_max_fps_option(*fps),
                    )
                })
                .collect();
            select::state_with_options(opts, Some(&max_fps_str), window, cx)
        });

        // Resolve through the palette so a legacy / unknown stored value
        // (e.g. an old "base16-ocean.dark") still shows its effective
        // palette selected instead of a blank dropdown.
        let syntax_theme = SharedString::from(
            crate::ui::theme::SyntaxPalette::from_config_name(&config.file_viewer.syntax_theme)
                .config_name(),
        );
        let syntax_theme_select = cx.new(|cx| {
            let opts = SYNTAX_THEMES
                .iter()
                .map(|v| SelectOption::new(*v, syntax_theme_label(v)))
                .collect();
            select::state_with_options(opts, Some(&syntax_theme), window, cx)
        });

        for (state, setting) in [
            (&language_select, SelectSetting::Language),
            (&terminal_preset_select, SelectSetting::TerminalPreset),
            (&ui_preset_select, SelectSetting::UiPreset),
            (&font_family_select, SelectSetting::FontFamily),
            (&cursor_style_select, SelectSetting::CursorStyle),
            (&max_fps_select, SelectSetting::RenderMaxFps),
            (&syntax_theme_select, SelectSetting::SyntaxTheme),
            (&editor_select, SelectSetting::PreferredEditor),
        ] {
            input_subscriptions.push(Self::subscribe_select_setting(state, setting, window, cx));
        }
        // Picking a preset swaps the Add button for install instructions when
        // that preset ships binaries only — repaint so the swap is immediate.
        input_subscriptions.push(cx.subscribe_in(
            &agent_preset_select,
            window,
            |_this, _state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    cx.notify();
                }
            },
        ));
        let editable_rows = || {
            agent_catalog.iter().filter_map(|item| match item {
                AgentCatalogItem::Editable(row) => Some(row),
                AgentCatalogItem::Unresolved(_) => None,
            })
        };
        for row in editable_rows() {
            Self::subscribe_agent_row(row, window, cx, &mut input_subscriptions);
        }

        // First-field jump target for the Agent section (its full tab-cycle
        // list is dynamic — see `focus_next_input` — since rows are
        // added/removed at runtime).
        if let Some(row) = editable_rows().next() {
            section_focus_targets
                .entry(BuiltinSection::Agent)
                .or_default()
                .push(row.id_input.read(cx).focus_handle(cx));
        }

        for row in &session_host_rows {
            Self::subscribe_session_host_row(row, window, cx, &mut input_subscriptions);
        }
        // First-field jump target for the Session Hosts section — mirrors
        // the Agent section above. An empty catalog is a valid state (see
        // `daruda_config::Config::session_hosts`), so there may be nothing
        // to jump to; `focus_section` already falls back to the panel focus
        // handle when the map has no entry.
        if let Some(row) = session_host_rows.first() {
            section_focus_targets
                .entry(BuiltinSection::SessionHosts)
                .or_default()
                .push(row.label_input.read(cx).focus_handle(cx));
        }

        let _updater_subscription =
            crate::update::Updater::get(cx).map(|e| cx.observe(&e, |_, _, cx| cx.notify()));

        // Managed accounts come from the app-wide Global (single source of
        // truth). Install it from disk if this window opened before any
        // Workspace (idempotent), then mirror the current value — refreshed
        // on change by `_accounts_global_subscription`.
        crate::workspace::accounts_global::install_if_absent(
            cx,
            daruda_store::accounts::load_accounts().unwrap_or_default(),
        );
        let accounts = crate::workspace::accounts_global::snapshot(cx);
        let account_login_busy = crate::workspace::accounts_global::login_busy(cx);
        let _accounts_global_subscription = cx
            .observe_global::<crate::workspace::accounts_global::AccountsGlobal>(|this, cx| {
                this.accounts = crate::workspace::accounts_global::snapshot(cx);
                this.account_login_busy = crate::workspace::accounts_global::login_busy(cx);
                cx.notify();
            });
        let _agent_vocabulary_global_subscription = cx.observe_global_in::<
            crate::workspace::agent_vocabulary_global::AgentVocabularyGlobal,
        >(window, |this, window, cx| {
            let data_dir = daruda_store::persistence::default_data_dir();
            this.agent_vocabulary =
                crate::workspace::agent_vocabulary_global::snapshot(cx, &data_dir);
            for index in 0..this.agent_catalog.len() {
                this.refresh_agent_row_vocabulary(index, window, cx);
            }
        });
        // Same mirror shape for the sign-in readings: a Workspace produces
        // them off-thread, so they land after this window is already open.
        crate::workspace::auth_status_global::install_if_absent(cx);
        let auth_statuses = crate::workspace::auth_status_global::snapshot(cx);
        let _auth_status_subscription = cx
            .observe_global::<crate::workspace::auth_status_global::AuthStatusGlobal>(
                |this, cx| {
                    this.auth_statuses = crate::workspace::auth_status_global::snapshot(cx);
                    cx.notify();
                },
            );

        let result = Self {
            panel_focus_handle: cx.focus_handle(),
            base_config: config.clone(),
            active_section: active,
            sidebar_search_input,
            sidebar_focus_handles,
            section_focus_targets,
            language_select,
            terminal_preset_select,
            ui_preset_select,
            font_family_select,
            font_size_input,
            editor_font_size_input,
            agent_chat_font_size_input,
            vertical_spacing_input,
            horizontal_spacing_input,
            cursor_style_select,
            cursor_blinking: config.cursor.blinking,
            agent_preset_select,
            agent_use_modifier_to_send: config.agent.use_modifier_to_send,
            agent_catalog,
            agent_vocabulary,
            session_host_rows,
            accounts,
            account_login_busy,
            auth_statuses,
            max_fps_select,
            close_pane_on_exit: config.shell.close_pane_on_exit,
            opacity_input,
            window_blur: config.window.blur,
            scrollback_input,
            inset_x_input,
            inset_y_input,
            files_show_hidden: config.left_dock.files_show_hidden,
            files_use_gitignore: config.left_dock.files_use_gitignore,
            syntax_theme_select,
            clipboard_streaming_input,
            editor_select,
            panels_grid_columns_input,
            claude_status_enable: config.claude_status.enable,
            telegram_enabled: config.telegram.enabled,
            telegram_token_input,
            telegram_token_configured,
            telegram_pair_code: None,
            telegram_pair_command_copied: false,
            _telegram_pair_copy_revert_task: None,
            scroll_handle: gpui::ScrollHandle::new(),
            _input_subscriptions: input_subscriptions,
            error: None,
            conflict: None,
            plugin_ops_in_flight: std::collections::HashSet::new(),
            plugin_last_error: None,
            plugin_installs: sections::plugin::read_plugin_installs_indexed(),
            plugin_selected: None,
            plugin_view_skill: None,
            _skills_global_subscription: cx.observe_global::<crate::agent::skills::SkillsState>(
                |this, cx| {
                    this.plugin_installs = sections::plugin::read_plugin_installs_indexed();
                    cx.notify();
                },
            ),
            _settings_global_subscription: cx
                .observe_global_in::<crate::settings_store::SettingsStore>(
                    window,
                    |this, window, cx| {
                        this.sync_external_settings(window, cx);
                    },
                ),
            _agent_vocabulary_global_subscription,
            _accounts_global_subscription,
            _auth_status_subscription,
            _updater_subscription,
        };
        // The scroll handle is populated during prepaint, which runs after render.
        // Schedule a re-render so the scrollbar thumb appears on first display
        // without requiring an initial scroll event.
        cx.spawn(async move |this, cx| {
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();

        // Track this window in the WindowRegistry so `open_settings_window`
        // can raise it instead of opening a second copy. The window's root
        // view is `gpui_component::Root` (the SettingsWindow needs Root in
        // the tree so `gpui_component::Input::TextElement::paint` can call
        // `Root::read` without panicking), so the registry stores a typed
        // `SettingsHandle` that bundles the window handle with a
        // `WeakEntity<SettingsWindow>` to recover the inner entity.
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        WindowRegistry::register_settings(window_handle, weak, cx);
        cx.on_release(move |_: &mut SettingsWindow, cx: &mut gpui::App| {
            WindowRegistry::clear_settings(cx);
        })
        .detach();

        result
    }

    /// Switch the active page and (when applicable) land focus on the
    /// section's natural starting field. Called both by sidebar clicks
    /// and by `windows::open_settings_window` when a second open
    /// dispatch arrives while a Settings window is already alive.
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
        if let Some(fh) = self
            .section_focus_targets
            .get(&section)
            .and_then(|handles| handles.first())
            .cloned()
        {
            fh.focus(window, cx);
        } else {
            self.panel_focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub fn active_section(&self) -> BuiltinSection {
        self.active_section
    }

    fn dismiss(&mut self, window: &mut Window) {
        window.remove_window();
    }

    /// Read, trim, and parse `input`'s value into `T`, rejecting values
    /// outside `range`. Collapses the "read → trim → parse → range filter →
    /// error" pattern repeated for every bounded numeric field below.
    fn parse_bounded_field<T: std::str::FromStr + PartialOrd>(
        input: &Entity<InputState>,
        range: impl RangeBounds<T>,
        err: impl FnOnce() -> SharedString,
        cx: &gpui::App,
    ) -> Result<T, SharedString> {
        input
            .read(cx)
            .value()
            .trim()
            .parse::<T>()
            .ok()
            .filter(|v| range.contains(v))
            .ok_or_else(err)
    }

    fn apply_settings_patch(
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
                self.error = Some(SharedString::from(message));
                cx.notify();
                false
            }
        }
    }

    fn apply_settings_patch_force(
        &mut self,
        patch: daruda_config::SettingsPatch,
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
                self.error = Some(SharedString::from(message));
                cx.notify();
                false
            }
        }
    }

    fn advance_base_field_from_live(
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

    fn reload_agent_catalog_from_live(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let live = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();
        self.load_agent_catalog_from_config(&live, window, cx);
        cx.notify();
    }

    fn load_agent_catalog_from_config(
        &mut self,
        live: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let vocabulary = &self.agent_vocabulary;
        let catalog = live
            .agents
            .iter()
            .map(|entry| match entry.resolve() {
                Some(definition) => AgentCatalogItem::Editable(Self::agent_row_from_definition(
                    &definition,
                    vocabulary,
                    entry.preset_id().map(str::to_string),
                    window,
                    cx,
                )),
                None => AgentCatalogItem::Unresolved(entry.clone()),
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

    fn overwrite_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn reload_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn load_settings_patch(
        &mut self,
        patch: &daruda_config::SettingsPatch,
        live: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match patch {
            daruda_config::SettingsPatch::GeneralLanguage(_) => Self::set_select_value(
                &self.language_select,
                live.general.language.clone(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::TerminalPreset(_) => Self::set_select_value(
                &self.terminal_preset_select,
                live.theme.terminal_preset.clone(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::UiPreset(_) => Self::set_select_value(
                &self.ui_preset_select,
                live.theme.ui_preset.clone(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::FontFamily(_) => Self::set_select_value(
                &self.font_family_select,
                live.font.family.clone(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::FontSize(_) => {
                Self::set_input_value(&self.font_size_input, live.font.size, window, cx)
            }
            daruda_config::SettingsPatch::EditorFontSize(_) => Self::set_input_value(
                &self.editor_font_size_input,
                live.font.editor_size,
                window,
                cx,
            ),
            daruda_config::SettingsPatch::AgentChatFontSize(_) => Self::set_input_value(
                &self.agent_chat_font_size_input,
                live.font.agent_chat_size,
                window,
                cx,
            ),
            daruda_config::SettingsPatch::VerticalSpacing(_) => Self::set_input_value(
                &self.vertical_spacing_input,
                live.font.vertical_spacing,
                window,
                cx,
            ),
            daruda_config::SettingsPatch::HorizontalSpacing(_) => Self::set_input_value(
                &self.horizontal_spacing_input,
                live.font.horizontal_spacing,
                window,
                cx,
            ),
            daruda_config::SettingsPatch::CursorStyle(_) => {
                let value = match live.cursor.style {
                    daruda_config::CursorStyle::Block => "block",
                    daruda_config::CursorStyle::Underline => "underline",
                    daruda_config::CursorStyle::Bar => "bar",
                };
                Self::set_select_value(&self.cursor_style_select, value, window, cx);
            }
            daruda_config::SettingsPatch::CursorBlinking(_) => {
                self.cursor_blinking = live.cursor.blinking;
            }
            daruda_config::SettingsPatch::AgentUseModifierToSend(_) => {
                self.agent_use_modifier_to_send = live.agent.use_modifier_to_send;
            }
            daruda_config::SettingsPatch::AgentCatalog(_) => {
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
            daruda_config::SettingsPatch::RenderMaxFps(_) => Self::set_select_value(
                &self.max_fps_select,
                live.render.max_fps.to_string(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::ShellClosePaneOnExit(_) => {
                self.close_pane_on_exit = live.shell.close_pane_on_exit;
            }
            daruda_config::SettingsPatch::WindowOpacity(_) => {
                Self::set_input_value(&self.opacity_input, live.window.opacity, window, cx)
            }
            daruda_config::SettingsPatch::WindowBlur(_) => {
                self.window_blur = live.window.blur;
            }
            daruda_config::SettingsPatch::ScrollbackMaxRows(_) => {
                Self::set_input_value(&self.scrollback_input, live.scrollback.max_rows, window, cx)
            }
            daruda_config::SettingsPatch::TerminalInsetX(_) => {
                Self::set_input_value(&self.inset_x_input, live.font.inset_x, window, cx)
            }
            daruda_config::SettingsPatch::TerminalInsetY(_) => {
                Self::set_input_value(&self.inset_y_input, live.font.inset_y, window, cx)
            }
            daruda_config::SettingsPatch::FilesShowHidden(_) => {
                self.files_show_hidden = live.left_dock.files_show_hidden;
            }
            daruda_config::SettingsPatch::FilesUseGitignore(_) => {
                self.files_use_gitignore = live.left_dock.files_use_gitignore;
            }
            daruda_config::SettingsPatch::SyntaxTheme(_) => Self::set_select_value(
                &self.syntax_theme_select,
                live.file_viewer.syntax_theme.clone(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::ClipboardStreamingMaxBytes(_) => {
                Self::set_input_value(
                    &self.clipboard_streaming_input,
                    live.clipboard.streaming_max_bytes,
                    window,
                    cx,
                );
            }
            daruda_config::SettingsPatch::PreferredEditor(_) => Self::set_select_value(
                &self.editor_select,
                live.editor.preferred.clone(),
                window,
                cx,
            ),
            daruda_config::SettingsPatch::PanelsGridColumns(_) => Self::set_input_value(
                &self.panels_grid_columns_input,
                live.panels.grid_columns,
                window,
                cx,
            ),
            daruda_config::SettingsPatch::ClaudeStatusEnabled(_) => {
                self.claude_status_enable = live.claude_status.enable;
            }
            daruda_config::SettingsPatch::TelegramEnabled(_) => {
                self.telegram_enabled = live.telegram.enabled;
            }
            daruda_config::SettingsPatch::ToggleStatusBarItem(_)
            | daruda_config::SettingsPatch::TelegramAuthorizedChatId(_) => {}
        }
    }

    fn set_select_value(
        select: &Entity<SelectState>,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        select.update(cx, |select, cx| {
            select.set_selected_value(&value, window, cx);
        });
    }

    fn set_input_value(
        input: &Entity<InputState>,
        value: impl ToString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.to_string();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn settings_ui_patches(config: &daruda_config::Config) -> Vec<daruda_config::SettingsPatch> {
        vec![
            daruda_config::SettingsPatch::GeneralLanguage(config.general.language.clone()),
            daruda_config::SettingsPatch::TerminalPreset(config.theme.terminal_preset.clone()),
            daruda_config::SettingsPatch::UiPreset(config.theme.ui_preset.clone()),
            daruda_config::SettingsPatch::FontFamily(config.font.family.clone()),
            daruda_config::SettingsPatch::FontSize(config.font.size),
            daruda_config::SettingsPatch::EditorFontSize(config.font.editor_size),
            daruda_config::SettingsPatch::AgentChatFontSize(config.font.agent_chat_size),
            daruda_config::SettingsPatch::VerticalSpacing(config.font.vertical_spacing),
            daruda_config::SettingsPatch::HorizontalSpacing(config.font.horizontal_spacing),
            daruda_config::SettingsPatch::CursorStyle(config.cursor.style),
            daruda_config::SettingsPatch::CursorBlinking(config.cursor.blinking),
            daruda_config::SettingsPatch::AgentUseModifierToSend(config.agent.use_modifier_to_send),
            daruda_config::SettingsPatch::AgentCatalog(config.agents.clone()),
            daruda_config::SettingsPatch::SessionHosts {
                entries: config.session_hosts.clone(),
                tombstones: config.session_host_tombstones.clone(),
            },
            daruda_config::SettingsPatch::RenderMaxFps(config.render.max_fps),
            daruda_config::SettingsPatch::ShellClosePaneOnExit(config.shell.close_pane_on_exit),
            daruda_config::SettingsPatch::WindowOpacity(config.window.opacity),
            daruda_config::SettingsPatch::WindowBlur(config.window.blur),
            daruda_config::SettingsPatch::ScrollbackMaxRows(config.scrollback.max_rows),
            daruda_config::SettingsPatch::TerminalInsetX(config.font.inset_x),
            daruda_config::SettingsPatch::TerminalInsetY(config.font.inset_y),
            daruda_config::SettingsPatch::FilesShowHidden(config.left_dock.files_show_hidden),
            daruda_config::SettingsPatch::FilesUseGitignore(config.left_dock.files_use_gitignore),
            daruda_config::SettingsPatch::SyntaxTheme(config.file_viewer.syntax_theme.clone()),
            daruda_config::SettingsPatch::ClipboardStreamingMaxBytes(
                config.clipboard.streaming_max_bytes,
            ),
            daruda_config::SettingsPatch::PreferredEditor(config.editor.preferred.clone()),
            daruda_config::SettingsPatch::PanelsGridColumns(config.panels.grid_columns),
            daruda_config::SettingsPatch::ClaudeStatusEnabled(config.claude_status.enable),
            daruda_config::SettingsPatch::TelegramEnabled(config.telegram.enabled),
        ]
    }

    fn sync_external_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn persist_select_setting(
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
        let patch = match setting {
            SelectSetting::Language => daruda_config::SettingsPatch::GeneralLanguage(value),
            SelectSetting::TerminalPreset => daruda_config::SettingsPatch::TerminalPreset(value),
            SelectSetting::UiPreset => daruda_config::SettingsPatch::UiPreset(value),
            SelectSetting::FontFamily => daruda_config::SettingsPatch::FontFamily(value),
            SelectSetting::CursorStyle => {
                let style = match value.as_str() {
                    "underline" => daruda_config::CursorStyle::Underline,
                    "bar" => daruda_config::CursorStyle::Bar,
                    _ => daruda_config::CursorStyle::Block,
                };
                daruda_config::SettingsPatch::CursorStyle(style)
            }
            SelectSetting::RenderMaxFps => {
                let Some(fps) = value.parse::<u32>().ok() else {
                    return;
                };
                daruda_config::SettingsPatch::RenderMaxFps(fps)
            }
            SelectSetting::SyntaxTheme => daruda_config::SettingsPatch::SyntaxTheme(value),
            SelectSetting::PreferredEditor => daruda_config::SettingsPatch::PreferredEditor(value),
        };
        self.apply_settings_patch(patch, cx);
    }

    pub(super) fn persist_bool_setting(
        &mut self,
        setting: BoolSetting,
        value: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let patch = match setting {
            BoolSetting::CursorBlinking => daruda_config::SettingsPatch::CursorBlinking(value),
            BoolSetting::AgentUseModifierToSend => {
                daruda_config::SettingsPatch::AgentUseModifierToSend(value)
            }
            BoolSetting::ShellClosePaneOnExit => {
                daruda_config::SettingsPatch::ShellClosePaneOnExit(value)
            }
            BoolSetting::WindowBlur => daruda_config::SettingsPatch::WindowBlur(value),
            BoolSetting::FilesShowHidden => daruda_config::SettingsPatch::FilesShowHidden(value),
            BoolSetting::FilesUseGitignore => {
                daruda_config::SettingsPatch::FilesUseGitignore(value)
            }
            BoolSetting::ClaudeStatusEnabled => {
                daruda_config::SettingsPatch::ClaudeStatusEnabled(value)
            }
            BoolSetting::TelegramEnabled => daruda_config::SettingsPatch::TelegramEnabled(value),
        };
        self.apply_settings_patch(patch, cx)
    }

    fn persist_text_setting(
        &mut self,
        input: &Entity<InputState>,
        setting: TextSetting,
        cx: &mut Context<Self>,
    ) {
        let patch = match setting {
            TextSetting::FontSize => Self::parse_bounded_field(
                input,
                6.0..=72.0,
                || SharedString::from(s::settings_err_font_size()),
                cx,
            )
            .map(daruda_config::SettingsPatch::FontSize),
            TextSetting::EditorFontSize => Self::parse_bounded_field(
                input,
                6.0..=72.0,
                || SharedString::from(s::settings_err_editor_font_size()),
                cx,
            )
            .map(daruda_config::SettingsPatch::EditorFontSize),
            TextSetting::AgentChatFontSize => Self::parse_bounded_field(
                input,
                6.0..=72.0,
                || SharedString::from(s::settings_err_agent_chat_font_size()),
                cx,
            )
            .map(daruda_config::SettingsPatch::AgentChatFontSize),
            TextSetting::VerticalSpacing => Self::parse_bounded_field(
                input,
                0.5..=2.0,
                || SharedString::from(s::settings_err_spacing()),
                cx,
            )
            .map(daruda_config::SettingsPatch::VerticalSpacing),
            TextSetting::HorizontalSpacing => Self::parse_bounded_field(
                input,
                0.5..=2.0,
                || SharedString::from(s::settings_err_spacing()),
                cx,
            )
            .map(daruda_config::SettingsPatch::HorizontalSpacing),
            TextSetting::WindowOpacity => Self::parse_bounded_field(
                input,
                0.1..=1.0,
                || SharedString::from(s::settings_err_opacity()),
                cx,
            )
            .map(daruda_config::SettingsPatch::WindowOpacity),
            TextSetting::ScrollbackMaxRows => Self::parse_bounded_field(
                input,
                1_000..=500_000,
                || SharedString::from(s::settings_err_scrollback()),
                cx,
            )
            .map(daruda_config::SettingsPatch::ScrollbackMaxRows),
            TextSetting::TerminalInsetX => Self::parse_bounded_field(
                input,
                0.0..=32.0,
                || SharedString::from(s::settings_err_inset()),
                cx,
            )
            .map(daruda_config::SettingsPatch::TerminalInsetX),
            TextSetting::TerminalInsetY => Self::parse_bounded_field(
                input,
                0.0..=32.0,
                || SharedString::from(s::settings_err_inset()),
                cx,
            )
            .map(daruda_config::SettingsPatch::TerminalInsetY),
            TextSetting::ClipboardStreamingMaxBytes => Self::parse_bounded_field(
                input,
                4_096..=67_108_864,
                || SharedString::from(s::settings_err_clipboard()),
                cx,
            )
            .map(daruda_config::SettingsPatch::ClipboardStreamingMaxBytes),
            TextSetting::PanelsGridColumns => Self::parse_bounded_field(
                input,
                1..=16,
                || SharedString::from(s::settings_err_grid_columns()),
                cx,
            )
            .map(daruda_config::SettingsPatch::PanelsGridColumns),
        };

        match patch {
            Ok(patch) => {
                self.apply_settings_patch(patch, cx);
            }
            Err(message) => {
                self.error = Some(message);
                cx.notify();
            }
        }
    }

    fn collect_agent_catalog(
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
                return Err(SharedString::from(s::settings_err_agent_catalog_field(
                    ordinal,
                )));
            }
            if !is_valid_agent_id(&id) {
                return Err(SharedString::from(s::settings_err_agent_catalog_id(&id)));
            }
            if !seen_agent_ids.insert(id.clone()) {
                return Err(SharedString::from(s::settings_err_agent_catalog_duplicate(
                    &id,
                )));
            }
            let kind = row
                .transport_select
                .read(cx)
                .selected_value()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "raw".to_string());
            let host = row.host_input.read(cx).value().trim().to_string();
            let container = row.container_input.read(cx).value().trim().to_string();
            if !agent_row_is_valid(&kind, &host, &container) {
                let message = if kind == "ssh" {
                    s::settings_err_agent_catalog_host(ordinal)
                } else {
                    s::settings_err_agent_catalog_container(ordinal)
                };
                return Err(SharedString::from(message));
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
            agents.push(daruda_config::AgentEntry::for_definition(
                daruda_config::AgentDefinition {
                    id,
                    name,
                    launch,
                    default_mode: row.default_mode(cx),
                    default_model: row.default_model(cx),
                    fold_mode: row.fold_mode(),
                    tail_window: row.tail_window(cx),
                    display_filter: row.display_filter(),
                },
                row.preset.as_deref(),
            ));
        }
        if agents.is_empty() {
            return Err(SharedString::from(s::settings_err_agent_catalog_empty()));
        }
        Ok(agents)
    }

    fn persist_agent_catalog(&mut self, cx: &mut Context<Self>) -> bool {
        match self.collect_agent_catalog(cx) {
            Ok(agents) => {
                self.apply_settings_patch(daruda_config::SettingsPatch::AgentCatalog(agents), cx)
            }
            Err(message) => {
                self.error = Some(message);
                cx.notify();
                false
            }
        }
    }

    fn collect_session_hosts(
        &self,
        cx: &gpui::App,
    ) -> Result<Vec<daruda_config::SessionHostEntry>, SharedString> {
        let previous = &crate::settings_store::SettingsStore::global(cx)
            .user()
            .session_hosts;
        self.collect_session_hosts_against(previous, true, cx)
    }

    fn collect_session_hosts_against(
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
                    s::settings_err_session_host_label_empty(index + 1),
                ));
            }
            if !seen_labels.insert(label.to_ascii_lowercase()) {
                return Err(SharedString::from(
                    s::settings_err_session_host_label_duplicate(&label),
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

    fn persist_session_hosts(&mut self, cx: &mut Context<Self>) -> bool {
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

    fn validate(&self, cx: &gpui::App) -> Result<daruda_config::Config, SharedString> {
        // Start from the snapshot taken at window-open time so fields not
        // exposed in the UI (e.g. [colors], [keybindings]) are preserved.
        let mut config = self.base_config.clone();

        config.general.language = self
            .language_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "auto".to_owned());

        config.theme.terminal_preset = self
            .terminal_preset_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "default".to_owned());

        config.theme.ui_preset = self
            .ui_preset_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| daruda_config::ui_theme_presets::DEFAULT.to_owned());

        config.font.family = self
            .font_family_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| daruda_config::FontConfig::default().family);

        config.font.size = Self::parse_bounded_field(
            &self.font_size_input,
            6.0..=72.0,
            || SharedString::from(s::settings_err_font_size()),
            cx,
        )?;
        config.font.editor_size = Self::parse_bounded_field(
            &self.editor_font_size_input,
            6.0..=72.0,
            || SharedString::from(s::settings_err_editor_font_size()),
            cx,
        )?;
        config.font.agent_chat_size = Self::parse_bounded_field(
            &self.agent_chat_font_size_input,
            6.0..=72.0,
            || SharedString::from(s::settings_err_agent_chat_font_size()),
            cx,
        )?;
        config.font.vertical_spacing = Self::parse_bounded_field(
            &self.vertical_spacing_input,
            0.5..=2.0,
            || SharedString::from(s::settings_err_spacing()),
            cx,
        )?;
        config.font.horizontal_spacing = Self::parse_bounded_field(
            &self.horizontal_spacing_input,
            0.5..=2.0,
            || SharedString::from(s::settings_err_spacing()),
            cx,
        )?;

        config.cursor.style = match self
            .cursor_style_select
            .read(cx)
            .selected_value()
            .map(|s| s.as_ref())
        {
            Some("underline") => daruda_config::CursorStyle::Underline,
            Some("bar") => daruda_config::CursorStyle::Bar,
            _ => daruda_config::CursorStyle::Block,
        };
        config.cursor.blinking = self.cursor_blinking;

        config.agent.use_modifier_to_send = self.agent_use_modifier_to_send;
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

        config.render.max_fps = self
            .max_fps_select
            .read(cx)
            .selected_value()
            .and_then(|s| s.as_ref().parse::<u32>().ok())
            .unwrap_or(config.render.max_fps);
        config.render.clamp();

        config.shell.close_pane_on_exit = self.close_pane_on_exit;

        config.window.opacity = Self::parse_bounded_field(
            &self.opacity_input,
            0.1..=1.0,
            || SharedString::from(s::settings_err_opacity()),
            cx,
        )?;
        config.window.blur = self.window_blur;

        config.scrollback.max_rows = Self::parse_bounded_field(
            &self.scrollback_input,
            1_000..=500_000,
            || SharedString::from(s::settings_err_scrollback()),
            cx,
        )?;

        config.font.inset_x = Self::parse_bounded_field(
            &self.inset_x_input,
            0.0..=32.0,
            || SharedString::from(s::settings_err_inset()),
            cx,
        )?;
        config.font.inset_y = Self::parse_bounded_field(
            &self.inset_y_input,
            0.0..=32.0,
            || SharedString::from(s::settings_err_inset()),
            cx,
        )?;

        config.left_dock.files_show_hidden = self.files_show_hidden;
        config.left_dock.files_use_gitignore = self.files_use_gitignore;

        config.file_viewer.syntax_theme = self
            .syntax_theme_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| daruda_config::FileViewerConfig::default().syntax_theme);

        config.clipboard.streaming_max_bytes = Self::parse_bounded_field(
            &self.clipboard_streaming_input,
            4_096..=67_108_864,
            || SharedString::from(s::settings_err_clipboard()),
            cx,
        )?;

        config.editor.preferred = self
            .editor_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_default();

        config.panels.grid_columns = Self::parse_bounded_field(
            &self.panels_grid_columns_input,
            1..=16,
            || SharedString::from(s::settings_err_grid_columns()),
            cx,
        )?;

        config.claude_status.enable = self.claude_status_enable;

        // `authorized_chat_id` is managed asynchronously by pairing/unpairing.
        // Re-read it here so draft detection never treats a completed pairing
        // as a local form edit.
        config.telegram.authorized_chat_id = crate::settings_store::SettingsStore::global(cx)
            .user_arc()
            .telegram
            .authorized_chat_id;
        config.telegram.enabled = self.telegram_enabled;

        Ok(config)
    }

    #[cfg(test)]
    fn focus_next_input(&self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    }

    pub(super) fn section_label(
        label: impl Into<gpui::SharedString>,
        cx: &gpui::App,
    ) -> impl IntoElement {
        div()
            .text_size(px(theme::LANE_SECTION_HEADER_FONT_SIZE))
            .text_color(theme::current(cx).text_muted)
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(label.into())
    }
}

/// Map config window settings to the GPUI window background appearance.
pub(crate) fn window_background_for(config: &daruda_config::Config) -> WindowBackgroundAppearance {
    if config.window.blur {
        WindowBackgroundAppearance::Blurred
    } else if config.window.opacity < 1.0 {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

/// Curated syntax palettes. Value = config key resolved by
/// `palette::SyntaxPalette::from_config_name`; labels are i18n'd via
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
        "one-dark" => s::settings_syntax_theme_one_dark(),
        "tokyo-night" => s::settings_syntax_theme_tokyo_night(),
        "catppuccin-mocha" => s::settings_syntax_theme_catppuccin_mocha(),
        "dracula" => s::settings_syntax_theme_dracula(),
        "github-dark" => s::settings_syntax_theme_github_dark(),
        "material-palenight" => s::settings_syntax_theme_material_palenight(),
        "monokai" => s::settings_syntax_theme_monokai(),
        "nord" => s::settings_syntax_theme_nord(),
        "gruvbox-dark" => s::settings_syntax_theme_gruvbox_dark(),
        "solarized-dark" => s::settings_syntax_theme_solarized_dark(),
        "ayu-mirage" => s::settings_syntax_theme_ayu_mirage(),
        "night-owl" => s::settings_syntax_theme_night_owl(),
        "darcula" => s::settings_syntax_theme_darcula(),
        _ => s::settings_syntax_theme_daruda(),
    }
}

fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Whether an agent catalog row's transport-specific fields are filled in.
/// `"ssh"` requires a non-blank `host`; `"docker"` requires a non-blank
/// `container`; any other `kind` (`"raw"`, or an unrecognized/absent select
/// value) has no extra requirement. Pure and GPUI-free so it is directly
/// unit-testable — the save loop in [`SettingsWindow::validate`] is the only
/// caller.
fn agent_row_is_valid(kind: &str, host: &str, container: &str) -> bool {
    match kind {
        "ssh" => !host.trim().is_empty(),
        "docker" => !container.trim().is_empty(),
        _ => true,
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

/// `session_host::SessionHostError` names the field by
/// [`session_host::SessionHostField`], but the registry editor's rows have
/// no natural field label beyond their position — reuses the same
/// `session_host.err_*` strings [`SessionHostModal`] shows, wrapped with the
/// row's 1-based ordinal so a multi-row save can still point at which one
/// failed.
///
/// [`SessionHostModal`]: crate::workspace::left_dock::projects::session_host_modal::SessionHostModal
fn session_host_validation_message(
    index: usize,
    err: session_host::SessionHostError,
) -> SharedString {
    use session_host::{SessionHostError, SessionHostField};
    let reason = match err {
        SessionHostError::Empty(SessionHostField::Target) => s::session_host_err_target_empty(),
        SessionHostError::Empty(SessionHostField::Container) => {
            s::session_host_err_container_empty()
        }
        // Never produced by this editor (it never validates a session path),
        // but matched explicitly rather than panicking so a future caller
        // that does can't trigger an `unreachable!()`.
        SessionHostError::Empty(SessionHostField::SessionPath) => {
            s::session_host_err_session_path_empty()
        }
        SessionHostError::Unsafe(SessionHostField::Target) => s::session_host_err_target_unsafe(),
        SessionHostError::Unsafe(SessionHostField::Container) => {
            s::session_host_err_container_unsafe()
        }
        SessionHostError::Unsafe(SessionHostField::SessionPath) => {
            s::session_host_err_session_path_unsafe()
        }
    };
    SharedString::from(s::settings_err_session_host_field(index + 1, &reason))
}

/// The string value carried inside a [`daruda_config::SessionHostKind`] —
/// `target` for `Ssh`, `container` for `Docker`. Used to populate a
/// [`daruda_config::SessionHostTombstone::value`] display field, which
/// duplicates what `kind` already carries structurally (see that field's
/// doc in `daruda_config::session_host`).
fn session_host_kind_value(kind: &daruda_config::SessionHostKind) -> &str {
    match kind {
        daruda_config::SessionHostKind::Ssh { target } => target,
        daruda_config::SessionHostKind::Docker { container } => container,
    }
}

/// The id a saved session-host row carries into `config.toml`: the row's own
/// id, unless the row's Type was changed — i.e. a persisted entry holds that
/// id under the other [`daruda_config::SessionHostKind`] variant.
///
/// A Type change has to retire the id because a lane's `registry_id` resolves
/// on id *and* kind (`lane::session_host` treats a kind mismatch as "not
/// found", never coercing an SSH lane into a Docker one). Keeping the id would
/// leave every linked lane silently unresolvable with nothing recorded;
/// retiring it makes [`reconcile_session_host_tombstones`] log the removal, so
/// the lane reports Orphaned and heals if an equivalent host is registered
/// again.
fn session_host_entry_id(
    previous_entries: &[daruda_config::SessionHostEntry],
    row_id: daruda_store::project::SessionHostId,
    kind: &daruda_config::SessionHostKind,
) -> daruda_store::project::SessionHostId {
    let retyped = previous_entries.iter().any(|entry| {
        entry.id == row_id && std::mem::discriminant(&entry.kind) != std::mem::discriminant(kind)
    });
    if retyped {
        daruda_store::project::SessionHostId::new()
    } else {
        row_id
    }
}

/// Cap on the session host tombstone list — see
/// [`reconcile_session_host_tombstones`].
const MAX_SESSION_HOST_TOMBSTONES: usize = 20;

/// Diff `previous_entries`/`current_entries` (matched by
/// [`daruda_store::project::SessionHostId`]) against `previous_tombstones` to
/// produce the tombstone list committed with the catalog:
///
/// 1. Every entry present in `previous_entries` but missing from
///    `current_entries` was removed by this edit — append a fresh tombstone for
///    it (`redirected_to: None`, `removed_at`).
/// 2. Trim to the most recently removed [`MAX_SESSION_HOST_TOMBSTONES`]
///    (oldest evicted first).
/// 3. Every entry in `current_entries` that is genuinely new (its id was not
///    in `previous_entries`) is matched by exact `(kind, value)` —
///    `SessionHostKind` equality already covers both, since a kind's inner
///    field *is* its value — against the surviving unresolved tombstones
///    (`redirected_to: None`). The most recently removed match gets
///    `redirected_to` set to the new entry's id; an older tie is left
///    unresolved, the plan's stated tie-break.
///
/// Pure and GPUI-free so it is directly unit-testable — [`SettingsWindow::validate`]
/// is the only caller.
fn reconcile_session_host_tombstones(
    previous_entries: &[daruda_config::SessionHostEntry],
    previous_tombstones: &[daruda_config::SessionHostTombstone],
    current_entries: &[daruda_config::SessionHostEntry],
    removed_at: u64,
) -> Vec<daruda_config::SessionHostTombstone> {
    let mut tombstones = previous_tombstones.to_vec();

    for old in previous_entries {
        if current_entries.iter().any(|e| e.id == old.id) {
            continue;
        }
        tombstones.push(daruda_config::SessionHostTombstone {
            old_id: old.id,
            kind: old.kind.clone(),
            value: session_host_kind_value(&old.kind).to_string(),
            removed_at,
            redirected_to: None,
        });
    }
    if tombstones.len() > MAX_SESSION_HOST_TOMBSTONES {
        tombstones.sort_by_key(|t| t.removed_at);
        let excess = tombstones.len() - MAX_SESSION_HOST_TOMBSTONES;
        tombstones.drain(0..excess);
    }

    for new in current_entries {
        if previous_entries.iter().any(|e| e.id == new.id) {
            continue;
        }
        let redirect = tombstones
            .iter_mut()
            .filter(|t| t.redirected_to.is_none() && t.kind == new.kind)
            .max_by_key(|t| t.removed_at);
        if let Some(t) = redirect {
            t.redirected_to = Some(new.id);
        }
    }

    tombstones
}

/// The token [`agent_command_path_warning`] should look up on `PATH`, or
/// `None` when `command` means no local-PATH check applies.
///
/// No check applies when: the command is a JSON stdio config (self-contained,
/// no executable name to look up — same discrimination `daruda_config`'s
/// `AgentLaunch` uses to gate its own shell-string edits); or the first real
/// token (after stripping any `NAME=value` env-prefix assignments) is
/// `npx`/`uvx` — daruda provisions Node.js itself, and `uvx` resolves its own
/// Python venvs, so neither names a binary the user is expected to have
/// installed locally. Transport (ssh/docker exempts the whole row) is not
/// considered here — that suppression needs no `which` call, so it is applied
/// at render time instead (`sections::agent::render_agent_catalog_row`).
fn path_check_token(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.starts_with('{') {
        return None;
    }
    let token = daruda_acp::node::first_command_token(trimmed)?;
    (!matches!(token.as_str(), "npx" | "uvx")).then_some(token)
}

/// The catalog row's local-PATH warning: `Some(token)` when
/// [`path_check_token`] says a check applies and that token is not found on
/// `PATH`, `None` otherwise (no check applies, or the binary was found).
/// Advisory only — a missing command never blocks
/// [`SettingsWindow::validate`], since registering an agent before
/// installing its CLI (or before adding it to `PATH`) is a legitimate flow.
fn agent_command_path_warning(command: &str) -> Option<String> {
    let token = path_check_token(command)?;
    which::which(&token).is_err().then_some(token)
}

/// Return all font family names available on this system, sorted alphabetically.
/// The `current` family is always included as the first entry so the select
/// can show the currently-configured font even if it is not yet installed.
fn all_font_names(cx: &gpui::App, current: &str) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.sort();
    names.dedup();
    if !current.is_empty() && !names.iter().any(|n| n == current) {
        names.insert(0, current.to_owned());
    }
    names
}

// `render::*` is just the `impl Render for SettingsWindow` block —
// no items are re-exported, but keeping the module declaration above
// is what makes the impl visible to external callers.

#[cfg(test)]
mod agent_row_validation_tests {
    use super::agent_row_is_valid;

    #[test]
    fn agent_row_validation_cases() {
        let cases = [
            ("ssh empty host", "ssh", "", "", false),
            ("ssh blank host", "ssh", "   ", "", false),
            ("ssh host", "ssh", "vm-work", "", true),
            ("docker empty container", "docker", "", "", false),
            ("docker blank container", "docker", "", "   ", false),
            ("docker container", "docker", "", "ubuntu-dev", true),
            ("raw empty", "raw", "", "", true),
            (
                "raw ignores host and container",
                "raw",
                "irrelevant",
                "irrelevant",
                true,
            ),
            ("empty kind", "", "", "", true),
            ("bogus kind", "bogus", "", "", true),
        ];

        for (name, kind, host, container, expected) in cases {
            assert_eq!(
                agent_row_is_valid(kind, host, container),
                expected,
                "{name}"
            );
        }
    }
}

#[cfg(test)]
mod agent_command_path_warning_tests {
    use super::{agent_command_path_warning, path_check_token};

    /// Never a real executable name, so `which` is guaranteed to miss it.
    const MISSING_COMMAND: &str = "daruda-settings-path-warning-test-missing-binary";

    #[test]
    fn path_check_token_cases() {
        let cases = [
            ("npx", "npx -y some-pkg@latest --acp", None),
            ("uvx", "uvx some-pkg@latest -x", None),
            ("json stdio", r#"{"command": "some-binary"}"#, None),
            (
                "env npx",
                "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y pkg@latest --acp",
                None,
            ),
            (
                "env local command",
                "FOO=1 my-local-cli acp",
                Some("my-local-cli"),
            ),
            ("local command", "my-local-cli acp", Some("my-local-cli")),
        ];

        for (name, command, expected) in cases {
            assert_eq!(path_check_token(command).as_deref(), expected, "{name}");
        }
    }

    #[test]
    fn agent_command_path_warning_cases() {
        let cases = [
            ("found on path", "sh -c true", None),
            ("missing", MISSING_COMMAND, Some(MISSING_COMMAND)),
            (
                "npx unavailable",
                "npx -y definitely-nonexistent-package@latest",
                None,
            ),
        ];

        for (name, command, expected) in cases {
            assert_eq!(
                agent_command_path_warning(command).as_deref(),
                expected,
                "{name}"
            );
        }
    }
}

#[cfg(test)]
mod session_host_reconcile_tests {
    use super::{reconcile_session_host_tombstones, session_host_entry_id};
    use daruda_config::{SessionHostEntry, SessionHostKind, SessionHostTombstone};
    use daruda_store::project::SessionHostId;

    fn ssh_entry(id: SessionHostId, label: &str, target: &str) -> SessionHostEntry {
        SessionHostEntry {
            id,
            label: label.to_string(),
            kind: SessionHostKind::Ssh {
                target: target.to_string(),
            },
        }
    }

    #[test]
    fn an_unchanged_catalog_produces_no_new_tombstones() {
        let id = SessionHostId::new();
        let entries = vec![ssh_entry(id, "Box", "vm-work")];
        let tombstones = reconcile_session_host_tombstones(&entries, &[], &entries, 100);
        assert!(tombstones.is_empty());
    }

    #[test]
    fn a_removed_entry_gets_a_fresh_tombstone() {
        let id = SessionHostId::new();
        let previous = vec![ssh_entry(id, "Box", "vm-work")];
        let tombstones = reconcile_session_host_tombstones(&previous, &[], &[], 100);
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].old_id, id);
        assert_eq!(
            tombstones[0].kind,
            SessionHostKind::Ssh {
                target: "vm-work".into()
            }
        );
        assert_eq!(tombstones[0].value, "vm-work");
        assert_eq!(tombstones[0].removed_at, 100);
        assert_eq!(tombstones[0].redirected_to, None);
    }

    /// A row surviving with the same id but an edited target/label is an
    /// in-place edit, not a removal — the catalog's own id-based resolution
    /// already picks up the new value (see `lane::session_host::resolve_catalog_id`),
    /// so no tombstone should be recorded for it.
    #[test]
    fn an_edited_entry_that_keeps_its_id_is_not_tombstoned() {
        let id = SessionHostId::new();
        let previous = vec![ssh_entry(id, "Box", "old-target")];
        let current = vec![ssh_entry(id, "Renamed", "new-target")];
        let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
        assert!(tombstones.is_empty());
    }

    /// A row that keeps its id while switching Type stops resolving for every
    /// lane linked to it (id *and* kind must match), so the id is retired and
    /// the removal recorded — with no redirect, since bridging kinds would
    /// turn an SSH lane into a Docker one.
    #[test]
    fn a_retyped_row_retires_its_id_and_gets_tombstoned() {
        let row_id = SessionHostId::new();
        let previous = vec![ssh_entry(row_id, "Box", "vm-work")];
        let retyped = SessionHostKind::Docker {
            container: "dev-1".into(),
        };
        let saved_id = session_host_entry_id(&previous, row_id, &retyped);
        assert_ne!(saved_id, row_id);

        let current = vec![SessionHostEntry {
            id: saved_id,
            label: "Box".to_string(),
            kind: retyped,
        }];
        let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].old_id, row_id);
        assert_eq!(
            tombstones[0].kind,
            SessionHostKind::Ssh {
                target: "vm-work".into()
            }
        );
        assert_eq!(tombstones[0].redirected_to, None);
    }

    /// Editing only the value keeps the same kind variant, which still
    /// resolves — retiring the id there would orphan lanes for nothing.
    #[test]
    fn an_edited_value_keeps_the_rows_id() {
        let row_id = SessionHostId::new();
        let previous = vec![ssh_entry(row_id, "Box", "vm-work")];
        let kind = SessionHostKind::Ssh {
            target: "vm-other".into(),
        };
        assert_eq!(session_host_entry_id(&previous, row_id, &kind), row_id);
    }

    #[test]
    fn a_row_with_no_persisted_entry_keeps_its_id() {
        let row_id = SessionHostId::new();
        let kind = SessionHostKind::Docker {
            container: "dev-1".into(),
        };
        assert_eq!(session_host_entry_id(&[], row_id, &kind), row_id);
    }

    #[test]
    fn a_new_entry_matching_kind_and_value_redirects_the_matching_tombstone() {
        let old_id = SessionHostId::new();
        let new_id = SessionHostId::new();
        let previous = vec![ssh_entry(old_id, "Box", "vm-work")];
        let current = vec![ssh_entry(new_id, "Box (recreated)", "vm-work")];
        let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].old_id, old_id);
        assert_eq!(tombstones[0].redirected_to, Some(new_id));
    }

    /// A Docker entry must never redirect an SSH tombstone (or vice versa)
    /// even if the string value happens to collide.
    #[test]
    fn a_kind_mismatch_never_redirects() {
        let old_id = SessionHostId::new();
        let new_id = SessionHostId::new();
        let previous = vec![ssh_entry(old_id, "Box", "shared-name")];
        let current = vec![SessionHostEntry {
            id: new_id,
            label: "Container".into(),
            kind: SessionHostKind::Docker {
                container: "shared-name".into(),
            },
        }];
        let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].redirected_to, None);
    }

    /// Two live tombstones share `(kind, value)` — only the most recently
    /// removed one gets redirected; the older one stays unresolved rather
    /// than being touched, per the plan's stated tie-break.
    #[test]
    fn ties_redirect_only_the_most_recently_removed_tombstone() {
        let older_id = SessionHostId::new();
        let newer_id = SessionHostId::new();
        let recreated_id = SessionHostId::new();
        let previous_tombstones = vec![
            SessionHostTombstone {
                old_id: older_id,
                kind: SessionHostKind::Ssh {
                    target: "vm-work".into(),
                },
                value: "vm-work".into(),
                removed_at: 50,
                redirected_to: None,
            },
            SessionHostTombstone {
                old_id: newer_id,
                kind: SessionHostKind::Ssh {
                    target: "vm-work".into(),
                },
                value: "vm-work".into(),
                removed_at: 75,
                redirected_to: None,
            },
        ];
        let current = vec![ssh_entry(recreated_id, "Box", "vm-work")];
        let tombstones =
            reconcile_session_host_tombstones(&[], &previous_tombstones, &current, 100);
        let older = tombstones.iter().find(|t| t.old_id == older_id).unwrap();
        let newer = tombstones.iter().find(|t| t.old_id == newer_id).unwrap();
        assert_eq!(older.redirected_to, None);
        assert_eq!(newer.redirected_to, Some(recreated_id));
    }

    /// A tombstone that already resolved to a surviving entry is never
    /// re-targeted by a second recreation of the same value.
    #[test]
    fn an_already_resolved_tombstone_is_never_redirected_again() {
        let old_id = SessionHostId::new();
        let first_redirect = SessionHostId::new();
        let second_id = SessionHostId::new();
        let previous_tombstones = vec![SessionHostTombstone {
            old_id,
            kind: SessionHostKind::Ssh {
                target: "vm-work".into(),
            },
            value: "vm-work".into(),
            removed_at: 50,
            redirected_to: Some(first_redirect),
        }];
        let current = vec![ssh_entry(second_id, "Box again", "vm-work")];
        let tombstones =
            reconcile_session_host_tombstones(&[], &previous_tombstones, &current, 100);
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].redirected_to, Some(first_redirect));
    }

    #[test]
    fn the_tombstone_list_is_trimmed_to_the_most_recent_twenty_oldest_evicted_first() {
        let previous_tombstones: Vec<SessionHostTombstone> = (0..20)
            .map(|i| SessionHostTombstone {
                old_id: SessionHostId::new(),
                kind: SessionHostKind::Ssh {
                    target: format!("box-{i}"),
                },
                value: format!("box-{i}"),
                removed_at: i,
                redirected_to: None,
            })
            .collect();
        let oldest_id = previous_tombstones[0].old_id;
        let newest_removed_id = SessionHostId::new();
        let previous_entries = vec![ssh_entry(newest_removed_id, "Freshly removed", "box-fresh")];
        // Removing this one 21st-oldest entry pushes the total to 21, which
        // must evict exactly the single oldest tombstone (removed_at: 0).
        let tombstones =
            reconcile_session_host_tombstones(&previous_entries, &previous_tombstones, &[], 1_000);
        assert_eq!(tombstones.len(), 20);
        assert!(
            !tombstones.iter().any(|t| t.old_id == oldest_id),
            "the oldest tombstone must be evicted"
        );
        assert!(
            tombstones
                .iter()
                .any(|t| t.old_id == newest_removed_id && t.removed_at == 1_000),
            "the just-created tombstone must survive the trim"
        );
    }
}
