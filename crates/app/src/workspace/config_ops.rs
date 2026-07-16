use gpui::{Context, Window};

use crate::surface::strings as s;
use crate::workspace::Workspace;

impl Workspace {
    /// Reload config from the live store. Only wired up in tests —
    /// production goes through the `observe_global::<SettingsStore>`
    /// subscription installed in `new_with_project`.
    #[cfg(test)]
    pub fn reload_config(&mut self, user: &daruda_config::Config, cx: &mut Context<Self>) {
        let proj = self
            .active_project()
            .map(|p| daruda_store::project::Project::from_path(p.root.clone()));
        let effective = effective_config_for(proj.as_ref(), user);
        self.apply_config(&effective, cx);
    }

    /// Apply a reloaded config to all running panes. Called by the
    /// config file watcher and the Settings window when the TOML changes.
    ///
    /// **UI theme:** Workspace does *not* swap the live `DarudaTheme`
    /// here — the config watcher (`main.rs::spawn_config_watcher`) owns
    /// that via `apply_ui_theme` once per reload, app-wide. Keeping the
    /// swap out avoids Workspace tests (built without the full
    /// `gpui_component::init` chain) painting into uninitialised Globals.
    pub fn apply_config(&mut self, config: &daruda_config::Config, cx: &mut Context<Self>) {
        // A config reload may create/remove the active project's config
        // layer; drop the memo so the status-bar dot re-stats on next render.
        self.cached_project_config = None;
        // Single source of truth for the config → terminal-config mapping;
        // its resolved colors patch live panes so all panes match.
        self.terminal_config = terminal_config_from(config);
        let fg = self.terminal_config.default_fg;
        let bg = self.terminal_config.default_bg;
        let pal = self
            .terminal_config
            .palette
            .expect("terminal_config_from always sets palette");
        self.font_family = config.font.family.clone();
        self.shell_program = config.shell.program.clone();
        let syntax_theme_changed = self.syntax_theme != config.file_viewer.syntax_theme;
        self.syntax_theme = config.file_viewer.syntax_theme.clone();
        self.file_viewer_preview_tab = config.file_viewer.preview_tab;
        self.notifications = config.notifications.clone();
        let previous_telegram_chat_id = self.telegram.authorized_chat_id;
        self.telegram = config.telegram.clone();
        // The bridge just went disabled/unpaired, or the target chat changed:
        // clear held pings rather than deliver old-context messages later.
        if !(self.telegram.enabled && self.telegram.authorized_chat_id.is_some())
            || self.telegram.authorized_chat_id != previous_telegram_chat_id
        {
            self.deferred_telegram.clear();
        }
        self.clipboard = config.clipboard.clone();
        self.agent = config.agent.clone();
        self.agents = config.agents.clone();
        let agent_names = self
            .agents
            .iter()
            .map(|agent| (agent.id.clone(), agent.name.clone()))
            .collect::<Vec<_>>();
        for view in self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter_map(|pane| pane.agent_chat_content().map(|ac| ac.view.clone()))
        {
            view.update(cx, |view, cx| {
                let name = agent_names
                    .iter()
                    .find(|(id, _)| id == &view.agent_id)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| view.agent_id.clone());
                if view.agent_name != name {
                    view.agent_name = name;
                    cx.notify();
                }
            });
        }
        // Keep the `InputState`'s auto-grow cap in sync with the new
        // `input_max_rows` value (also baked in at construction). No
        // `&mut Window` needed, so it runs inline.
        let new_max_rows = usize::from(config.agent.input_max_rows);
        self.terminal_input
            .update(cx, |s, _cx| s.set_auto_grow(1, new_max_rows));
        // Resync dock height after the cap change (idempotent via guard).
        // `apply_config` runs inside `observe_global` with no `&mut Window`
        // and the workspace already borrowed, so re-enter via
        // `try_update_workspace_window` + `window.defer` to push the
        // entity-borrowing work past the current observe callback's borrow.
        let handle = self.window_handle;
        let ws_weak = cx.weak_entity();
        crate::windows::try_update_workspace_window(
            handle,
            cx,
            "apply_config.resync_dock",
            |window, cx| {
                window.defer(cx, move |window, cx| {
                    if let Some(ws) = ws_weak.upgrade() {
                        ws.update(cx, |ws, cx| ws.adapt_dock_to_input_lines(window, cx));
                    }
                });
            },
        );
        self.claude.usage_poll = config.usage.poll.clone();

        // Patch all existing pane views (font + colors + opacity) across
        // every lane, not just the active one, so parked terminals update
        // too. `set_font` / `apply_font_settings` only invalidate the shape
        // cache (no resize / `last_viewport` read), so a parked never-painted
        // view is safe — geometry recomputes on its next paint.
        let font = daruda_terminal::terminal_font_with_family(&self.font_family);
        for pane in self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
        {
            let Some(view) = pane.terminal_view() else {
                continue;
            };
            view.update(cx, |view, _cx| {
                view.set_font(font.clone());
                view.apply_font_settings(
                    config.font.size,
                    config.font.vertical_spacing,
                    config.font.horizontal_spacing,
                );
                view.apply_colors(fg, bg, &pal);
                view.set_background_alpha(config.window.opacity);
                view.apply_inset(config.font.inset_x, config.font.inset_y);
            });
        }
        let new_mirrors = crate::workspace::ConfigMirrors::from_config(config);
        let filter_changed = self.mirrors.files_show_hidden != new_mirrors.files_show_hidden
            || self.mirrors.files_use_gitignore != new_mirrors.files_use_gitignore;
        let icon_changed = self.mirrors.files_icon_color_mode != new_mirrors.files_icon_color_mode;
        let panels_changed = self.mirrors.panels_grid_columns != new_mirrors.panels_grid_columns;
        let theme_changed = self.mirrors.ui_preset != new_mirrors.ui_preset;
        self.mirrors = new_mirrors;
        if filter_changed {
            let refs: Vec<_> = self.file_tree.file_trees.keys().copied().collect();
            for wt_ref in refs {
                self.invalidate_visible_files_cache(wt_ref);
            }
        }
        if icon_changed || panels_changed {
            cx.notify();
            // `panels_grid_columns` is a BottomDockSnapshot source — no left-dock notify needed.
        }
        if filter_changed || icon_changed {
            // These are `LeftDockSnapshot` sources picked up by the render
            // staging diff on the next render, so a plain `cx.notify()` suffices.
            cx.notify();
        }
        // File-viewer editor font size, independent of the terminal font.
        // Mirror to the GPUI-side global *before* any file-pane reload so a
        // re-bake reads the fresh size; the render path reads it directly.
        let editor_font_changed =
            (crate::ui::theme::editor_font_size(cx) - config.font.editor_size).abs() > f32::EPSILON;
        crate::ui::theme::set_editor_font_size(cx, config.font.editor_size);

        // Agent-chat font size, independent of terminal/editor fonts. Mirror
        // to the GPUI-side global the agent-chat render path reads directly; on
        // change, cached `AgentChatView`s are dirtied below (a bare workspace
        // `cx.notify()` would not reach a cached child — render-cost rule §10).
        let agent_chat_font_changed =
            (crate::ui::theme::agent_chat_font_size(cx) - config.font.agent_chat_size).abs()
                > f32::EPSILON;
        crate::ui::theme::set_agent_chat_font_size(cx, config.font.agent_chat_size);

        // Background opacity drives both the terminal pane fill (pushed above)
        // and the agent-chat pane background. Mirror to the GPUI-side global;
        // on change, dirty each cached `AgentChatView` below so its `.cached()`
        // subtree repaints with the new alpha (render-cost rule §10).
        let bg_alpha_changed =
            (crate::ui::theme::background_alpha(cx) - config.window.opacity).abs() > f32::EPSILON;
        crate::ui::theme::set_background_alpha(cx, config.window.opacity);
        // Mirror the terminal fg/bg so the agent-chat pane tracks the terminal
        // color theme on a live reload too.
        let bg_color_changed = crate::ui::theme::set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
        let fg_color_changed = crate::ui::theme::set_agent_chat_fg(cx, fg.r, fg.g, fg.b);
        if bg_alpha_changed || bg_color_changed || fg_color_changed || agent_chat_font_changed {
            let views: Vec<_> = self
                .main_area
                .runtimes
                .values()
                .flat_map(|rt| rt.panes.iter())
                .filter_map(|pane| pane.agent_chat_content().map(|ac| ac.view.clone()))
                .collect();
            for view in views {
                view.update(cx, |_, cx| cx.notify());
            }
        }

        // A UI-theme switch flips the syntax palette's light/dark variant.
        // Reload open file views to recompute baked diff/markdown spans (and
        // re-theme mermaid); raw editors recolour from the re-seeded theme.
        if theme_changed {
            self.reload_file_panes(cx);
        }
        // A syntax-palette switch re-seeds the editor highlight theme and
        // recomputes baked diff/markdown spans by reloading open file panes.
        if syntax_theme_changed {
            crate::ui::theme::set_active_syntax_palette(
                cx,
                crate::ui::theme::SyntaxPalette::from_config_name(&self.syntax_theme),
            );
            self.reload_file_panes(cx);
        }
        // Reload for a standalone editor-font change, gated to avoid a
        // double when a theme / syntax switch above already reloaded.
        if editor_font_changed && !theme_changed && !syntax_theme_changed {
            self.reload_file_panes(cx);
        }
        // Mirror for the notification-push freshness gate.
        self.claude.stale_threshold_secs = config.claude_status.stale_threshold_secs;
        let new_enabled = config.claude_status.enable;
        if new_enabled != self.claude.claude_status_enabled {
            self.claude.claude_status_enabled = new_enabled;
            self.refresh_jsonl_watcher(cx);
        }
        // Refresh locale-dependent widget strings. `apply_locale_str` runs
        // before this method, so `rust_i18n::locale()` already reflects the
        // new language here.
        self.refresh_locale_strings(cx);

        cx.notify();
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
            },
        );

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
                let view = v.read(cx);
                view.modes.as_ref().and_then(|m| {
                    m.available
                        .iter()
                        .find(|av| av.id == m.current)
                        .map(|av| av.name.clone())
                })
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
) -> daruda_config::Config {
    let project_cfg = project
        .map(|p| daruda_config::ProjectConfig::load_for(&p.root))
        .unwrap_or_default();
    user.clone().resolve(&project_cfg)
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
        font_size: config.font.size,
        vertical_spacing: config.font.vertical_spacing,
        horizontal_spacing: config.font.horizontal_spacing,
        inset_x: config.font.inset_x,
        inset_y: config.font.inset_y,
        max_scrollback: config.scrollback.max_rows,
        background_alpha: config.window.opacity,
        osc1337_max_bytes: config.clipboard.streaming_max_bytes,
        natural_text_editing: config.shell.natural_text_editing,
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
        c.font.size = 1000.0;
        assert_eq!(
            terminal_config_from(&c).font_size,
            daruda_terminal::FONT_SIZE_MAX
        );
    }
}
