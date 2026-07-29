use std::{
    ops::Range,
    rc::Rc,
    sync::{Arc, Mutex},
};

use gpui::{
    App, BorderStyle, Bounds, CursorStyle, Edges, Element, ElementId, GlobalElementId, Half,
    HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyledText, TextLayout, Window,
    point, px, quad,
};

use crate::{
    ActiveTheme, global_state::GlobalState, input::Selection, text::node::LinkMark,
    text::text_view::SelectMode,
};
use daruda_core::text::{char_cell_hit_x, word_range};

/// A inline element used to render a inline text and support selectable.
///
/// All text in TextView (including the CodeBlock) used this for text rendering.
pub(super) struct Inline {
    id: ElementId,
    text: SharedString,
    links: Rc<Vec<(Range<usize>, LinkMark)>>,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    styled_text: StyledText,

    state: Arc<Mutex<InlineState>>,
}

/// The inline text state, used RefCell to keep the selection state.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct InlineState {
    hovered_index: Option<usize>,
    /// The text that actually rendering, matched with selection.
    pub(super) text: SharedString,
    pub(super) selection: Option<Selection>,
}

impl InlineState {
    /// Save actually rendered text for selected text to use.
    pub(crate) fn set_text(&mut self, text: SharedString) {
        self.text = text;
    }
}

impl Inline {
    pub(super) fn new(
        id: impl Into<ElementId>,
        state: Arc<Mutex<InlineState>>,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
    ) -> Self {
        let text = state.lock().unwrap().text.clone();
        Self {
            id: id.into(),
            links: Rc::new(links),
            highlights,
            text: text.clone(),
            styled_text: StyledText::new(text),
            state,
        }
    }

    /// Get link at given mouse position.
    fn link_for_position(
        layout: &TextLayout,
        links: &Vec<(Range<usize>, LinkMark)>,
        position: Point<Pixels>,
    ) -> Option<LinkMark> {
        let offset = layout.index_for_position(position).ok()?;
        for (range, link) in links.iter() {
            if range.contains(&offset) {
                return Some(link.clone());
            }
        }

        None
    }

    /// Paint selected bounds for debug.
    #[allow(unused)]
    fn paint_selected_bounds(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        window.paint_quad(gpui::PaintQuad {
            bounds,
            background: cx.theme().blue.alpha(0.01).into(),
            corner_radii: gpui::Corners::default(),
            border_color: gpui::transparent_black(),
            border_style: BorderStyle::default(),
            border_widths: gpui::Edges::all(px(0.)),
        });
    }

    fn layout_selections(
        &self,
        text_layout: &TextLayout,
        window: &mut Window,
        cx: &mut App,
    ) -> (bool, bool, Option<Selection>) {
        let Some(text_view_state) = GlobalState::global(cx).text_view_state() else {
            return (false, false, None);
        };

        let text_view_state = text_view_state.read(cx);
        let is_selectable = text_view_state.is_selectable();
        if !text_view_state.has_selection() {
            return (is_selectable, false, None);
        }

        let mode = text_view_state.select_mode();

        // All mode: select this entire Inline element's text.
        if matches!(mode, SelectMode::All) {
            if self.text.is_empty() {
                return (is_selectable, true, None);
            }
            return (is_selectable, true, Some((0..self.text.len()).into()));
        }

        // The layout's own row pitch, not `window.line_height()`: bands must tile
        // exactly against the rows `position_for_index` reports, and the two are
        // separate roundings of the same value. `paint_selection` reads it too,
        // so hit-testing and the painted highlight agree on one number.
        let line_height = text_layout.line_height();
        let Some((anchor, cursor)) = text_view_state.selection_span() else {
            return (is_selectable, false, None);
        };

        // Use for debug selection bounds
        // self.paint_selected_bounds(text_view_state.selection_bounds(), window, cx);

        // Pass 1: collect the raw byte range covered by the pixel selection bounds.
        let mut raw_start: Option<usize> = None;
        let mut raw_end: Option<usize> = None;
        let mut offset = 0;
        let mut chars = self.text.chars().peekable();
        while let Some(c) = chars.next() {
            let Some(pos) = text_layout.position_for_index(offset) else {
                offset += c.len_utf8();
                continue;
            };

            // Advance by the current char's byte length, not a single byte, so
            // the next index is a valid UTF-8 boundary. `offset + 1` lands
            // mid-char for multi-byte glyphs (e.g. 3-byte Hangul/CJK), which
            // makes `position_for_index` miss and the cell width collapse to the
            // fallback — so clicks on the char's right half miss it and word
            // selection never fires for non-ASCII text.
            let mut char_width = line_height.half();
            if let Some(next_pos) = text_layout.position_for_index(offset + c.len_utf8()) {
                if next_pos.y == pos.y {
                    char_width = next_pos.x - pos.x;
                }
            }

            if char_in_text_selection(pos, char_width, anchor, cursor, line_height) {
                if raw_start.is_none() {
                    raw_start = Some(offset);
                }
                raw_end = Some(offset + c.len_utf8());
            }

            offset += c.len_utf8();
        }

        let Some(raw_start) = raw_start else {
            return (is_selectable, true, None);
        };
        let raw_end = raw_end.unwrap_or(raw_start);

        let selection = match mode {
            SelectMode::Character => {
                // Existing character-granularity behaviour.
                Some((raw_start..raw_end).into())
            }
            SelectMode::Word => {
                // Expand the first and last selected characters to their word
                // boundaries within this Inline element's text.
                //
                // Design note: expansion is limited to this single Inline
                // element.  A word that spans an Inline boundary (e.g. the
                // last word of one run and the first of the next share no
                // common Inline) will be clipped at the element edge.  This
                // is an accepted limitation of the current single-Inline
                // architecture.
                let text = self.text.as_ref();
                let char_at = |i: usize| -> Option<char> {
                    if i >= text.len() || !text.is_char_boundary(i) {
                        return None;
                    }
                    text[i..].chars().next()
                };
                let start = word_range(text.len(), char_at, raw_start)
                    .map(|r| r.start)
                    .unwrap_or(raw_start);
                // Expand from the last selected char's *start*. `raw_end` is
                // exclusive, so `raw_end - 1` would land inside a multi-byte
                // glyph (Hangul/CJK) and `word_range` would bail (char_at at a
                // non-boundary is None) — leaving the word end unexpanded.
                let last_char_start = text.floor_char_boundary(raw_end.saturating_sub(1));
                let end = word_range(text.len(), char_at, last_char_start)
                    .map(|r| r.end)
                    .unwrap_or(raw_end);
                Some((start..end).into())
            }
            SelectMode::Line => {
                // Expand to the full visual (wrapped) line(s) that contain the
                // raw selection.  We walk all character positions and include
                // every character whose Y coordinate (visual row) overlaps the
                // Y range of the raw selection.
                let raw_top = text_layout
                    .position_for_index(raw_start)
                    .map(|p| p.y)
                    .unwrap_or_default();
                // Floor to a char boundary: `raw_end - 1` is mid-glyph for
                // multi-byte text, which `position_for_index` can't resolve.
                let raw_last = self.text.floor_char_boundary(raw_end.saturating_sub(1));
                let raw_bottom = text_layout
                    .position_for_index(raw_last)
                    .map(|p| p.y + line_height)
                    .unwrap_or(raw_top + line_height);

                let mut line_start: Option<usize> = None;
                let mut line_end: Option<usize> = None;
                let mut scan_offset = 0;
                for c in self.text.chars() {
                    if let Some(pos) = text_layout.position_for_index(scan_offset) {
                        // Character is on a visual row that overlaps [raw_top, raw_bottom).
                        if pos.y < raw_bottom && pos.y + line_height > raw_top {
                            if line_start.is_none() {
                                line_start = Some(scan_offset);
                            }
                            line_end = Some(scan_offset + c.len_utf8());
                        }
                    }
                    scan_offset += c.len_utf8();
                }

                match (line_start, line_end) {
                    (Some(s), Some(e)) => Some((s..e).into()),
                    _ => None,
                }
            }
            // All is handled above; Character is already matched.
            SelectMode::All => unreachable!(),
        };

        (is_selectable, true, selection)
    }

    /// Paint the selection background.
    fn paint_selection(
        selection: &Selection,
        text_layout: &TextLayout,
        bounds: &Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut start = selection.start;
        let mut end = selection.end;
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        let Some(start_position) = text_layout.position_for_index(start) else {
            return;
        };
        let Some(end_position) = text_layout.position_for_index(end) else {
            return;
        };

        let line_height = text_layout.line_height();
        if start_position.y == end_position.y {
            window.paint_quad(quad(
                Bounds::from_corners(
                    start_position,
                    point(end_position.x, end_position.y + line_height),
                ),
                px(0.),
                cx.theme().selection,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));
        } else {
            window.paint_quad(quad(
                Bounds::from_corners(
                    start_position,
                    point(bounds.right(), start_position.y + line_height),
                ),
                px(0.),
                cx.theme().selection,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));

            if end_position.y > start_position.y + line_height {
                window.paint_quad(quad(
                    Bounds::from_corners(
                        point(bounds.left(), start_position.y + line_height),
                        point(bounds.right(), end_position.y),
                    ),
                    px(0.),
                    cx.theme().selection,
                    Edges::default(),
                    gpui::transparent_black(),
                    BorderStyle::default(),
                ));
            }

            window.paint_quad(quad(
                Bounds::from_corners(
                    point(bounds.left(), end_position.y),
                    point(end_position.x, end_position.y + line_height),
                ),
                px(0.),
                cx.theme().selection,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));
        }
    }
}

impl IntoElement for Inline {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Inline {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_element_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();

        let mut runs = Vec::new();
        let mut ix = 0;
        for (range, highlight) in self.highlights.iter() {
            if ix < range.start {
                runs.push(text_style.clone().to_run(range.start - ix));
            }
            runs.push(text_style.clone().highlight(*highlight).to_run(range.len()));
            ix = range.end;
        }
        if ix < self.text.len() {
            runs.push(text_style.to_run(self.text.len() - ix));
        }

        self.styled_text = StyledText::new(self.text.clone()).with_runs(runs);
        let (layout_id, _) =
            self.styled_text
                .request_layout(global_element_id, inspector_id, window, cx);

        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled_text
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();
        let hitbox = prepaint;
        let mut state = self.state.lock().unwrap();

        let text_layout = self.styled_text.layout().clone();
        self.styled_text
            .paint(global_id, None, bounds, &mut (), &mut (), window, cx);

        // layout selections
        let (is_selectable, is_selection, selection) =
            self.layout_selections(&text_layout, window, cx);

        state.selection = selection;

        if is_selection || is_selectable {
            window.set_cursor_style(CursorStyle::IBeam, &hitbox);
        }

        // link cursor pointer
        let mouse_position = window.mouse_position();
        if let Some(_) = Self::link_for_position(&text_layout, &self.links, mouse_position) {
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
        }

        if let Some(selection) = &state.selection {
            Self::paint_selection(selection, &text_layout, &bounds, window, cx);
        }

        // mouse move, update hovered link
        window.on_mouse_event({
            let hitbox = hitbox.clone();
            let text_layout = text_layout.clone();
            let mut hovered_index = state.hovered_index;
            move |event: &MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() || !hitbox.is_hovered(window) {
                    return;
                }

                let current = hovered_index;
                let updated = text_layout.index_for_position(event.position).ok();
                //  notify update when hovering over different links
                if current != updated {
                    hovered_index = updated;
                    cx.notify(current_view);
                }
            }
        });

        if !is_selection {
            // click to open link
            window.on_mouse_event({
                let links = self.links.clone();
                let text_layout = text_layout.clone();

                move |event: &MouseUpEvent, phase, _, cx| {
                    if !bounds.contains(&event.position) || !phase.bubble() {
                        return;
                    }

                    if let Some(link) =
                        Self::link_for_position(&text_layout, &links, event.position)
                    {
                        cx.stop_propagation();
                        cx.open_url(&link.url);
                    }
                }
            });
        }
    }
}

/// Where a drag endpoint cuts the line band `[line_top, line_top + line_height)`,
/// expressed as an x on that line: an endpoint on an earlier line cuts before
/// every character, one on a later line cuts after every character.
///
/// This is what turns a selection back into a document-ordered range. Reducing
/// both endpoints to the same line before comparing means the drag's shape —
/// whether the lower endpoint happens to sit left or right of the upper one —
/// can no longer decide which side of a line it bounds.
fn endpoint_cut_x(endpoint: Point<Pixels>, line_top: Pixels, line_height: Pixels) -> f32 {
    if endpoint.y < line_top {
        f32::NEG_INFINITY
    } else if endpoint.y >= line_top + line_height {
        f32::INFINITY
    } else {
        f32::from(endpoint.x)
    }
}

/// Whether the character cell at `pos` (width `char_width`) falls inside the
/// drag from `anchor` to `cursor`.
///
/// A line neither endpoint reaches collapses to two equal infinities, which
/// [`char_cell_hit_x`]'s degenerate-span branch rejects — so lines outside the
/// drag need no separate vertical test.
fn char_in_text_selection(
    pos: Point<Pixels>,
    char_width: Pixels,
    anchor: Point<Pixels>,
    cursor: Point<Pixels>,
    line_height: Pixels,
) -> bool {
    let a = endpoint_cut_x(anchor, pos.y, line_height);
    let b = endpoint_cut_x(cursor, pos.y, line_height);
    char_cell_hit_x(f32::from(pos.x), f32::from(char_width), a.min(b), a.max(b))
}

#[cfg(test)]
mod tests {
    use super::{char_in_text_selection, endpoint_cut_x};
    use gpui::{Pixels, Point, point, px};

    const LINE_HEIGHT: Pixels = px(20.);
    const CHAR_WIDTH: Pixels = px(10.);

    /// Line `n`'s top, for a grid anchored at y = 0.
    fn line_top(n: f32) -> Pixels {
        px(n * 20.)
    }

    /// Is the character cell starting at (`x`, top of line `line`) selected by
    /// the drag `anchor` → `cursor`?
    fn selected(line: f32, x: f32, anchor: Point<Pixels>, cursor: Point<Pixels>) -> bool {
        char_in_text_selection(
            point(px(x), line_top(line)),
            CHAR_WIDTH,
            anchor,
            cursor,
            LINE_HEIGHT,
        )
    }

    #[test]
    fn endpoint_cut_collapses_to_the_side_it_falls_on() {
        let top = line_top(1.);
        assert_eq!(
            endpoint_cut_x(point(px(70.), px(15.)), top, LINE_HEIGHT),
            f32::NEG_INFINITY,
            "an endpoint above the line cuts before every character on it"
        );
        assert_eq!(
            endpoint_cut_x(point(px(70.), px(45.)), top, LINE_HEIGHT),
            f32::INFINITY,
            "an endpoint below the line cuts after every character on it"
        );
        assert_eq!(
            endpoint_cut_x(point(px(70.), px(25.)), top, LINE_HEIGHT),
            70.,
            "an endpoint on the line cuts at its own x"
        );
        assert_eq!(
            endpoint_cut_x(point(px(70.), top), top, LINE_HEIGHT),
            70.,
            "the band is inclusive at its top edge"
        );
        assert_eq!(
            endpoint_cut_x(point(px(70.), top + LINE_HEIGHT), top, LINE_HEIGHT),
            f32::INFINITY,
            "and exclusive at its bottom edge — that y belongs to the next line"
        );
    }

    /// Drag down from line 0 to line 1, ending LEFT of where it started.
    ///
    /// The endpoints' x order says nothing about direction here: line 0 must
    /// still run from the anchor rightwards, and line 1 from its start to the
    /// cursor. Treating the drag as a rectangle instead selected the union —
    /// text before the anchor on line 0, and text past the cursor on line 1.
    #[test]
    fn a_downward_drag_ending_left_of_its_anchor_does_not_grow() {
        let anchor = point(px(100.), px(5.));
        let cursor = point(px(50.), px(35.));

        assert!(
            !selected(0., 60., anchor, cursor),
            "line 0 before the anchor stays unselected"
        );
        assert!(
            selected(0., 120., anchor, cursor),
            "line 0 after the anchor runs to the end of the line"
        );
        assert!(
            selected(1., 30., anchor, cursor),
            "line 1 is selected up to the cursor"
        );
        assert!(
            !selected(1., 80., anchor, cursor),
            "line 1 past the cursor stays unselected"
        );
    }

    /// The same drag, mirrored: moving the cursor further left may only shrink
    /// the lower line, never extend the upper one.
    #[test]
    fn moving_the_cursor_left_only_shrinks_the_lower_line() {
        let anchor = point(px(100.), px(5.));
        let near = point(px(90.), px(35.));
        let far = point(px(20.), px(35.));

        assert!(selected(1., 60., anchor, near));
        assert!(
            !selected(1., 60., anchor, far),
            "dragging further left releases line 1 text"
        );
        assert!(selected(0., 120., anchor, near));
        assert!(
            selected(0., 120., anchor, far),
            "line 0 after the anchor is unaffected by the cursor's x"
        );
    }

    /// Each line is bounded by its own endpoint, not by the drag's shared x
    /// span. Pass 1 keeps only the first and last hit, so for this rightward
    /// drag the contiguous envelope hides the difference — but it is the same
    /// predicate that makes a leftward drag come out right, so pin it directly.
    #[test]
    fn each_line_is_bounded_by_its_own_endpoint() {
        let anchor = point(px(30.), px(5.));
        let cursor = point(px(80.), px(22.));

        assert!(
            selected(0., 150., anchor, cursor),
            "line 0 continues past the cursor's x to the end of the line"
        );
        assert!(selected(1., 50., anchor, cursor), "line 1 up to the cursor");
        assert!(!selected(1., 150., anchor, cursor), "but not past it");
    }

    #[test]
    fn a_drag_inside_one_line_never_reaches_its_neighbours() {
        let anchor = point(px(30.), px(5.));
        let cursor = point(px(80.), px(18.));

        assert!(selected(0., 50., anchor, cursor));
        assert!(
            !selected(1., 50., anchor, cursor),
            "the line below is outside the drag"
        );
        assert!(!selected(-1., 50., anchor, cursor), "so is the line above");
    }

    #[test]
    fn a_single_line_drag_selects_between_its_endpoints_in_either_direction() {
        let left = point(px(30.), px(5.));
        let right = point(px(80.), px(15.));

        for (anchor, cursor) in [(left, right), (right, left)] {
            assert!(!selected(0., 10., anchor, cursor), "before the span");
            assert!(selected(0., 50., anchor, cursor), "inside the span");
            assert!(!selected(0., 100., anchor, cursor), "after the span");
        }
    }

    #[test]
    fn lines_between_the_endpoints_are_fully_selected() {
        let anchor = point(px(100.), px(5.));
        let cursor = point(px(50.), px(55.));

        assert!(selected(1., 0., anchor, cursor));
        assert!(selected(1., 500., anchor, cursor));
    }

    /// Direction independence is the whole point of the fix: nothing orders the
    /// endpoints, so dragging up must give the same selection as dragging down
    /// between the same two points.
    #[test]
    fn an_upward_drag_matches_the_downward_one() {
        let upper = point(px(100.), px(5.));
        let lower = point(px(50.), px(55.));

        for (line, x) in [
            (0., 60.),
            (0., 120.),
            (1., 0.),
            (1., 500.),
            (2., 30.),
            (2., 80.),
        ] {
            assert_eq!(
                selected(line, x, upper, lower),
                selected(line, x, lower, upper),
                "line {line} x {x} differs by drag direction"
            );
        }
    }

    #[test]
    fn a_click_with_no_drag_hits_the_character_under_it() {
        // Word/line clicks produce a zero-width span and re-expand from this
        // raw scan, so the cell containing the point must still register.
        let at = point(px(35.), px(5.));
        assert!(selected(0., 30., at, at), "cell 30..40 contains x = 35");
        assert!(!selected(0., 40., at, at), "the next cell does not");
        // A click must not reach other lines — Word/Line expansion re-reads
        // this scan, so a stray hit there would select a whole foreign line.
        assert!(!selected(1., 30., at, at), "nor the line below");
        assert!(!selected(-1., 30., at, at), "nor the line above");
    }
}
