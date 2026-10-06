//! The passes `Workspace::apply_config` runs, one per domain a config
//! reload reaches, and the [`ConfigDelta`] that tells each what moved.
//!
//! `apply_config` itself only orders them. The order matters in two places:
//! everything reads its "changed?" answer from the delta, taken before any
//! field is written, and the app-wide theme globals are written before the
//! passes that rebuild against them.

use gpui::Context;

use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::transcript_defaults::TranscriptDefaults;

use super::ConfigMirrors;
use super::config_ops::{cursor_shape_from, terminal_config_from};

/// What one config reload moved, read before anything is written — from
/// this window's own fields and mirrors, never from the app-wide globals the
/// reload also writes (every window writes those, so they would tell only the
/// first window that anything changed).
pub(super) struct ConfigDelta {
    painted_ui_preset: bool,
    syntax_theme: bool,
    preferred_editor: bool,
    pub(super) telegram_recipient: bool,
    files_filter: bool,
    files_icon: bool,
    panels_grid: bool,
    editor_font: bool,
    agent_chat_font: bool,
    agent_chat_reading_width: bool,
    window_opacity: bool,
    /// The terminal's foreground or background — the colours the chat pane
    /// and the file viewer bake into their mermaid rasters and diff rows.
    terminal_palette: bool,
    pub(super) claude_status_enabled: bool,
}

impl ConfigDelta {
    /// The chat diff embeds bake their palette in, so they only follow a
    /// palette move through a rebuild. Both of these move it: the terminal
    /// colours feed the hunk rows *and* the light/dark syntax variant, and the
    /// syntax-palette name feeds the tokens. A UI-preset switch is not here —
    /// it re-sets the `DarudaTheme` global, which each view observes itself.
    fn agent_chat_diff_palette(&self) -> bool {
        self.terminal_palette || self.syntax_theme
    }
}

impl Workspace {
    /// The delta, and the mirrors the reload leaves behind.
    pub(super) fn config_delta(
        &self,
        config: &daruda_config::Config,
        cx: &Context<Self>,
    ) -> (ConfigDelta, ConfigMirrors) {
        let mirrors = ConfigMirrors::from_config(
            config,
            crate::ui::theme::painted_ui_preset(&config.theme.ui_preset, cx),
        );
        let (was, now) = (&self.mirrors, &mirrors);
        let (was_surface, now_surface) = (&was.shared_surface, &now.shared_surface);
        let delta = ConfigDelta {
            painted_ui_preset: was.painted_ui_preset != now.painted_ui_preset,
            syntax_theme: self.syntax_theme != config.file_viewer.syntax_theme,
            // The chat diff header names this editor in its open-externally
            // tooltip, so a change has to dirty those cached views.
            preferred_editor: self.preferred_editor != config.editor.preferred,
            telegram_recipient: self.telegram.authorized_chat_id
                != config.telegram.authorized_chat_id,
            files_filter: was.files_show_hidden != now.files_show_hidden
                || was.files_use_gitignore != now.files_use_gitignore,
            files_icon: was.files_icon_color_mode != now.files_icon_color_mode,
            panels_grid: was.panels_grid_columns != now.panels_grid_columns,
            editor_font: was_surface.editor_font != now_surface.editor_font,
            agent_chat_font: was_surface.agent_chat_font != now_surface.agent_chat_font,
            agent_chat_reading_width: was_surface.agent_chat_reading_width
                != now_surface.agent_chat_reading_width,
            window_opacity: was_surface.window_opacity != now_surface.window_opacity,
            terminal_palette: was_surface.terminal_bg != now_surface.terminal_bg
                || was_surface.terminal_fg != now_surface.terminal_fg,
            claude_status_enabled: self.claude.claude_status_enabled != config.claude_status.enable,
        };
        (delta, mirrors)
    }

    /// The plain field copies, and the mirrors. No side effects: the passes
    /// after this one read the new values.
    pub(super) fn store_config_fields(
        &mut self,
        config: &daruda_config::Config,
        mirrors: ConfigMirrors,
    ) {
        // A reload may create or remove the active project's config layer;
        // drop the memo so the status-bar dot re-stats on the next render.
        self.cached_project_config = None;
        // The single config → terminal-config mapping; its resolved colours
        // patch every live pane in `apply_config_to_terminals`.
        self.terminal_config = terminal_config_from(config);
        self.font_family = config.font.terminal.family.clone();
        self.shell_program = config.shell.program.clone();
        self.syntax_theme = config.file_viewer.syntax_theme.clone();
        self.agent_reader_defaults =
            crate::workspace::main_area::agent_chat_pane::transcript_defaults::ReaderDefaults::from_config(
                &config.agent,
            );
        self.file_viewer_preview_tab = config.file_viewer.preview_tab;
        self.preferred_editor = config.editor.preferred.clone();
        self.notifications = config.notifications.clone();
        self.git_config = config.git.clone();
        self.telegram = config.telegram.clone();
        self.clipboard = config.clipboard.clone();
        self.agent = config.agent.clone();
        self.agents = config.resolved_agents().into();
        self.flow_config = config.flow.clone();
        self.session_hosts = config.session_hosts.clone();
        self.session_host_tombstones = config.session_host_tombstones.clone();
        self.claude.usage_poll = config.usage.poll.clone();
        // Read by the notification-push freshness gate.
        self.claude.stale_threshold_secs = config.claude_status.stale_threshold_secs;
        self.claude.claude_status_enabled = config.claude_status.enable;
        self.mirrors = mirrors;
    }

    /// Each chat pane's name and the transcript defaults it follows until the
    /// user overrides them — applied here as well as at pane creation, so an
    /// open, untouched pane tracks a config edit live. Resolved per pane: the
    /// defaults are per agent, and a window holds panes on different agents.
    pub(super) fn apply_config_to_agent_chat_defaults(
        &mut self,
        telegram_recipient_changed: bool,
        cx: &mut Context<Self>,
    ) {
        let agent_names = self
            .agents
            .iter()
            .map(|agent| (agent.id.clone(), agent.name.clone()))
            .collect::<Vec<_>>();
        for (_, view) in self.every_agent_chat() {
            view.update(cx, |view, cx| {
                if telegram_recipient_changed {
                    view.permissions_told_to_phone.clear();
                }
                let name = agent_names
                    .iter()
                    .find(|(id, _)| id == &view.agent_id)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| view.agent_id.clone());
                if view.agent_name != name {
                    view.agent_name = name;
                    cx.notify();
                }
                let defaults = TranscriptDefaults::resolve(
                    self.agents.iter().find(|a| a.id == view.agent_id),
                    self.agent_reader_defaults,
                );
                view.reseed_transcript_defaults(&defaults, cx);
            });
        }
    }

    /// Each live chat session follows an edited `default_model` /
    /// `default_mode` of its agent, unless its user picked that axis — the
    /// same rule a connect applies, compared against `previous_agents`.
    pub(super) fn apply_config_to_agent_sessions(
        &mut self,
        previous_agents: &[daruda_config::AgentDefinition],
        cx: &mut Context<Self>,
    ) {
        let mut mode_switched = false;
        for (_, view) in self.every_agent_chat() {
            let was = view.read(cx).session_preferences(previous_agents);
            let now = view.read(cx).session_preferences(&self.agents);
            if was != now {
                mode_switched |= view.update(cx, |view, cx| {
                    view.follow_session_preferences(&was, &now, cx)
                });
            }
        }
        // The bottom-input placeholder names the focused pane's mode.
        if mode_switched {
            self.refresh_terminal_input_placeholder(cx);
        }
    }

    /// The bottom input's auto-grow cap. The dock height that follows it
    /// needs the window, so it is [`Self::resync_input_dock_height`]'s, run
    /// from `apply_config`'s deferred window work.
    pub(super) fn apply_config_to_input_dock(
        &mut self,
        config: &daruda_config::Config,
        cx: &mut Context<Self>,
    ) {
        // Also baked in at construction. No `&mut Window` needed, so inline.
        let new_max_rows = usize::from(config.agent.input_max_rows);
        self.terminal_input
            .update(cx, |s, _cx| s.set_auto_grow(1, new_max_rows));
    }

    /// Resync the dock height after an auto-grow cap change (idempotent via
    /// guard). `window.defer` pushes the entity-borrowing work past the window
    /// update this re-enters.
    pub(super) fn resync_input_dock_height(&mut self, cx: &mut Context<Self>) {
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
    }

    /// Patch every terminal view (font, colours, opacity, cursor, input)
    /// across every lane, not just the active one, so parked terminals follow
    /// too. `set_font` / `apply_font_settings` only invalidate the shape cache
    /// (no resize, no `last_viewport` read), so a never-painted view is safe —
    /// its geometry recomputes on its next paint.
    pub(super) fn apply_config_to_terminals(
        &mut self,
        config: &daruda_config::Config,
        cx: &mut Context<Self>,
    ) {
        let fg = self.terminal_config.default_fg;
        let bg = self.terminal_config.default_bg;
        let pal = self
            .terminal_config
            .palette
            .expect("terminal_config_from always sets palette");
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
                    config.font.terminal.size,
                    config.font.terminal.line_height,
                    config.font.terminal.cell_width,
                );
                view.apply_colors(fg, bg, &pal);
                view.set_background_alpha(config.window.opacity);
                view.apply_inset(config.font.terminal.inset_x, config.font.terminal.inset_y);
                view.set_default_cursor_shape(cursor_shape_from(config.cursor.style));
                view.apply_input_settings(daruda_terminal::InputSettings {
                    natural_text_editing: config.shell.natural_text_editing,
                    osc1337_max_bytes: config.clipboard.streaming_max_bytes,
                    copy_on_select: config.clipboard.copy_on_select,
                });
            });
        }
    }

    /// The Files view's filter and icons, and the macro grid's columns.
    pub(super) fn apply_config_to_file_tree(
        &mut self,
        delta: &ConfigDelta,
        cx: &mut Context<Self>,
    ) {
        if delta.files_filter {
            let refs: Vec<_> = self.lane_file_tree_refs().collect();
            for wt_ref in refs {
                self.invalidate_visible_files_cache(wt_ref);
            }
        }
        // Left-dock and bottom-dock snapshot sources, picked up by the render
        // staging diff on the next render, so a plain notify suffices.
        if delta.files_filter || delta.files_icon || delta.panels_grid {
            cx.notify();
        }
    }

    /// Write the config's shared surface into the app-wide `crate::ui::theme`
    /// globals: editor and chat fonts, the reading width, the background
    /// opacity, and the terminal colours the chat pane mirrors. Every window
    /// writes the same values; what changed is [`ConfigDelta`]'s to say.
    pub(super) fn write_shared_surface_globals(
        &mut self,
        config: &daruda_config::Config,
        cx: &mut Context<Self>,
    ) {
        use crate::ui::theme;
        let fg = self.terminal_config.default_fg;
        let bg = self.terminal_config.default_bg;
        theme::set_editor_font_family(cx, config.font.editor.family.clone());
        theme::set_editor_font_size(cx, config.font.editor.size);
        theme::set_editor_line_height(cx, config.font.editor.line_height);
        theme::set_agent_chat_font_family(cx, config.font.agent_chat.family.clone());
        theme::set_agent_chat_font_size(cx, config.font.agent_chat.size);
        theme::set_agent_chat_line_height(cx, config.font.agent_chat.line_height);
        theme::set_agent_chat_reading_width(cx, config.agent.reading_width);
        theme::set_background_alpha(cx, config.window.opacity);
        theme::set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
        theme::set_agent_chat_fg(cx, fg.r, fg.g, fg.b);
    }

    /// Chat views are cached child entities, so any metric or palette they
    /// bake in has to dirty them explicitly — and some rebuild first.
    pub(super) fn rebuild_agent_chat_for_config(
        &mut self,
        delta: &ConfigDelta,
        cx: &mut Context<Self>,
    ) {
        let diff_palette = delta.agent_chat_diff_palette();
        if !(delta.window_opacity
            || delta.terminal_palette
            || delta.agent_chat_font
            || delta.agent_chat_reading_width
            || diff_palette
            || delta.preferred_editor)
        {
            return;
        }
        let syntax_theme = self.syntax_theme.clone();
        let views: Vec<_> = self
            .every_agent_chat()
            .map(|(_, view)| view.clone())
            .collect();
        for view in views {
            view.update(cx, |view, cx| {
                if delta.terminal_palette {
                    view.assets.clear_mermaid();
                    view.reconcile_mermaid(!crate::ui::theme::agent_chat_syntax_is_light(cx), cx);
                }
                if diff_palette {
                    // Push the new palette name before the pass reads it: an
                    // idle pane sees no ACP event to carry it in.
                    view.set_syntax_theme(&syntax_theme);
                    view.reconcile_embeds_after_theme_change(cx);
                }
                if delta.agent_chat_reading_width && view.content_width.is_reading() {
                    view.list_state.remeasure();
                }
                cx.notify();
            });
        }
    }

    /// Reload open file views whose baked content — diff and markdown spans,
    /// mermaid rasters — captured something the reload moved. At most once:
    /// each later reason is gated on the earlier ones not having reloaded.
    pub(super) fn rebuild_file_panes_for_config(
        &mut self,
        delta: &ConfigDelta,
        cx: &mut Context<Self>,
    ) {
        // A syntax-palette switch re-seeds the editor highlight theme first.
        if delta.syntax_theme {
            crate::ui::theme::set_active_syntax_palette(
                cx,
                crate::ui::theme::SyntaxPalette::from_config_name(&self.syntax_theme),
            );
        }
        // A UI-theme switch flips the syntax palette's light/dark variant; the
        // pane palette feeds markdown raw highlighting, mermaid rasters and
        // diff hunk rows; the editor font feeds every baked line. Pane chrome
        // reads the colours at render time and needs only the final notify.
        if delta.painted_ui_preset
            || delta.syntax_theme
            || delta.terminal_palette
            || delta.editor_font
        {
            self.reload_file_panes(cx);
        }
    }
}
