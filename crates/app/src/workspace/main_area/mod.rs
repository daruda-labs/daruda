//! MainArea: TabBar + PaneTree walker + submodule declarations.
//!
//! `render_layout` recurses over the active tab's `PaneLayout`,
//! emitting one `div` per leaf (with optional pane header in split
//! mode and the file-viewer overlay when one is open) and inserting
//! draggable dividers between siblings of a `Split`. Lives next to
//! `pane_header` (only called from here) so the two stay in sync.

pub(in crate::workspace) mod agent_chat_pane;
pub(in crate::workspace) mod bottom_dock;
pub(in crate::workspace) mod context;
pub(in crate::workspace) mod file_pane_ops;
pub(in crate::workspace) mod file_view_pane;
pub(in crate::workspace) mod flow_graph_pane;
pub(in crate::workspace) mod nav;
pub(in crate::workspace) mod pane;
pub(in crate::workspace) mod pane_drag_ops;
pub(in crate::workspace) mod pane_input_ops;
pub(in crate::workspace) mod pane_menu;
pub(in crate::workspace) mod pane_tree;
pub(in crate::workspace) mod prompt_watcher;
pub(in crate::workspace) mod resize;
pub(in crate::workspace) mod tab_drag_ops;
pub(in crate::workspace) mod tab_ops;
pub(in crate::workspace) mod task_edit_pane;

pub(in crate::workspace) use context::MainAreaContext;

use crate::ui::cursor::CursorReachExt as _;
use crate::ui::theme;
use gpui::{
    AnyElement, AnyView, ClickEvent, Context, CursorStyle, ExternalPaths, IntoElement, MouseButton,
    MouseDownEvent, SharedString, StyleRefinement, div, prelude::*, px,
};

use crate::shell_quote::{Shell, format_paths_for_drop, quote_path};
use crate::workspace::path_drag::PathDrag;

use self::file_view_pane::render::render_pane_file_viewer;
use self::pane::Pane;
use self::pane_drag_ops::{PaneHeaderDrag, PaneHeaderDragGhost};
use self::pane_tree::{DIVIDER_PX, DropHalf, PaneId, PaneLayout, SplitDirection};
use self::tab_drag_ops::TabDrag;
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

/// Half-fill drop-target hint drawn over the pane the cursor hovers while a
/// Pane header is dragged. The filled half indicates where the dragged pane
/// will land (West/East = side-by-side, North/South = stacked).
fn drop_target_overlay(half: DropHalf, cx: &mut Context<Workspace>) -> impl IntoElement {
    let bg = theme::current(cx).terminal_drop_target_bg;
    let base = div().absolute().bg(bg);
    match half {
        DropHalf::North => base.top_0().left_0().right_0().h(gpui::relative(0.5)),
        DropHalf::South => base.bottom_0().left_0().right_0().h(gpui::relative(0.5)),
        DropHalf::West => base.top_0().bottom_0().left_0().w(gpui::relative(0.5)),
        DropHalf::East => base.top_0().bottom_0().right_0().w(gpui::relative(0.5)),
    }
}

/// 1px visible divider + absolute hit-zone overlay between split siblings.
fn pane_divider(
    is_horizontal: bool,
    leaf_id: PaneId,
    dragging: bool,
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
            // Hitbox-bound while idle, window-wide once held: the drag is
            // what takes the pointer off the divider. See `ui::cursor`.
            .cursor_reach(Some(crate::ui::cursor::CursorReach::while_dragging(
                overlay_cursor,
                dragging,
            )))
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
    let divider_bg = theme::current(cx).border;
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
    _is_zoomed: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let close = crate::ui::button_close(("pane-close", pane_id as usize), cx).on_click(
        cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.request_close_pane(pane_id, window, cx);
        }),
    );

    let t = theme::current(cx);
    let focused_bg = t.pane_header_focused_bg;
    let focused_text = t.text_primary;
    let unfocused_bg = t.pane_header_unfocused_bg;
    let unfocused_text = t.text_muted;
    let cwd_text = t.text_muted;

    div()
        .id(("pane-hdr", pane_id as usize))
        .flex()
        .flex_row()
        .items_center()
        .h(px(theme::PANE_HEADER_HEIGHT))
        .w_full()
        .px(px(theme::PANE_HEADER_PAD_X))
        .gap(px(theme::PANE_HEADER_GAP))
        .text_size(px(theme::PANE_HEADER_FONT_SIZE))
        .on_drag(
            PaneHeaderDrag {
                dragged: pane_id,
                title: title.clone(),
            },
            |d, offset, _window, cx| {
                cx.new(|_| PaneHeaderDragGhost {
                    title: d.title.clone(),
                    offset,
                })
            },
        )
        .when(is_focused, |d| d.bg(focused_bg).text_color(focused_text))
        .when(!is_focused, |d| {
            d.bg(unfocused_bg).text_color(unfocused_text)
        })
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
    font_family: SharedString,
    zoomed_pane_id: Option<PaneId>,
    drop_target: Option<(PaneId, DropHalf)>,
    // The divider being held, if one is — its cursor has to reach past the
    // few pixels it occupies, since a drag is what pulls the pointer off them.
    dragged_divider: Option<PaneId>,
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
                .relative()
                .size_full()
                .flex()
                .flex_col()
                // The context menu, for every pane kind that does not already
                // answer a right-click itself. Terminal does — it has to decide
                // between the host menu and the program holding mouse capture
                // (`TerminalViewEvent::ContextMenuRequested`), and a second
                // handler here would open a menu it had ruled out.
                .when(!pane.is_terminal(), |d| {
                    d.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
                            this.open_pane_context_menu_at(id, ev.position, window, cx)
                        }),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.focus_pane_on_click(id, window, cx)
                    }),
                )
                .on_drop::<PaneHeaderDrag>(cx.listener(
                    move |this, d: &PaneHeaderDrag, window, cx| {
                        this.drop_pane_onto(d.dragged, window, cx)
                    },
                ))
                .on_drop::<TabDrag>(cx.listener(move |this, d: &TabDrag, window, cx| {
                    this.drop_tab_onto_pane(d.tab_id, window, cx)
                }));

            if has_splits {
                let basename = pane.display_cwd();
                let is_zoomed = zoomed_pane_id == Some(id);
                root = root.child(pane_header(
                    id,
                    is_focused,
                    pane.title(cx),
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
                        id,
                        &f.view,
                        f.editor_state.clone(),
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
                self::pane::PaneContent::AgentChat(ac) => div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    // Cached element (like the Terminal arm): a sibling-only
                    // dirty reuses the prior paint; the view's own `cx.notify()`
                    // forces a re-render. It tracks its own focus handle, so no
                    // `track_focus` here. Inactive-split dim is applied inside
                    // the view (`AgentChatView::dim`, alpha-preserving) rather
                    // than an overlay scrim, so window translucency survives.
                    .child(
                        AnyView::from(ac.view.clone())
                            .cached(StyleRefinement::default().size_full().flex()),
                    ),
                self::pane::PaneContent::FlowGraph(fg) => div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    // Cached for the same reason as the AgentChat arm: a run
                    // reports node by node, and this keeps those repaints in
                    // the view's own subtree. It tracks its own focus handle.
                    .child(
                        AnyView::from(fg.view.clone())
                            .cached(StyleRefinement::default().size_full().flex()),
                    ),
                self::pane::PaneContent::Terminal(t) => {
                    let view_for_path_drag = t.view.clone();
                    let view_for_external = t.view.clone();
                    // Inactive-pane dim goes through `TerminalView::set_dim_amount`
                    // on the terminal's own colors, not a black overlay, so a
                    // transparent background survives.
                    div()
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
                        // Cache the terminal view as an element: a sibling-only
                        // dirty reuses the prior prepaint+paint instead of
                        // re-shaping the grid; a real update's `cx.notify()` on
                        // the view forces a re-render. Style mirrors the view's
                        // own root (`size_full().flex()`).
                        .child(
                            AnyView::from(t.view.clone())
                                .cached(StyleRefinement::default().size_full().flex()),
                        )
                }
            };
            root = root.child(content);
            // Drop-target half-fill hint, drawn as a sibling of the `.cached()`
            // view so caching is preserved. Renders from the `pane_drop_hover`
            // snapshot only — no state transition here.
            if let Some((target_id, half)) = drop_target
                && target_id == id
            {
                root = root.child(drop_target_overlay(half, cx));
            }
            root.into_any_element()
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
                    font_family.clone(),
                    zoomed_pane_id,
                    drop_target,
                    dragged_divider,
                    cx,
                );
                let ratio = ratios[i];
                let cell = split_cell(is_horizontal, ratio, child_el);
                container = container.child(cell);
                if i + 1 < n {
                    let leaf_id = left_first_leaf;
                    container = container.child(pane_divider(
                        is_horizontal,
                        leaf_id,
                        dragged_divider == Some(leaf_id),
                        cx,
                    ));
                }
            }
            container.into_any_element()
        }
    }
}
