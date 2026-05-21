//! MainArea: TabBar + PaneTree walker + submodule declarations.
//!
//! `render_layout` recurses over the active tab's `PaneLayout`,
//! emitting one `div` per leaf (with optional pane header in split
//! mode and the file-viewer overlay when one is open) and inserting
//! draggable dividers between siblings of a `Split`. Lives next to
//! `pane_header` (only called from here) so the two stay in sync.

pub(in crate::workspace) mod bottom_dock;
pub(in crate::workspace) mod context;
pub(in crate::workspace) mod file_pane_ops;
pub(in crate::workspace) mod file_view_pane;
pub(in crate::workspace) mod nav;
pub(in crate::workspace) mod pane;
pub(in crate::workspace) mod pane_tree;
pub(in crate::workspace) mod prompt_watcher;
pub(in crate::workspace) mod resize;
pub(in crate::workspace) mod tab_ops;
pub(in crate::workspace) mod task_edit_pane;

pub(in crate::workspace) use context::MainAreaContext;

use crate::ui::theme;
use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, ExternalPaths, IntoElement, MouseButton,
    MouseDownEvent, SharedString, div, prelude::*, px,
};

use crate::shell_quote::{Shell, format_paths_for_drop, quote_path};
use crate::surface::strings as s;
use crate::ui::ContextMenuItem as CItem;
use crate::workspace::path_drag::PathDrag;

use self::file_view_pane::render::render_pane_file_viewer;
use self::pane::Pane;
use self::pane_tree::{DIVIDER_PX, PaneId, PaneLayout, SplitDirection};
use super::Workspace;

/// Flex child cell wrapping a pane or nested split in a Split layout.
fn split_cell(is_horizontal: bool, ratio: f32, child_el: AnyElement) -> gpui::Div {
    let base = div()
        .flex_basis(gpui::relative(ratio))
        .flex_shrink()
        .flex_grow();
    if is_horizontal {
        base.h_full().min_w(px(theme::RENDER_MIN_DIM))
    } else {
        base.w_full().min_h(px(theme::RENDER_MIN_DIM))
    }
    .child(child_el)
}

/// Dim overlay drawn on top of inactive panes in split mode.
fn dim_overlay(alpha: f32) -> impl IntoElement {
    div().absolute().inset_0().bg(gpui::black().opacity(alpha))
}

/// 1px visible divider + absolute hit-zone overlay between split siblings.
fn pane_divider(
    is_horizontal: bool,
    leaf_id: PaneId,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let hit = theme::RESIZE_HANDLE_HIT_PX;
    let offset = (DIVIDER_PX - hit) / 2.0;
    let div_id = (
        if is_horizontal { "div-h" } else { "div-v" },
        leaf_id as usize,
    );
    let overlay_cursor = if is_horizontal {
        CursorStyle::ResizeLeftRight
    } else {
        CursorStyle::ResizeUpDown
    };
    let overlay = {
        let base = div()
            .id(div_id)
            .absolute()
            .cursor(overlay_cursor)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                    let anchor: f32 = if is_horizontal {
                        ev.position.x.into()
                    } else {
                        ev.position.y.into()
                    };
                    this.begin_divider_drag(leaf_id, anchor, cx);
                }),
            );
        if is_horizontal {
            base.top_0().bottom_0().left(px(offset)).w(px(hit))
        } else {
            base.left_0().right_0().top(px(offset)).h(px(hit))
        }
    };
    let divider_bg = theme::current(cx).pane_divider_bg;
    if is_horizontal {
        div()
            .relative()
            .flex_none()
            .w(px(DIVIDER_PX))
            .h_full()
            .bg(divider_bg)
            .child(overlay)
    } else {
        div()
            .relative()
            .flex_none()
            .h(px(DIVIDER_PX))
            .w_full()
            .bg(divider_bg)
            .child(overlay)
    }
}

/// Per-pane header shown above each pane in split mode.
/// Mirrors iTerm2's `SessionView` title bar: title left, close × right
/// (hover-only), brighter background when focused. Not the same as the
/// top-level tab bar — see workspace/mod.rs terminology block.
fn pane_header(
    pane_id: PaneId,
    is_focused: bool,
    title: SharedString,
    cwd_basename: Option<SharedString>,
    is_zoomed: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let group_name = SharedString::from(format!("pane-hdr-{}", pane_id));
    let close = crate::ui::button_close(("pane-close", pane_id as usize), group_name.clone(), cx)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.request_close_pane(pane_id, window, cx);
        }));

    let t = theme::current(cx);
    let focused_bg = t.pane_header_focused_bg;
    let focused_text = t.pane_header_focused_text;
    let unfocused_bg = t.pane_header_unfocused_bg;
    let unfocused_text = t.muted_text;
    let cwd_text = t.pane_header_cwd_text;

    div()
        .id(("pane-hdr", pane_id as usize))
        .group(group_name)
        .flex()
        .flex_row()
        .items_center()
        .h(px(theme::PANE_HEADER_HEIGHT))
        .w_full()
        .px(px(theme::PANE_HEADER_PAD_X))
        .gap(px(theme::PANE_HEADER_GAP))
        .text_size(px(theme::PANE_HEADER_FONT_SIZE))
        .when(is_focused, |d| d.bg(focused_bg).text_color(focused_text))
        .when(!is_focused, |d| {
            d.bg(unfocused_bg).text_color(unfocused_text)
        })
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                let ws = cx.entity().downgrade();
                let zoom_label = if is_zoomed {
                    s::ctx_unzoom_pane()
                } else {
                    s::ctx_zoom_pane()
                };
                let items: Vec<CItem> = vec![
                    crate::workspace::render::ws_menu_item(
                        ws.clone(),
                        s::ctx_split_right(),
                        false,
                        |this, win, cx| {
                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                ws.split_focused_pane(SplitDirection::Horizontal, win, cx);
                            });
                        },
                    ),
                    crate::workspace::render::ws_menu_item(
                        ws.clone(),
                        s::ctx_split_down(),
                        false,
                        |this, win, cx| {
                            this.mutate_durable_in(win, cx, |ws, win, cx| {
                                ws.split_focused_pane(SplitDirection::Vertical, win, cx);
                            });
                        },
                    ),
                    CItem::separator(),
                    crate::workspace::render::ws_menu_item(
                        ws.clone(),
                        zoom_label,
                        false,
                        move |this, _win, cx| {
                            this.toggle_zoom_pane(pane_id, cx);
                        },
                    ),
                    CItem::separator(),
                    crate::workspace::render::ws_menu_item(
                        ws.clone(),
                        s::ctx_close_pane(),
                        false,
                        move |this, win, cx| {
                            this.request_close_pane(pane_id, win, cx);
                        },
                    ),
                ];
                this.open_context_menu(ev.position, items, cx);
            }),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::PANE_HEADER_INNER_GAP))
                .overflow_hidden()
                .whitespace_nowrap()
                .child(div().overflow_hidden().whitespace_nowrap().child(title))
                .when_some(cwd_basename, |d, name| {
                    d.child(
                        div()
                            .text_color(cwd_text)
                            .text_size(px(theme::PANE_HEADER_CWD_FONT_SIZE))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(name),
                    )
                }),
        )
        .child(close)
}

/// Render the layout tree as GPUI elements.
#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn render_layout(
    layout: &PaneLayout,
    panes: &[Pane],
    focused_pane_id: PaneId,
    has_splits: bool,
    dim_alpha: f32,
    font_family: SharedString,
    zoomed_pane_id: Option<PaneId>,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    match layout {
        PaneLayout::Pane(pane_id) => {
            let id = *pane_id;
            let Some(pane) = panes.iter().find(|p| p.id == id) else {
                return div().into_any_element();
            };
            let is_focused = has_splits && id == focused_pane_id;
            let mut root = div()
                .id(("pane", id as usize))
                .size_full()
                .flex()
                .flex_col()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if this.main_area.focused_pane_id != id {
                            this.main_area.focused_pane_id = id;
                            if let Some(tab) =
                                this.main_area.tabs.get_mut(this.main_area.active_tab_index)
                            {
                                tab.last_focused_pane = id;
                            }
                            this.bump_activity(id);
                            this.focus_pane(id, window, cx);
                            cx.notify();
                        }
                    }),
                );

            if has_splits {
                let basename = pane.display_cwd();
                let is_zoomed = zoomed_pane_id == Some(id);
                root = root.child(pane_header(
                    id,
                    is_focused,
                    pane.title(),
                    basename,
                    is_zoomed,
                    cx,
                ));
            }

            let content = match &pane.content {
                self::pane::PaneContent::File(f) => div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .when_some(pane.content.wrapper_focus_handle(), |d, fh| {
                        d.track_focus(fh)
                    })
                    .child(render_pane_file_viewer(
                        &f.view,
                        &f.scroll_handle,
                        f.search_input.clone(),
                        font_family.clone(),
                        cx,
                    )),
                self::pane::PaneContent::TaskEditPane(te) => div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .when_some(pane.content.wrapper_focus_handle(), |d, fh| {
                        d.track_focus(fh)
                    })
                    .child(self::task_edit_pane::render(id, te, cx)),
                self::pane::PaneContent::Terminal(t) => {
                    let view_for_path_drag = t.view.clone();
                    let view_for_external = t.view.clone();
                    let mut terminal_area = div()
                        .flex_1()
                        .flex()
                        .min_h(px(theme::RENDER_MIN_DIM))
                        .relative()
                        .drag_over::<PathDrag>(|style, _, _, cx| {
                            style.bg(theme::current(cx).terminal_drop_target_bg)
                        })
                        .drag_over::<ExternalPaths>(|style, _, _, cx| {
                            style.bg(theme::current(cx).terminal_drop_target_bg)
                        })
                        .on_drop::<PathDrag>(cx.listener(
                            move |this, drag: &PathDrag, _window, cx| {
                                let shell = this
                                    .shell_program
                                    .as_deref()
                                    .map(Shell::detect_from_program)
                                    .unwrap_or_default();
                                let quoted = quote_path(&drag.path, shell);
                                view_for_path_drag.update(cx, |view, _| {
                                    view.send_input(quoted.as_bytes());
                                });
                            },
                        ))
                        .on_drop::<ExternalPaths>(cx.listener(
                            move |this, paths: &ExternalPaths, _window, cx| {
                                if paths.paths().is_empty() {
                                    return;
                                }
                                let shell = this
                                    .shell_program
                                    .as_deref()
                                    .map(Shell::detect_from_program)
                                    .unwrap_or_default();
                                let formatted = format_paths_for_drop(paths.paths(), shell);
                                view_for_external.update(cx, |view, _| {
                                    view.send_input(formatted.as_bytes());
                                });
                            },
                        ))
                        .child(t.view.clone());
                    if has_splits && !is_focused && dim_alpha > 0.0 {
                        terminal_area = terminal_area.child(dim_overlay(dim_alpha));
                    }
                    terminal_area
                }
            };
            root.child(content).into_any_element()
        }
        PaneLayout::Split {
            direction,
            children,
            ratios,
        } => {
            let is_horizontal = matches!(direction, SplitDirection::Horizontal);
            let mut container = div().size_full().flex();
            container = if is_horizontal {
                container.flex_row()
            } else {
                container.flex_col()
            };

            let n = children.len();
            for (i, child) in children.iter().enumerate() {
                let left_first_leaf = child.first_leaf();
                let child_el = render_layout(
                    child,
                    panes,
                    focused_pane_id,
                    has_splits,
                    dim_alpha,
                    font_family.clone(),
                    zoomed_pane_id,
                    cx,
                );
                let ratio = ratios[i];
                let cell = split_cell(is_horizontal, ratio, child_el);
                container = container.child(cell);
                if i + 1 < n {
                    let leaf_id = left_first_leaf;
                    container = container.child(pane_divider(is_horizontal, leaf_id, cx));
                }
            }
            container.into_any_element()
        }
    }
}
