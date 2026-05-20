//! Custom GPUI Element that renders one file-viewer content cell with
//! character-level text selection.
//!
//! Shapes text in `prepaint` so the pixel↔byte mapping is exact, then
//! registers mouse-event handlers in `paint` via `window.on_mouse_event`
//! so dragging across the row produces a `CharSelection` keyed to byte
//! offsets within the row's content.

use std::rc::Rc;

use crate::ui::theme;
use gpui::{
    App, Bounds, DispatchPhase, HitboxBehavior, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, ShapedLine, SharedString, Style, TextRun, Window, fill, px,
};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::{
    CharPos, CharSelection, HighlightedSpan, VisualRow, VisualRowKind, WordChange,
};

/// Custom Element that renders one file-viewer content cell with character-level
/// text selection. Shapes text in `prepaint` so pixel↔byte mapping is exact,
/// then registers mouse event handlers in `paint` via `window.on_mouse_event`.
pub(super) struct FileViewerContentElement {
    workspace: gpui::Entity<Workspace>,
    row_idx: usize,
    content: String,
    spans: Vec<HighlightedSpan>,
    word_changes: Vec<WordChange>,
    row_kind: VisualRowKind,
    char_selection: Option<CharSelection>,
    default_text_color: gpui::Hsla,
    line_h: Pixels,
}

impl FileViewerContentElement {
    pub(super) fn new(
        workspace: gpui::Entity<Workspace>,
        row_idx: usize,
        row: &VisualRow,
        char_selection: Option<&CharSelection>,
        default_text_color: gpui::Hsla,
        line_h: Pixels,
    ) -> Self {
        Self {
            workspace,
            row_idx,
            content: row.content.clone(),
            spans: row.spans.clone(),
            word_changes: row.word_changes.clone(),
            row_kind: row.kind,
            char_selection: char_selection.cloned(),
            default_text_color,
            line_h,
        }
    }
}

pub(super) struct FileViewerContentPrepaint {
    shaped_rc: Rc<Option<ShapedLine>>,
    selection_quad: Option<gpui::PaintQuad>,
    word_quads: Vec<gpui::PaintQuad>,
    hitbox: gpui::Hitbox,
}

impl IntoElement for FileViewerContentElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for FileViewerContentElement {
    type RequestLayoutState = ();
    type PrepaintState = FileViewerContentPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            flex_grow: 1.0,
            min_size: gpui::Size {
                width: px(0.).into(),
                ..Default::default()
            },
            size: gpui::Size {
                height: self.line_h.into(),
                ..Default::default()
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> FileViewerContentPrepaint {
        let t = theme::current(cx);
        let selection_bg = t.file_viewer_selection_bg;
        let word_del_bg = t.file_diff_word_del_bg;
        let word_add_bg = t.file_diff_word_add_bg;
        let font_size = px(theme::FILE_VIEWER_FONT_SIZE);
        let font = window.text_style().font();

        // Build TextRun array. Prefer syntax-highlighted spans when present and
        // contiguous; fall back to a single default-color run otherwise.
        let spans_len: usize = self.spans.iter().map(|s| s.text.len()).sum();
        let use_spans = !self.spans.is_empty() && spans_len == self.content.len();

        let runs: Vec<TextRun> = if use_spans {
            self.spans
                .iter()
                .filter(|s| !s.text.is_empty())
                .map(|span| TextRun {
                    len: span.text.len(),
                    font: font.clone(),
                    color: span.color.unwrap_or(self.default_text_color),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect()
        } else if self.content.is_empty() {
            vec![]
        } else {
            vec![TextRun {
                len: self.content.len(),
                font,
                color: self.default_text_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }]
        };

        let shaped_opt: Option<ShapedLine> = if self.content.is_empty() || runs.is_empty() {
            None
        } else {
            let text: SharedString = self.content.clone().into();
            Some(
                window
                    .text_system()
                    .shape_line(text, font_size, &runs, None),
            )
        };

        // Compute character-level selection quad.
        let selection_quad = shaped_opt.as_ref().and_then(|sh| {
            let sel = self.char_selection.as_ref()?;
            let range = sel.byte_range_for_row(self.row_idx, self.content.len())?;
            let x1 = sh.x_for_index(range.start);
            let x2 = if range.end >= self.content.len() {
                bounds.size.width
            } else {
                sh.x_for_index(range.end)
            };
            if x2 <= x1 {
                return None;
            }
            Some(fill(
                Bounds {
                    origin: gpui::Point {
                        x: bounds.origin.x + x1,
                        y: bounds.origin.y,
                    },
                    size: gpui::Size {
                        width: x2 - x1,
                        height: self.line_h,
                    },
                },
                selection_bg,
            ))
        });

        // Compute word-diff background quads (only when not overridden by selection).
        let word_quads: Vec<gpui::PaintQuad> =
            if selection_quad.is_none() && !self.word_changes.is_empty() {
                let word_bg = match self.row_kind {
                    VisualRowKind::Removed => word_del_bg,
                    _ => word_add_bg,
                };
                shaped_opt
                    .as_ref()
                    .map(|sh| {
                        self.word_changes
                            .iter()
                            .filter_map(|wc| {
                                let start = wc.start.min(self.content.len());
                                let end = wc.end.min(self.content.len());
                                if start >= end
                                    || !self.content.is_char_boundary(start)
                                    || !self.content.is_char_boundary(end)
                                {
                                    return None;
                                }
                                let x1 = sh.x_for_index(start);
                                let x2 = sh.x_for_index(end);
                                if x2 <= x1 {
                                    return None;
                                }
                                Some(fill(
                                    Bounds {
                                        origin: gpui::Point {
                                            x: bounds.origin.x + x1,
                                            y: bounds.origin.y,
                                        },
                                        size: gpui::Size {
                                            width: x2 - x1,
                                            height: self.line_h,
                                        },
                                    },
                                    word_bg,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let shaped_rc = Rc::new(shaped_opt);

        FileViewerContentPrepaint {
            shaped_rc,
            selection_quad,
            word_quads,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut FileViewerContentPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Draw selection or word-diff background quads (under text).
        if let Some(sel_quad) = prepaint.selection_quad.take() {
            window.paint_quad(sel_quad);
        } else {
            for quad in prepaint.word_quads.drain(..) {
                window.paint_quad(quad);
            }
        }

        // Draw text.
        if let Some(shaped) = prepaint.shaped_rc.as_ref() {
            let _ = shaped.paint(
                bounds.origin,
                self.line_h,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        // Register mouse event handlers. Closures capture Rc clones so the
        // shaped line stays alive as long as any handler holds a reference.
        let shaped_down = Rc::clone(&prepaint.shaped_rc);
        let shaped_move = Rc::clone(&prepaint.shaped_rc);
        let hitbox_down = prepaint.hitbox.clone();
        let hitbox_move = prepaint.hitbox.clone();
        let origin_x = bounds.origin.x;
        let workspace_down = self.workspace.clone();
        let workspace_move = self.workspace.clone();
        let row_idx = self.row_idx;

        window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || ev.button != MouseButton::Left
                || !hitbox_down.is_hovered(window)
            {
                return;
            }
            let local_x = (ev.position.x - origin_x).max(px(0.));
            let byte = if let Some(s) = shaped_down.as_ref() {
                s.closest_index_for_x(local_x)
            } else {
                0
            };
            let hit = CharPos { row: row_idx, byte };
            let shift = ev.modifiers.shift;
            workspace_down.update(cx, |ws, cx| {
                ws.file_view_mouse_down(hit, shift, cx);
            });
        });

        window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let local_x = (ev.position.x - origin_x).max(px(0.));
            let byte = if let Some(s) = shaped_move.as_ref() {
                s.closest_index_for_x(local_x)
            } else {
                0
            };
            let active = CharPos { row: row_idx, byte };
            let still_pressed = ev.pressed_button == Some(MouseButton::Left);
            let hovered = hitbox_move.is_hovered(window);
            workspace_move.update(cx, |ws, cx| {
                ws.file_view_mouse_drag(active, still_pressed, hovered, cx);
            });
        });
    }
}
