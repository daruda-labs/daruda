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
    /// here. The config watcher (`main.rs::spawn_config_watcher`) owns
    /// that — it calls `crate::ui::theme::apply_ui_theme` once per
    /// reload, app-wide, so a single config change repaints every
    /// open window. Keeping the swap out of this method means
    /// Workspace tests that build a sub-tree directly (without the
    /// full `gpui_component::init` chain) don't accidentally trigger
    /// a paint that reaches into uninitialised theme Globals.
    pub fn apply_config(&mut self, config: &daruda_config::Config, cx: &mut Context<Self>) {
        // A config reload may create/remove the active project's config
        // layer; drop the memo so the status-bar dot re-stats on next render.
        self.cached_project_config = None;
        // Derive the terminal config once (single source of truth), then reuse
        // its resolved colors to patch live panes — so existing panes and
        // future panes always resolve to identical fg/bg/palette values.
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
        self.telegram = config.telegram.clone();
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
        // `input_max_rows` value. The cap is also baked in at construction
        // (`workspace/mod.rs`); without this update a live reload would leave
        // the editor expanding past the new limit or stopping short of it.
        // `set_auto_grow` requires no `&mut Window`, so it runs inline.
        let new_max_rows = usize::from(config.agent.input_max_rows);
        self.terminal_input
            .update(cx, |s, _cx| s.set_auto_grow(1, new_max_rows));
        // Resync the dock height after the cap change — lowering `input_max_rows`
        // mid-edit would otherwise leave the dock too tall until the next
        // keystroke. `adapt_dock_to_input_lines` is idempotent via its guard.
        // `apply_config` runs inside `observe_global` (no `&mut Window` in scope
        // and the workspace entity is already borrowed), so we re-enter the window
        // via `try_update_workspace_window` and then `window.defer` to push the
        // entity-borrowing work to the next event cycle (after the current observe
        // callback releases its `&mut Workspace` borrow).
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

        // Patch all existing pane views: font + colors + opacity. Iterate
        // every lane's panes (not just the active lane) so a parked lane's
        // terminals pick up the new font/colors too. `set_font` /
        // `apply_font_settings` only invalidate the shape cache (they do
        // not resize or read `last_viewport`), so patching a parked,
        // never-painted view is safe — the new geometry is recomputed on
        // its next paint, after the lane is activated.
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
        // Trigger #7 — left-dock config affecting filter state changed.
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
            // `files_show_hidden` / `files_use_gitignore` / `files_icon_color_mode`
            // are `LeftDockSnapshot` sources; the render staging diff picks them
            // up on the next workspace render, so a plain `cx.notify()` suffices.
            cx.notify();
        }
        // File-viewer editor font size (config `font.editor_size`),
        // independent of the terminal font. Mirror it to the GPUI-side
        // global *before* any file-pane reload below so a re-bake reads
        // the fresh size; the render path reads the global directly.
        let editor_font_changed =
            (crate::ui::theme::editor_font_size(cx) - config.font.editor_size).abs() > f32::EPSILON;
        crate::ui::theme::set_editor_font_size(cx, config.font.editor_size);

        // Agent-chat font size (config `font.agent_chat_size`), independent of
        // the terminal and editor fonts. Mirror it to the GPUI-side global the
        // agent-chat render path reads directly; on a change, the cached
        // `AgentChatView`s are dirtied below (a bare workspace `cx.notify()`
        // would not reach a cached child — render-cost rule §10).
        let agent_chat_font_changed =
            (crate::ui::theme::agent_chat_font_size(cx) - config.font.agent_chat_size).abs()
                > f32::EPSILON;
        crate::ui::theme::set_agent_chat_font_size(cx, config.font.agent_chat_size);

        // Background opacity (config `window.opacity`) drives both the terminal
        // pane fill (pushed per-view above) and the agent-chat pane background.
        // Mirror it to the GPUI-side global the agent-chat render path reads
        // directly; on a change, dirty each cached `AgentChatView` so its
        // `.cached()` subtree repaints with the new alpha (a bare workspace
        // `cx.notify()` would not reach a cached child — render-cost rule §10).
        let bg_alpha_changed =
            (crate::ui::theme::background_alpha(cx) - config.window.opacity).abs() > f32::EPSILON;
        crate::ui::theme::set_background_alpha(cx, config.window.opacity);
        // Mirror the terminal background color (same `bg` the terminal fill
        // uses, resolved above) so the agent-chat pane tracks the terminal
        // color theme on a live reload too.
        let bg_color_changed = crate::ui::theme::set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
        // Mirror the terminal foreground too so the agent-chat pane's text color
        // tracks the terminal color theme (counterpart to the background above).
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

        // A UI-theme switch changes the editor background's lightness, so the
        // syntax palette flips its light/dark variant. Reload open file views
        // to recompute the baked diff/markdown spans (and re-theme mermaid
        // diagrams) for the new surface; raw editors recolour from the
        // re-seeded highlight theme on their own.
        if theme_changed {
            self.reload_file_panes(cx);
        }
        // A syntax-palette switch re-seeds the editor highlight theme (the
        // raw editor repaints from it) and recomputes the baked-in diff /
        // markdown spans by reloading every open file-view pane.
        if syntax_theme_changed {
            crate::ui::theme::set_active_syntax_palette(
                cx,
                crate::ui::theme::SyntaxPalette::from_config_name(&self.syntax_theme),
            );
            self.reload_file_panes(cx);
        }
        // Reload for a standalone editor-font change (a theme / syntax
        // switch above already reloaded, so gate it out to avoid a double).
        if editor_font_changed && !theme_changed && !syntax_theme_changed {
            self.reload_file_panes(cx);
        }
        // Mirrors `claude_status.stale_threshold_secs` for the
        // notification-push freshness gate.
        self.claude.stale_threshold_secs = config.claude_status.stale_threshold_secs;
        // Picks up `claude_status.enable` flips.
        let new_enabled = config.claude_status.enable;
        if new_enabled != self.claude.claude_status_enabled {
            self.claude.claude_status_enabled = new_enabled;
            self.refresh_jsonl_watcher(cx);
        }
        // Refresh locale-dependent widget strings. `apply_locale_str` in
        // `globals::register_settings_observer` runs before this method, so
        // `rust_i18n::locale()` already reflects the new language by the time
        // we get here.
        self.refresh_locale_strings(cx);

        cx.notify();
    }

    /// Re-apply translated strings to all widgets whose labels are captured
    /// at construction time (InputState placeholders, InputPanel button
    /// labels). Called from `apply_config` so every language switch
    /// refreshes them in one place.
    ///
    /// Uses `try_update_workspace_window` to obtain a live `&mut Window`
    /// because `apply_config` is called from `observe_global` which has no
    /// window in scope, yet `InputState::set_placeholder` requires one.
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
    ///
    /// **No `&mut Window` needed at the call site.** This method captures
    /// the placeholder string (which requires only `&Context<Self>`) and then
    /// re-enters the window via `try_update_workspace_window` to call
    /// `set_placeholder`. Callers that *already hold* a `&mut Window`
    /// (i.e. `focus_pane`) should call
    /// [`Workspace::apply_input_placeholder`] instead to avoid nested
    /// window re-entry.
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
/// (`new_with_project_impl`) and reload (`apply_config`) call this, so a new
/// config-derived field can only be wired in one place.
///
/// No `..TerminalConfig::default()` — every field is named so the compiler
/// rejects an incomplete mapping. The four fields that are not yet wired to
/// `daruda_config` are spelled out explicitly to keep that gap visible.
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
        // Guards the startup-drift bug: the creation site used to fall back
        // to `TerminalConfig::default()`'s 10_000 and ignore the user value
        // until the first config reload.
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
