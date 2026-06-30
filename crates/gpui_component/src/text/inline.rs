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
    text::text_view::SelectMode, text_selection::word_range,
};

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

        let line_height = window.line_height();
        let selection_bounds = text_view_state.selection_bounds();

        // Use for debug selection bounds
        // self.paint_selected_bounds(selection_bounds, window, cx);

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

            let mut char_width = line_height.half();
            if let Some(next_pos) = text_layout.position_for_index(offset + 1) {
                if next_pos.y == pos.y {
                    char_width = next_pos.x - pos.x;
                }
            }

            if point_in_text_selection(pos, char_width, &selection_bounds, line_height) {
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
            SelectMode::Word(_anchor) => {
                // Expand the first and last selected characters to their word
                // boundaries within this Inline element's text.
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
                let end = word_range(text.len(), char_at, raw_end.saturating_sub(1))
                    .map(|r| r.end)
                    .unwrap_or(raw_end);
                Some((start..end).into())
            }
            SelectMode::Line(_anchor) => {
                // Expand to the full visual (wrapped) line(s) that contain the
                // raw selection.  We walk all character positions and include
                // every character whose Y coordinate (visual row) overlaps the
                // Y range of the raw selection.
                let raw_top = text_layout
                    .position_for_index(raw_start)
                    .map(|p| p.y)
                    .unwrap_or_default();
                let raw_bottom = text_layout
                    .position_for_index(raw_end.saturating_sub(1))
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

/// Check if a `pos` is within a `bounds`, considering multi-line selections.
fn point_in_text_selection(
    pos: Point<Pixels>,
    char_width: Pixels,
    bounds: &Bounds<Pixels>,
    line_height: Pixels,
) -> bool {
    let top = bounds.top();
    let bottom = bounds.bottom();
    let left = bounds.left();
    let right = bounds.right();

    // Out of the vertical bounds
    if pos.y + line_height < top || pos.y >= bottom {
        return false;
    }

    let single_line = (bottom - top) <= line_height;
    if single_line {
        // If it's a single line selection, just check horizontal bounds
        return pos.x + char_width.half() >= left && pos.x + char_width.half() <= right;
    }

    let is_above = pos.y <= top;
    let is_below = pos.y + line_height >= bottom;

    if is_above {
        return pos.x + char_width.half() >= left;
    } else if is_below {
        return pos.x + char_width.half() <= right;
    } else {
        return true;
    }
}

#[cfg(test)]
mod tests {
    use super::point_in_text_selection;
    use gpui::{Bounds, point, px, size};

    #[test]
    fn test_point_in_text_selection() {
        let line_height = px(20.);
        let char_width = px(10.);
        let bounds = Bounds {
            origin: point(px(50.), px(50.)),
            size: size(px(100.), px(100.)),
        };

        // First line but haft line height, true
        // | p --------|
        // | selection |
        // |-----------|
        assert!(point_in_text_selection(
            point(px(50.), px(40.)),
            char_width,
            &bounds,
            line_height
        ));

        // First line in selection, true
        // | p --------|
        // | selection |
        // |-----------|
        assert!(point_in_text_selection(
            point(px(50.), px(50.)),
            char_width,
            &bounds,
            line_height
        ));
        // First line, but left out of selection, false
        // p |-----------|
        //   | selection |
        //   |-----------|
        assert!(!point_in_text_selection(
            point(px(40.), px(50.)),
            char_width,
            &bounds,
            line_height
        ));
        // First line but right out of selection, true
        // |-----------| p
        // | selection |
        // |-----------|
        assert!(point_in_text_selection(
            point(px(160.), px(50.)),
            char_width,
            &bounds,
            line_height
        ));

        // Middle line in selection, true
        // |-----------|
        // |     p     |
        // |-----------|
        assert!(point_in_text_selection(
            point(px(100.), px(70.)),
            char_width,
            &bounds,
            line_height
        ));
        // Middle line, but left out of selection, true
        //   |-----------|
        // p | selection |
        //   |-----------|
        assert!(point_in_text_selection(
            point(px(40.), px(70.)),
            char_width,
            &bounds,
            line_height
        ));
        // Middle line, but right out of selection, true
        // |-----------|
        // | selection | p
        // |-----------|
        assert!(point_in_text_selection(
            point(px(160.), px(70.)),
            char_width,
            &bounds,
            line_height
        ));

        // Last line in selection, true
        // |-----------|
        // | selection |
        // |------- p -|
        assert!(point_in_text_selection(
            point(px(100.), px(140.)),
            char_width,
            &bounds,
            line_height
        ));
        // Last line, but left out of selection, true
        //
        //   |-----------|
        //   | selection |
        // p |-----------|
        assert!(point_in_text_selection(
            point(px(40.), px(140.)),
            char_width,
            &bounds,
            line_height
        ));
        // Last line, but right out of selection, false
        // |-----------|
        // | selection |
        // |-----------| p
        assert!(!point_in_text_selection(
            point(px(160.), px(140.)),
            char_width,
            &bounds,
            line_height
        ));

        // Out of vertical bounds (top), false
        //       p
        // |-----------|
        // | selection |
        // |-----------|
        assert!(!point_in_text_selection(
            point(px(100.), px(20.)),
            char_width,
            &bounds,
            line_height
        ));
        // Out of vertical bounds (bottom), false
        // |-----------|
        // | selection |
        // |-----------|
        //       p
        assert!(!point_in_text_selection(
            point(px(100.), px(160.)),
            char_width,
            &bounds,
            line_height
        ));
    }

    /// Test the word-expansion logic used by SelectMode::Word inside
    /// layout_selections.  Replicates the char_at/word_range calls from
    /// the production path without requiring a live GPUI TextLayout.
    #[test]
    fn test_word_expansion_in_inline_text() {
        use crate::text_selection::word_range;

        let text = "hello world foo";

        let char_at = |i: usize| -> Option<char> {
            if i >= text.len() || !text.is_char_boundary(i) {
                return None;
            }
            text[i..].chars().next()
        };

        // raw_start=6, raw_end=11 (characters 'w' through 'd' — the pixel selection spans "world").
        // word_range on start (offset 6 = 'w') should give 6..11.
        // word_range on last char of "world" (offset 10 = 'd'), end should be 11.
        let start_expansion = word_range(text.len(), char_at, 6)
            .map(|r| r.start)
            .unwrap_or(6);
        let end_expansion = word_range(text.len(), char_at, 10)
            .map(|r| r.end)
            .unwrap_or(11);
        assert_eq!(&text[start_expansion..end_expansion], "world");

        // Double-click on middle of "hello" (offset 2): word should be 0..5.
        let start = word_range(text.len(), char_at, 2)
            .map(|r| r.start)
            .unwrap_or(2);
        let end = word_range(text.len(), char_at, 2)
            .map(|r| r.end)
            .unwrap_or(2);
        assert_eq!(&text[start..end], "hello");

        // Click on space (offset 5): word_range gives the space run 5..6.
        let r = word_range(text.len(), char_at, 5).unwrap();
        assert_eq!(&text[r], " ");
    }

    /// Test the visual-line-range boundary expansion logic used by
    /// SelectMode::Line inside layout_selections.
    ///
    /// We simulate the layout with a fake position-for-index that assigns
    /// fixed Y positions per character, matching a two-line layout:
    ///   Line 0 (y=0):   "hello " (offsets 0..6)
    ///   Line 1 (y=20):  "world"  (offsets 6..11)
    #[test]
    fn test_visual_line_expansion_logic() {
        use gpui::px;

        let text = "hello world";
        let line_height = px(20.0_f32);

        // Fake position_for_index: first 6 bytes on y=0, rest on y=20.
        let position_for_index = |offset: usize| -> Option<gpui::Point<gpui::Pixels>> {
            if offset > text.len() {
                return None;
            }
            let y = if offset < 6 { 0.0_f32 } else { 20.0_f32 };
            Some(gpui::point(gpui::px(offset as f32 * 8.0), gpui::px(y)))
        };

        // Simulate: user triple-clicked in line 1 (y=20).
        // pixel selection bounds cover only a small range in line 1.
        // raw_start = 6 ('w' at y=20), raw_end = 8 ('o' end).
        let raw_start = 6usize;
        let raw_end = 8usize;

        let raw_top = position_for_index(raw_start)
            .map(|p| p.y)
            .unwrap_or_default();
        let raw_bottom = position_for_index(raw_end.saturating_sub(1))
            .map(|p| p.y + line_height)
            .unwrap_or(raw_top + line_height);

        let mut line_start: Option<usize> = None;
        let mut line_end: Option<usize> = None;
        let mut scan_offset = 0;
        for c in text.chars() {
            if let Some(pos) = position_for_index(scan_offset) {
                if pos.y < raw_bottom && pos.y + line_height > raw_top {
                    if line_start.is_none() {
                        line_start = Some(scan_offset);
                    }
                    line_end = Some(scan_offset + c.len_utf8());
                }
            }
            scan_offset += c.len_utf8();
        }

        // Expected: full line 1 = "world" (offsets 6..11).
        assert_eq!(line_start, Some(6));
        assert_eq!(line_end, Some(11));
        assert_eq!(&text[line_start.unwrap()..line_end.unwrap()], "world");

        // Now simulate triple-click in line 0 (y=0).
        let raw_start_l0 = 1usize; // 'e'
        let raw_end_l0 = 3usize; // 'l'
        let raw_top_l0 = position_for_index(raw_start_l0)
            .map(|p| p.y)
            .unwrap_or_default();
        let raw_bottom_l0 = position_for_index(raw_end_l0.saturating_sub(1))
            .map(|p| p.y + line_height)
            .unwrap_or(raw_top_l0 + line_height);

        let mut ls0: Option<usize> = None;
        let mut le0: Option<usize> = None;
        let mut so = 0;
        for c in text.chars() {
            if let Some(pos) = position_for_index(so) {
                if pos.y < raw_bottom_l0 && pos.y + line_height > raw_top_l0 {
                    if ls0.is_none() {
                        ls0 = Some(so);
                    }
                    le0 = Some(so + c.len_utf8());
                }
            }
            so += c.len_utf8();
        }

        // Expected: full line 0 = "hello " (offsets 0..6).
        assert_eq!(ls0, Some(0));
        assert_eq!(le0, Some(6));
        assert_eq!(&text[ls0.unwrap()..le0.unwrap()], "hello ");
    }
}
