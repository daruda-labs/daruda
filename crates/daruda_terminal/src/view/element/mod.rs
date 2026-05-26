mod paint;
mod prepaint;
mod prepaint_helpers;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, IntoElement, LayoutId, PaintQuad, Pixels,
    Style, Window, relative,
};

use super::TerminalView;

pub(super) struct TerminalPrepaintState {
    line_height: Pixels,
    shaped_lines: Vec<gpui::ShapedLine>,
    background_quads: Vec<PaintQuad>,
    selection_quads: Vec<PaintQuad>,
    box_drawing_quads: Vec<PaintQuad>,
    marked_text: Option<(gpui::ShapedLine, gpui::Point<Pixels>)>,
    marked_text_background: Option<PaintQuad>,
    /// When IME preedit is active, visually shifts the text to the
    /// right of the caret rightward by the preedit's grid width so
    /// the user gets a "push" effect similar to typing ASCII. The
    /// shell hasn't received anything yet, so this is purely a
    /// render-side translation; on cancel the composition dissolves
    /// and `None` here restores the original layout, on commit the
    /// shell's own insertion catches up.
    preedit_post_shift: Option<PreeditPostShift>,
    cursor: Option<PaintQuad>,
    scrollbar: Option<PaintQuad>,
    bell_flash: Option<PaintQuad>,
    /// Brief overlay shown when a prompt-jump wraps from first→last
    /// (or vice versa). Mirrors iTerm2's wrap-to-top indicator.
    prompt_jump_flash: Option<PaintQuad>,
    hover_underline: Option<PaintQuad>,
    /// Left-gutter tint marks for OSC 133 prompt/command boundaries.
    prompt_marks: Vec<PaintQuad>,
    /// Per-match background tint for the active literal search.
    search_quads: Vec<PaintQuad>,
    /// Tick marks on the scrollbar track showing where search matches
    /// live across the entire scrollback. Only emitted when the
    /// scrollbar itself is visible.
    search_scrollbar_ticks: Vec<PaintQuad>,
    /// Annotation overlay quads. Painted above terminal text but below
    /// selection and cursor (spec §8 z-order:
    /// cursor > selection > search > annotation > text).
    annotation_quads: Vec<PaintQuad>,
    /// Shaped first-line text for each annotation overlay box, paired
    /// with the pixel origin where the text should paint. Shaped in
    /// prepaint alongside the quads; painted in the paint phase after
    /// the annotation background and border quads so the text sits on
    /// top of the fill.
    annotation_text_lines: Vec<(gpui::ShapedLine, gpui::Point<gpui::Pixels>)>,
}

pub(super) struct PreeditPostShift {
    /// Rectangle to fill with default bg, erasing the shell's
    /// post-caret glyphs at their original on-grid positions.
    erase: PaintQuad,
    /// Re-shaped post-caret substring, using a uniform style (the
    /// underlying per-cell colouring is lost for the duration of
    /// the composition — acceptable since the state is transient).
    post_shape: gpui::ShapedLine,
    /// Where `post_shape` paints. `erase.left + preedit_grid_width`.
    post_origin: gpui::Point<Pixels>,
}

pub(super) struct TerminalTextElement {
    pub(super) view: gpui::Entity<TerminalView>,
}

impl IntoElement for TerminalTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalTextElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.do_prepaint(bounds, window, cx)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.do_paint(bounds, prepaint, window, cx)
    }
}
