use gpui::{Context, Window};

use crate::surface::strings as s;
use crate::workspace::Workspace;

impl Workspace {
    /// Re-resolve the live store with this workspace's project overlay and
    /// apply it — the one path both a settings change and an OS appearance
    /// flip take into `apply_config`.
    pub(crate) fn apply_store_config(&mut self, cx: &mut Context<Self>) {
        let store = crate::settings_store::SettingsStore::global(cx);
        let lane = self.active_project().map(|p| p.root.as_path());
        let effective = store.effective_for(lane);
        self.apply_config(&effective, cx);
    }

    /// An OS light / dark flip. The theme state decides whether that repaints
    /// anything and swaps it once, however many windows report the same flip;
    /// this window then re-runs the full `apply_config` so its file panes
    /// re-bake — more than a flip needs, but it is rare, and a narrower path
    /// would be a second sync site for the config mirrors.
    pub(in crate::workspace) fn on_system_appearance_changed(
        &mut self,
        appearance: gpui::WindowAppearance,
        cx: &mut Context<Self>,
    ) {
        crate::ui::theme::note_system_appearance(appearance, cx);
        let user = crate::settings_store::SettingsStore::global(cx).user_arc();
        if user.theme.ui_preset == daruda_config::ui_theme_presets::SYSTEM {
            self.apply_store_config(cx);
        }
    }

    /// Reload config from the live store. Only wired up in tests —
    /// production goes through the `observe_global::<SettingsStore>`
    /// subscription installed in `new_with_project`.
    #[cfg(test)]
    pub fn reload_config(&mut self, user: &daruda_config::Config, cx: &mut Context<Self>) {
        let proj = self
            .active_project()
            .map(|p| daruda_store::project::Project::from_path(p.root.clone()));
        let effective = effective_config_for(proj.as_ref(), user, &self.data_dir);
        self.apply_config(&effective, cx);
    }

    /// Apply a reloaded config to all running panes — through
    /// [`Self::apply_store_config`] when the settings store changes (a
    /// file-watch tick or a Settings save) or the OS appearance flips.
    ///
    /// **UI theme:** Workspace does *not* swap the live `DarudaTheme` — the
    /// `crate::ui::theme` state does, told by the settings observer
    /// (`globals.rs`) and the window appearance observer. Keeping the
    /// swap out avoids Workspace tests (built without the full
    /// `gpui_component::init` chain) painting into uninitialised Globals.
    pub fn apply_config(&mut self, config: &daruda_config::Config, cx: &mut Context<Self>) {
        // What moved is read before anything is written: every pass below
        // compares against the values this reload replaces.
        let (delta, mirrors) = self.config_delta(config, cx);
        self.store_config_fields(config, mirrors);
        self.apply_config_to_agent_chat_defaults(delta.telegram_recipient, cx);
        self.apply_config_to_input_dock(config, cx);
        self.apply_config_to_terminals(config, cx);
        self.apply_config_to_file_tree(&delta, cx);
        // The shared surface goes to the app-wide globals before anything
        // that rebuilds against them — chat embeds and file panes.
        self.write_shared_surface_globals(config, cx);
        self.rebuild_agent_chat_for_config(&delta, cx);
        self.rebuild_file_panes_for_config(&delta, cx);
        if delta.claude_status_enabled {
            self.refresh_jsonl_watcher(cx);
        }
        self.defer_window_config_passes(cx);
        cx.notify();
    }

    /// The passes that need this window's `&mut Window` — the dock height,
    /// and the translated labels and placeholders — run once the current
    /// update ends. `apply_config` is also reached from *inside* this window's
    /// update (its appearance observer, a screenshot's theme pass), where the
    /// window is checked out and re-entering it fails with "window not found".
    fn defer_window_config_passes(&mut self, cx: &mut Context<Self>) {
        let ws = cx.weak_entity();
        cx.defer(move |cx| {
            // SILENT-OK: a workspace closed before its reload's window work
            // ran has no window left to update.
            let Some(ws) = ws.upgrade() else {
                return;
            };
            ws.update(cx, |ws, cx| {
                ws.resync_input_dock_height(cx);
                // `apply_locale_str` ran before the reload, so
                // `rust_i18n::locale()` already reflects the new language.
                ws.refresh_locale_strings(cx);
            });
        });
    }

    /// Re-apply translated strings to widgets whose labels are captured at
    /// construction (InputState placeholders, InputPanel button labels), in
    /// one place per language switch. Uses `try_update_workspace_window` for a
    /// live `&mut Window` since `apply_config` runs from `observe_global` (no
    /// window in scope) yet `set_placeholder` requires one.
    fn refresh_locale_strings(&mut self, cx: &mut Context<Self>) {
        let git_commit_input = self.git_commit_input.clone();
        let skill_search_input = self.skill_search_input.clone();
        let task_search_input = self.task_search_input.clone();
        let task_edit_inputs = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .filter_map(|pane| {
                pane.task_edit_content().map(|task_edit| {
                    (
                        task_edit.title_input.clone(),
                        task_edit.branch_input.clone(),
                        task_edit.prompt_state.clone(),
                        task_edit.notes_state.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let handle = self.window_handle;
        // Keep the amend-mode labels if a language switch lands mid-amend.
        let amend_mode = self.is_amend_mode();

        crate::windows::try_update_workspace_window(
            handle,
            cx,
            "refresh_locale_strings",
            |window, cx| {
                // Git Changes commit input — placeholder + button + dropdown.
                git_commit_input.update(cx, |panel, cx| {
                    panel.area.update(cx, |input, cx| {
                        input.set_placeholder(s::git_commit_placeholder(), window, cx);
                    });
                    let (primary, dropdown) = if amend_mode {
                        (s::git_amend_btn(), s::git_cancel_amend())
                    } else {
                        (s::git_commit_btn(), s::ctx_git_commit_amend())
                    };
                    panel.set_action_label("commit", primary, cx);
                    panel.set_action_dropdown_label("commit", 0, dropdown, cx);
                });

                // Skills search input — placeholder.
                skill_search_input.update(cx, |input, cx| {
                    input.set_placeholder(s::skills_search_placeholder(), window, cx);
                });

                // Task search input — placeholder.
                task_search_input.update(cx, |input, cx| {
                    input.set_placeholder(s::task_search_placeholder(), window, cx);
                });

                // Every open Task Edit pane, including panes parked in
                // inactive lanes, owns four locale-dependent placeholders.
                for (title, branch, prompt, notes) in task_edit_inputs {
                    title.update(cx, |input, cx| {
                        input.set_placeholder(s::task_edit_title_placeholder(), window, cx);
                    });
                    branch.update(cx, |input, cx| {
                        input.set_placeholder(s::task_edit_branch_placeholder(), window, cx);
                    });
                    prompt.update(cx, |input, cx| {
                        input.set_placeholder(s::task_edit_prompt_placeholder(), window, cx);
                    });
                    notes.update(cx, |input, cx| {
                        input.set_placeholder(s::task_edit_notes_placeholder(), window, cx);
                    });
                }
            },
        );

        // Terminal fallback titles and untitled Task Edit tabs are cached by
        // the workspace, so refresh them separately from the widget strings.
        for pane in self
            .main_area
            .runtimes
            .values_mut()
            .flat_map(|runtime| runtime.panes.iter_mut())
        {
            pane.refresh_locale_dependent_title(cx);
        }

        // Re-sync the bottom input placeholder: a language switch or
        // `use_modifier_to_send` toggle may change its copy.
        self.refresh_terminal_input_placeholder(cx);
    }

    /// Derive the bottom-input placeholder from the focused pane's kind and
    /// the agent mode / modifier-key policy, then push it to the widget.
    /// Re-enters the window via `try_update_workspace_window` for
    /// `set_placeholder`. Callers already holding a `&mut Window` (e.g.
    /// `focus_pane`) should call [`Workspace::apply_input_placeholder`]
    /// instead to avoid nested window re-entry.
    pub(in crate::workspace) fn refresh_terminal_input_placeholder(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let placeholder = self.compute_input_placeholder(cx);
        let terminal_input = self.terminal_input.clone();
        let handle = self.window_handle;
        crate::windows::try_update_workspace_window(
            handle,
            cx,
            "refresh_terminal_input_placeholder",
            |window, cx| {
                terminal_input.update(cx, |state, cx| {
                    state.set_placeholder(placeholder, window, cx);
                });
            },
        );
    }

    /// Push the context-derived placeholder to the bottom input using the
    /// live `window`. Use this from paths that already hold `&mut Window`
    /// (e.g. `focus_pane`) to avoid nested `update_window` re-entry.
    pub(in crate::workspace) fn apply_input_placeholder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = self.compute_input_placeholder(cx);
        let terminal_input = self.terminal_input.clone();
        terminal_input.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx);
        });
    }

    /// Derive the bottom-input placeholder string from the current focused-pane
    /// context. Pure read — no window or side-effects required.
    fn compute_input_placeholder(&self, cx: &Context<Self>) -> String {
        let focused_id = self.active_runtime().focused_pane_id;
        let is_agent = self.is_agent_chat_pane(focused_id);
        let mode_name: Option<String> = if is_agent {
            self.agent_chat_view(focused_id).and_then(|v| {
                v.read(cx)
                    .session_config
                    .current_mode_name()
                    .map(str::to_string)
            })
        } else {
            None
        };
        s::bottom_input_placeholder_for_context(
            is_agent,
            mode_name.as_deref(),
            self.agent.use_modifier_to_send,
        )
    }
}

/// Layer the project-local override on top of the user-global config
/// and return the resolved [`daruda_config::Config`].
pub(in crate::workspace) fn effective_config_for(
    project: Option<&daruda_store::project::Project>,
    user: &daruda_config::Config,
    data_dir: &std::path::Path,
) -> daruda_config::Config {
    let project_cfg = project
        .map(|p| daruda_config::ProjectConfig::load_in(data_dir, &p.root))
        .unwrap_or_default();
    user.clone().resolve(&project_cfg)
}

/// The terminal crate cannot see `daruda_config`, so the shape crosses here.
pub(super) fn cursor_shape_from(style: daruda_config::CursorStyle) -> daruda_terminal::CursorShape {
    match style {
        daruda_config::CursorStyle::Block => daruda_terminal::CursorShape::Block,
        daruda_config::CursorStyle::Underline => daruda_terminal::CursorShape::Underline,
        daruda_config::CursorStyle::Bar => daruda_terminal::CursorShape::Bar,
    }
}

/// Build a [`TerminalConfig`] from the resolved app config. Single source
/// of truth for the config → terminal-config mapping: both pane creation
/// and reload call this, so a config-derived field is wired in one place.
///
/// No `..TerminalConfig::default()` — every field is named so the compiler
/// rejects an incomplete mapping; the not-yet-wired fields are spelled out
/// explicitly to keep that gap visible.
pub(in crate::workspace) fn terminal_config_from(
    config: &daruda_config::Config,
) -> daruda_terminal::TerminalConfig {
    let colors = config.effective_colors();
    let mut c = daruda_terminal::TerminalConfig {
        // ── wired to daruda_config ──
        default_fg: ghostty_vt::Rgb {
            r: colors.foreground.r,
            g: colors.foreground.g,
            b: colors.foreground.b,
        },
        default_bg: ghostty_vt::Rgb {
            r: colors.background.r,
            g: colors.background.g,
            b: colors.background.b,
        },
        palette: Some(colors.to_ansi_palette()),
        font_size: config.font.terminal.size,
        vertical_spacing: config.font.terminal.line_height,
        horizontal_spacing: config.font.terminal.cell_width,
        inset_x: config.font.terminal.inset_x,
        inset_y: config.font.terminal.inset_y,
        max_scrollback: config.scrollback.max_rows,
        background_alpha: config.window.opacity,
        osc1337_max_bytes: config.clipboard.streaming_max_bytes,
        natural_text_editing: config.shell.natural_text_editing,
        default_cursor_shape: cursor_shape_from(config.cursor.style),
        // ── not yet wired to daruda_config (named to force completeness) ──
        update_window_title: true,
        track_cwd: true,
        visual_bell: false,
        prompt_jump_scroll: daruda_terminal::PromptJumpScroll::AlwaysTop,
    };
    c.clamp_font_settings();
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_config_honors_scrollback_max_rows() {
        let mut c = daruda_config::Config::default();
        c.scrollback.max_rows = 5000;
        // Regression guard: the creation site must honor the user's
        // scrollback value immediately, not only after a config reload.
        assert_eq!(terminal_config_from(&c).max_scrollback, 5000);
    }

    #[test]
    fn terminal_config_clamps_font_size() {
        let mut c = daruda_config::Config::default();
        c.font.terminal.size = 1000.0;
        assert_eq!(
            terminal_config_from(&c).font_size,
            daruda_terminal::FONT_SIZE_MAX
        );
    }

    #[test]
    fn terminal_config_carries_the_cursor_style() {
        let mut c = daruda_config::Config::default();
        c.cursor.style = daruda_config::CursorStyle::Bar;
        assert_eq!(
            terminal_config_from(&c).default_cursor_shape,
            daruda_terminal::CursorShape::Bar
        );
    }
}
