use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{App, Context, Entity, FocusHandle, Pixels, Point, SharedString, Window};

use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

use super::adapter::build_popup_menu;
use super::context::{ClickInfo, LaneAccess, PaneMenuContext, PaneMenuKind, PaneRole, SendTarget};
use super::sections::compose;

impl Workspace {
    pub(in crate::workspace) fn open_pane_context_menu_at(
        &mut self,
        pane_id: PaneId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let click = self.terminal_click_info(pane_id, position, window, cx);
        let Some(context) = self.begin_pane_menu(pane_id, click, window, cx) else {
            return;
        };
        let action_context = self.pane_menu_action_context(pane_id, cx);
        let entries = compose(&context);
        let menu = build_popup_menu(entries, action_context, cx.entity().downgrade(), window, cx);
        self.open_context_menu(position, menu, cx);
    }

    /// Snapshot the menu's inputs, then point the model's focused pane at
    /// `pane_id`.
    ///
    /// Order matters twice over. The selection is read first because moving
    /// focus can clear it. And focus moves through `set_menu_target_pane`,
    /// **not** `focus_pane_on_click`: the latter also runs `focus_pane`,
    /// which surfaces the bottom dock and lazily connects an idle Agent chat
    /// session. Right-click is an inspection gesture and must not spawn an
    /// agent. Only the model field matters here — `split_focused_pane_kind`
    /// and `toggle_zoom_pane` read `focused_pane_id`, while keyboard focus
    /// belongs to the menu for as long as it is open.
    pub(super) fn begin_pane_menu(
        &mut self,
        pane_id: PaneId,
        click: Option<ClickInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneMenuContext> {
        let snapshot = self.pane_menu_kind_and_selection(pane_id, cx)?;
        let role = self.pane_role(pane_id);
        let lane = if self.active_lane_is_inaccessible() {
            LaneAccess::Inaccessible
        } else {
            LaneAccess::Accessible
        };
        let send_targets = self.send_targets_for(&snapshot.kind, cx);

        self.set_menu_target_pane(pane_id, window, cx);

        Some(PaneMenuContext {
            pane_id,
            role,
            lane,
            selection: snapshot.selection,
            click,
            send_targets,
            kind: snapshot.kind,
        })
    }

    fn terminal_click_info(
        &self,
        pane_id: PaneId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ClickInfo> {
        let view = self.terminal_view_for_pane(pane_id)?;
        let link = view.read(cx).link_at_window_position(position, window);
        let annotation = view
            .read(cx)
            .annotation_at_window_position(position, window);
        if link.is_none() && annotation.is_none() {
            None
        } else {
            Some(ClickInfo { link, annotation })
        }
    }

    fn pane_menu_kind_and_selection(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<PaneMenuSnapshot> {
        let pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)?;
        match &pane.content {
            PaneContent::Terminal(content) => {
                let view = content.view.clone();
                let (selection, annotation_range) = view.update(cx, |view, cx| {
                    (
                        view.selection_text(cx).map(SharedString::from),
                        view.selection_single_line_range(),
                    )
                });
                Some(PaneMenuSnapshot {
                    selection,
                    kind: PaneMenuKind::Terminal { annotation_range },
                })
            }
            PaneContent::AgentChat(content) => {
                let view = content.view.clone();
                let selection = Self::agent_chat_selection(&view, cx);
                let busy = view.read(cx).is_busy();
                Some(PaneMenuSnapshot {
                    selection,
                    kind: PaneMenuKind::AgentChat { busy },
                })
            }
            PaneContent::File(_) | PaneContent::TaskEditPane(_) => Some(PaneMenuSnapshot {
                selection: None,
                kind: PaneMenuKind::Other,
            }),
        }
    }

    /// Selection text owned by *this* pane.
    ///
    /// `active_text_selection` is a single global slot, so with two agent
    /// chats open it can hold a selection made in the other one. Gate it on
    /// the selected block sitting inside this pane's list viewport —
    /// otherwise a right-click here would offer to send text from there.
    fn agent_chat_selection(view: &Entity<AgentChatView>, cx: &App) -> Option<SharedString> {
        let handle = crate::ui::active_text_selection(cx)?;
        let pane_bounds = view.read(cx).list_bounds?;
        if !pane_bounds.intersects(&handle.block_bounds(cx)) {
            return None;
        }
        handle
            .selection_text(cx)
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .map(SharedString::from)
    }

    /// A pane is `Solo` when it is the only leaf in **its own** tab. Resolved
    /// through `tab_index_for_pane` rather than the active tab so the answer
    /// stays correct for a pane in a background tab (`send_targets` already
    /// collects those).
    fn pane_role(&self, pane_id: PaneId) -> PaneRole {
        let Some(tab) = self
            .tab_index_for_pane(pane_id)
            .and_then(|index| self.active_runtime().tabs.get(index))
        else {
            return PaneRole::Solo;
        };
        if tab.layout.leaf_count() > 1 {
            PaneRole::InSplit {
                zoomed: self.main_area.zoomed_pane_id == Some(pane_id),
            }
        } else {
            PaneRole::Solo
        }
    }

    pub(super) fn pane_menu_action_context(
        &self,
        pane_id: PaneId,
        cx: &App,
    ) -> Option<FocusHandle> {
        self.active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| match &pane.content {
                PaneContent::Terminal(content) => {
                    Some(content.view.read(cx).focus_handle().clone())
                }
                PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => {
                    None
                }
            })
    }

    fn send_targets_for(&self, source_kind: &PaneMenuKind, cx: &App) -> Vec<SendTarget> {
        let wants_agent = matches!(source_kind, PaneMenuKind::Terminal { .. });
        let wants_terminal = matches!(source_kind, PaneMenuKind::AgentChat { .. });
        if !wants_agent && !wants_terminal {
            return Vec::new();
        }

        let active_tab = self.active_runtime().active_tab_index;
        self.active_runtime()
            .panes
            .iter()
            .filter(|pane| match &pane.content {
                PaneContent::AgentChat(_) => wants_agent,
                PaneContent::Terminal(_) => wants_terminal,
                PaneContent::File(_) | PaneContent::TaskEditPane(_) => false,
            })
            .filter_map(|pane| {
                let tab_index = self.tab_index_for_pane(pane.id)?;
                Some(SendTarget {
                    pane_id: pane.id,
                    label: self.send_target_label(pane.id, tab_index, active_tab, cx),
                })
            })
            .collect()
    }

    fn tab_index_for_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.active_runtime()
            .tabs
            .iter()
            .position(|tab| tab.layout.pane_ids().contains(&pane_id))
    }

    fn send_target_label(
        &self,
        pane_id: PaneId,
        tab_index: usize,
        active_tab: usize,
        cx: &App,
    ) -> SharedString {
        let pane_label = self
            .active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .map(|pane| pane.title(cx))
            .unwrap_or_else(|| SharedString::from(s::ctx_send_target_pane_fallback(pane_id)));
        if tab_index == active_tab {
            return pane_label;
        }
        SharedString::from(format!(
            "{} - {}",
            pane_label.as_ref(),
            self.tab_label(tab_index, cx).as_ref()
        ))
    }

    fn tab_label(&self, tab_index: usize, cx: &App) -> SharedString {
        let Some(tab) = self.active_runtime().tabs.get(tab_index) else {
            return SharedString::from(s::ctx_send_target_tab_fallback(tab_index + 1));
        };
        if let Some(label) = tab.user_label.clone() {
            return label;
        }
        self.active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == tab.last_focused_pane)
            .and_then(|pane| pane.display_cwd().or_else(|| Some(pane.title(cx))))
            .unwrap_or_else(|| SharedString::from(s::ctx_send_target_tab_fallback(tab_index + 1)))
    }

    pub(super) fn scroll_agent_chat_to_bottom(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |view, cx| view.scroll_to_bottom(cx));
            self.bump_activity(pane_id);
        }
    }

    pub(super) fn open_pane_menu_link(&mut self, url: String, cx: &mut Context<Self>) {
        if let Err(err) = open::that_detached(&url) {
            let report = ErrorReport::new(s::ctx_open_link_failed_title())
                .severity(ErrorSeverity::Warning)
                .message(s::ctx_open_link_failed_message())
                .with_context("url", url)
                .with_context("error", format!("{err}"))
                .dedup("pane_menu.open_link_failed")
                .at(file!(), line!())
                .build();
            self.report_error(report, cx);
        }
    }

    pub(super) fn report_pane_menu_send_failed(&mut self, cx: &mut Context<Self>) {
        let report = ErrorReport::new(s::ctx_send_selection_failed_title())
            .severity(ErrorSeverity::Info)
            .message(s::ctx_send_selection_failed_message())
            .dedup("pane_menu.send_selection_failed")
            .at(file!(), line!())
            .build();
        self.report_error(report, cx);
    }
}

struct PaneMenuSnapshot {
    selection: Option<SharedString>,
    kind: PaneMenuKind,
}
