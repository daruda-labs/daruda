//! GPUI rendering for Workspace.
//!
//! Two distinct UI elements live here — keep them straight (see workspace/mod.rs):
//!   • Tab bar  (top of window)   — built by `build_tab_bar()` (`tab_bar.rs`).
//!                                   Identifiers: `tab_bar`, `tab_cells`, `TAB_BAR_HEIGHT`.
//!   • Pane header (per pane)     — built by `pane_header()`, only in split mode.
//!                                   Identifiers: `pane_header`, `PANE_HEADER_HEIGHT`.

use crate::ui::theme;
use gpui::{Context, IntoElement, KeyContext, Render, SharedString, Window, div, prelude::*, px};

use super::Workspace;
use super::command::flow_picker;
use super::command::lane_switcher;
use super::command::palette as command_palette;
use super::main_area::pane::PaneContent;
use super::status_bar::{self, StatusBarData};
#[allow(unused_imports)]
use super::{FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp};

mod center;
mod docks;
mod landing;
mod menus;
mod snapshots;
mod tab_bar;
mod tab_cells;
mod workspace_frame;

use docks::DockFrame;
pub(in crate::workspace) use menus::{ws_popup_clipboard_item, ws_popup_menu_item};

pub(super) const PANE_HEADER_HEIGHT: f32 = theme::PANE_HEADER_HEIGHT;

/// Centered main-area placeholder shown when the active lane's root
/// directory is inaccessible (Missing / AccessDenied). The render gate
/// in `center_content` keys off `availability` before any tab lookup,
/// so this is shown whenever the active lane is non-`Present` — whether
/// its pane spawn was suppressed (empty `tabs`) or its runtime entry
/// still carries tabs from when the lane was last `Present`. It fills the center
/// with the state message and a Remove affordance. The Remove button is
/// a one-line dispatch into `request_remove_inaccessible_active`
/// (one-way data flow).
/// Amount each inactive split pane's terminal colors blend toward
/// mid-gray (iTerm2's default dim, `colorDimmedBy:0.4`). Pushed onto the
/// pane's `TerminalView` by `refresh_pane_dimming`; alpha is preserved so
/// a transparent background stays equally transparent, just duller.
pub(in crate::workspace) const INACTIVE_PANE_DIM_AMOUNT: f32 = 0.4;

#[cfg(test)]
thread_local! {
    /// How many times the whole-window tree has been rebuilt. A repaint
    /// counter for tests that need to say what a change cost the window,
    /// which is not a claim any assertion on state can make.
    ///
    /// Thread-local, not a global: the harness runs tests in parallel and
    /// every one of them renders a workspace, so a shared counter would
    /// measure the suite rather than the test.
    pub(in crate::workspace) static WORKSPACE_RENDERS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// What a frame needs computed before it can be drawn — and, with
/// [`Workspace::prepare_frame`], the only place a render pass changes
/// anything. `build_frame` takes `&self`, so the compiler is what keeps the
/// two apart rather than a convention someone has to remember.
struct FramePrep {
    body: FrameBody,
    /// Whether the active project's config layer overrides anything
    /// (status-bar dot).
    has_project_config: bool,
}

/// What fills the frame between the title bar and the status bar. Decided
/// once, in `prepare_frame`, so a frame drawn behind Settings cannot reach
/// anything Settings covers.
enum FrameBody {
    Settings(gpui::Entity<daruda_settings::SettingsView>),
    Workspace(DockFrame),
}

impl Workspace {
    /// Everything a frame has to settle before it is drawn.
    fn prepare_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> FramePrep {
        if self.main_area.pending_resize {
            self.resize_all_tabs(window, cx);
        }

        // The sticky account map, the one write site: the Usage tab and the
        // status bar's usage pills both read it. Every frame, not inside the
        // dock staging below, because the status bar stays up behind Settings.
        let focused = self.focused_account();
        let pane_domain =
            super::main_area::pane::AccountDomain::for_pane(&self.focused_account_pane());
        super::sync::limits::observe_focus(
            &mut self.agent_usage.sticky_focus_by_recipe,
            focused,
            pane_domain,
        );

        // Nothing Settings covers may be touched while it is up. gpui filters
        // invalidation to windows that *display* an entity (`App::notify` →
        // `tracked_entities`), and that set is built from what a render pass
        // **accessed** — so staging a dock or reading a chat pane that is off
        // screen re-registers it as displayed, and every notify it sends
        // becomes a full workspace render thrown away. The dock snapshots are
        // re-staged by the render that `close_settings` notifies.
        let body = match self.settings_view() {
            Some(view) => FrameBody::Settings(view.clone()),
            None => FrameBody::Workspace(self.stage_docks(cx)),
        };

        let has_project_config = self.project_layer_overrides();

        FramePrep {
            body,
            has_project_config,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        WORKSPACE_RENDERS.with(|n| n.set(n.get() + 1));
        let prep = self.prepare_frame(window, cx);
        self.build_frame(prep, window, cx)
    }
}

impl Workspace {
    /// Draw the frame `prepare_frame` settled.
    ///
    /// `&self` on purpose: everything a frame changes belongs in
    /// [`Self::prepare_frame`], and taking a shared borrow here is what makes
    /// that a compiler rule instead of a convention. The `&mut Context` is not
    /// a way around it — it addresses the entity, not this state.
    fn build_frame(
        &self,
        prep: FramePrep,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Window title — user override (Window > Edit Window Title…) wins;
        // otherwise show `<project> · <branch>` for the active lane
        // (active project only, no aggregate count). Landing state
        // (no projects) leaves the title untouched.
        if let Some(label) = self.window_runtime.user_label.as_ref() {
            window.set_window_title(label.as_ref());
        } else if let Some(title) = self.window_title_label() {
            window.set_window_title(&title);
        }

        let status_bar = self.build_status_bar(prep.has_project_config, cx);
        let frame = match prep.body {
            FrameBody::Settings(view) => self.build_settings_frame(view, window, cx),
            FrameBody::Workspace(docks) => self.build_workspace_frame(docks, window, cx),
        };

        frame
            .child(status_bar)
            // Toast overlay paints last so it floats above the status
            // bar. ToastLayer owns its queue, expiry sweep, and render;
            // it notifies only itself when toasts change, sparing the
            // full Workspace repaint.
            .child(self.toast_layer.clone())
            .child(command_palette::CommandPaletteOverlay::new(
                self.overlays.palette.clone(),
                cx.listener(|this, _, _, cx| this.close_command_palette(cx)),
                cx.listener(|this, index: &usize, window, cx| {
                    this.pick_palette_row(*index, window, cx)
                }),
            ))
            .child(lane_switcher::LaneSwitcherOverlay::new(
                self.overlays.lane_switcher.clone(),
                cx.listener(|this, _, _, cx| this.close_lane_switcher(cx)),
                cx.listener(|this, index: &usize, window, cx| {
                    this.pick_lane_switcher_row(*index, window, cx)
                }),
            ))
            .child(flow_picker::FlowPickerOverlay::new(
                self.overlays.flow_picker.clone(),
                self.overlays.flow_picker.prompt(),
                crate::surface::strings::flow::picker_empty(),
                crate::surface::strings::flow::stop_prompt(),
                crate::surface::strings::flow::stop_action(),
                cx.listener(|this, _, _, cx| this.close_flow_picker(cx)),
                cx.listener(|this, index: &usize, window, cx| {
                    this.pick_flow_row(*index, window, cx)
                }),
            ))
            // Imperative PopupMenu deploy overlay. `PopupMenu` already
            // dismisses itself on outside click / Escape / confirmed click
            // (see `PopupMenuDeploy`'s dismiss subscription); the callback
            // below just routes that back into `close_context_menu`.
            .when_some(
                self.main_area
                    .popup_menu_deploy
                    .as_ref()
                    .map(|d| (d.menu.clone(), d.position)),
                |el, (menu, position)| {
                    let weak = cx.entity().downgrade();
                    el.child(crate::ui::popup_menu_deferred(
                        &menu,
                        position,
                        move |window, cx| {
                            if let Some(ws) = weak.upgrade() {
                                ws.update(cx, |ws, cx| ws.close_context_menu(window, cx));
                            }
                        },
                    ))
                },
            )
            // gpui_component overlay layers. The window root is
            // `gpui_component::Root`, but `Root::render` only
            // renders the inner view — Dialog/Sheet/Notification layers
            // must be rendered by the inner view explicitly. Without
            // these, `window.open_dialog(...)` registers a dialog into
            // `Root.active_dialogs` but nothing ever paints it.
            .children(gpui_component::Root::render_sheet_layer(window, cx))
            .children(gpui_component::Root::render_dialog_layer(window, cx))
            .children(gpui_component::Root::render_notification_layer(window, cx))
    }

    /// The root both frames share. It answers what survives Settings: the
    /// window's own commands, and every action the chrome still on screen —
    /// title bar and status bar — dispatches.
    fn frame_root(&self, key_ctx: KeyContext, cx: &mut Context<Self>) -> gpui::Div {
        div()
            // Toast overlay (and other absolute-positioned children added
            // later) anchor inside the workspace, not the entire window.
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .key_context(key_ctx)
            .track_focus(&self.window_runtime.focus_handle)
            .capture_key_down(cx.listener(Self::cancel_drag_on_escape))
            // `OpenSettings` is also how the menu moves an already-open view
            // to another section.
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_minimize_window))
            .on_action(cx.listener(Self::on_zoom_window))
            .on_action(cx.listener(Self::on_toggle_full_screen))
            // The status bar's account menu.
            .on_action(cx.listener(Self::on_add_managed_account))
    }

    /// Settings in place of the body, docks included. Nothing it covers is
    /// built — or read, which is what matters to gpui (see `prepare_frame`) —
    /// and only the root's actions are answered, so a chord cannot mutate a
    /// surface nobody can see.
    fn build_settings_frame(
        &self,
        view: gpui::Entity<daruda_settings::SettingsView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let t = theme::current(cx);
        let title_bar_bg = t.title_bar_bg;
        let title_bar_text = t.text_primary;
        // Center the caption against the whole window, independent of the
        // platform's asymmetric controls. The text adds no interactive hitbox.
        let title = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(title_bar_text)
            .child(crate::surface::strings::settings::title());
        let title_bar = crate::title_bar::render(
            crate::title_bar::chrome_for_window(window),
            title_bar_bg,
            None,
            None,
            window,
            cx,
        )
        .relative()
        .child(title);
        self.frame_root(root_key_context(), cx)
            .child(title_bar)
            // `.cached()` is safe because the view notifies itself on every
            // edit (Pitfall 10), so a workspace repaint does not rebuild the
            // form.
            .child(
                gpui::AnyView::from(view).cached(
                    gpui::StyleRefinement::default()
                        .flex_1()
                        .w_full()
                        .overflow_hidden(),
                ),
            )
    }

    fn build_status_bar(
        &self,
        has_project_config: bool,
        cx: &mut Context<Self>,
    ) -> status_bar::StatusBar {
        // The focused pane's title. AgentChat panes suppress it: their own activity bar already shows
        // the same live title, so repeating it here is redundant
        // (Terminal/File/TaskEdit have no such in-pane title, so they keep
        // using this slot).
        let focused_pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == self.active_runtime().focused_pane_id);
        let focused_title = match focused_pane {
            Some(p) if matches!(p.content, PaneContent::AgentChat(_)) => SharedString::default(),
            Some(p) => p.title(cx),
            None => "shell".into(),
        };
        // Same focused-pane lookup for the account slot: Terminal/AgentChat
        // panes carry an `AccountSelection`; File and flow panes don't track
        // an account at all, so the slot is hidden (`None`) rather than
        // showing a misleading "System".
        let focused_pane_id = self.active_runtime().focused_pane_id;
        let weak_workspace = cx.entity().downgrade();
        let login_pending = self.is_login_pending(cx);
        let focused_selection = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_pane_id)
            .and_then(|p| p.account_selection());
        let focused_account = focused_selection.map(|selection| {
            // The dropdown is scoped to the pane's own auth domain, so an
            // agent-chat pane follows its own agent rather than the session's
            // active one.
            let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
                &self.focused_account_pane(),
            );
            status_bar::AccountSlot::resolve(
                focused_pane_id,
                selection,
                domain,
                &self.accounts,
                weak_workspace.clone(),
                login_pending,
            )
        });
        // One usage pill per auth domain, reading the same per-account cache
        // the Usage tab does. `usage_account` picks the same sticky account
        // the pump filed under: a domain's own focused account when its pane
        // is (or was last) live, the ambient login otherwise. Domains with
        // nothing to show drop out here, so the renderer never has to decide
        // whether a pill is worth drawing.
        let usage: Vec<_> = daruda_store::accounts::AccountRecipeId::all()
            .map(|recipe| {
                let account = crate::workspace::sync::limits::usage_account(
                    recipe,
                    &self.agent_usage.sticky_focus_by_recipe,
                );
                let outcome = self
                    .agent_usage
                    .usage_by_account
                    .usage(crate::workspace::agent_usage::UsageKey { recipe, account });
                (recipe, outcome)
            })
            .filter(|(_, outcome)| outcome.has_numbers())
            .collect();
        let flows = self.flow_status_rows(cx);
        let status_data = StatusBarData {
            orchestrator: self.orchestrator_chip_state(cx),
            project_branch: self.active_project_branch_label().map(Into::into),
            is_detached: matches!(self.active_branch_status(), super::BranchStatus::Detached),
            title: focused_title,
            error: self.last_error.clone(),
            has_project_config,
            account: focused_account,
            ports: self.ports.entries.clone(),
            flows,
            ports_status: self.ports.status,
            usage,
            keep_awake: self.keep_awake_view(cx),
            visible: self.mirrors.status_bar.clone(),
            stop_on_overage: crate::overage_policy::stop_on_overage_enabled(cx),
            workspace: weak_workspace.clone(),
        };
        status_bar::StatusBar(status_data)
    }
}

/// The key context both frames start from; the workspace frame adds the
/// focused pane's tokens to it.
fn root_key_context() -> KeyContext {
    let mut key_ctx = KeyContext::default();
    key_ctx.add("Workspace");
    key_ctx
}

#[cfg(test)]
mod tests {
    use super::docks::dock_toggle_icon_path;
    use super::menus::agent_menu_is_flat;
    use crate::workspace::layout::DockPosition;

    #[test]
    fn dock_toggle_icons_follow_panel_position_and_visibility() {
        for (position, closed, open) in [
            (
                DockPosition::Left,
                "icons/ui/dock-to-right.svg",
                "icons/ui/dock-to-right-fill.svg",
            ),
            (
                DockPosition::Bottom,
                "icons/ui/dock-to-bottom.svg",
                "icons/ui/dock-to-bottom-fill.svg",
            ),
            (
                DockPosition::Right,
                "icons/ui/dock-to-left.svg",
                "icons/ui/dock-to-left-fill.svg",
            ),
        ] {
            assert_eq!(dock_toggle_icon_path(position, false), closed);
            assert_eq!(dock_toggle_icon_path(position, true), open);
        }
    }

    #[test]
    fn agent_menu_flat_boundary() {
        // 0..=5 flat, 6+ submenu (AGENT_MENU_FLAT_MAX = 5).
        assert!(agent_menu_is_flat(1));
        assert!(agent_menu_is_flat(5));
        assert!(!agent_menu_is_flat(6));
        assert!(!agent_menu_is_flat(20));
    }
}
