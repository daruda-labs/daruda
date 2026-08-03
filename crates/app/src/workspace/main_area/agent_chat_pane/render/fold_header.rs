//! The single source for every collapsible header in the agent chat: the
//! disclosure scaffold, the three-slot header grammar, and the two body-ownership
//! kinds.
//!
//! **Why this module exists.** A fold header is one row with three slots —
//! `leading` (fixed label), `stretch` (the one slot that eats the leftover width),
//! `trailing` (fixed, right-anchored). That grammar used to be expressed as two
//! free `AnyElement` parameters, so each header re-derived the spacing and picked
//! its own two of the three slots: the response bar lost the label→summary gap,
//! the tool card gave up the summary slot to keep its badge right-anchored, and
//! the diff block pushed a right-anchored `+N −M` through the summary slot.
//! Narrowing the slots to domain types ([`SummaryLine`], [`StretchSlot`]) removes
//! the escape hatch: the gap and the truncation idiom exist once, here, and a
//! caller cannot restate them. Enforced by `scripts/lint-fold-header.sh`.
//!
//! Every collapsible header in the pane routes through here — the conversation's
//! seven plus the plan region, whose collapse state is a view flag rather than a
//! [`FoldKey`] and so arrives as [`FoldToggle::External`].

use gpui::{
    AnyElement, Context, Div, ElementId, IntoElement, SharedString, Stateful, div, prelude::*, px,
};

use super::pulse_opacity;
use crate::ui::theme;
use crate::ui::{Disclosure, disclosure};
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::{
    Rollup, summary_preview_line,
};
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

/// Where the fold-toggle click lives on a [`FoldRow`] header: the whole row, or
/// the chevron glyph alone. Selected through [`FoldRow::toggle_on_chevron`], so it
/// stays private to the assembly.
#[derive(Clone, Copy)]
enum ToggleTarget {
    /// The whole header row toggles the fold — generous hit area (section bars,
    /// text blocks).
    Row,
    /// Only the chevron toggles; the rest of the header stays inert so a header
    /// carrying selectable content (the tool-card title) doesn't fight text
    /// selection.
    Chevron,
}

/// Visual treatment of a [`SummaryLine`]. An enum rather than a bare `italic`
/// flag so the call site names the *kind* of summary it has, not the styling.
#[derive(Clone, Copy)]
enum SummaryTone {
    /// Plain prose — an assistant reply, a conclusion, a tool title, a count.
    Prose,
    /// Agent reasoning — italic, matching the thinking block's treatment.
    Reasoning,
}

/// The one-line preview shown in a collapsed header's `stretch` slot.
///
/// There is no constructor taking a bare `String`: a summary is either derived
/// from markdown (which forces the inline-markdown flattening — `**bold**` reads
/// as prose, never as raw `**`) or an already-composed plain phrase the caller
/// names as such. That closes the path by which the response bar used to show a
/// raw source line while every other header showed flattened text.
pub(super) struct SummaryLine {
    text: String,
    tone: SummaryTone,
}

impl SummaryLine {
    /// The first non-empty line of a markdown body, inline markup flattened via
    /// [`summary_preview_line`]. `None` when there is nothing to summarize.
    pub(super) fn from_markdown(text: &str) -> Option<Self> {
        Some(Self {
            text: summary_preview_line(text)?,
            tone: SummaryTone::Prose,
        })
    }

    /// An already-composed phrase that is not markdown — a localized count, a
    /// tool title. Named `plain` so a markdown body can't reach this path by
    /// accident.
    pub(super) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: SummaryTone::Prose,
        }
    }

    /// Mark this summary as agent reasoning (italic).
    pub(super) fn reasoning(mut self) -> Self {
        self.tone = SummaryTone::Reasoning;
        self
    }
}

/// Builds a collapsed header's summary on demand.
type SummaryBuilder<'a> = Box<dyn FnOnce() -> Option<SummaryLine> + 'a>;

/// Builds an expanded [`FoldRow`]'s body on demand.
type BodyBuilder<'a> = Box<dyn FnOnce(&mut Context<AgentChatView>) -> AnyElement + 'a>;

/// Styles the assembled header row (the diff block's hunk background).
type ChromeFn<'a> = Box<dyn FnOnce(Stateful<Div>) -> Stateful<Div> + 'a>;

/// The single slot in a header row that consumes the leftover width. Exactly one
/// of these per header, chosen at construction — so "a summary *and* a title" is
/// not representable.
enum StretchSlot<'a> {
    /// A collapsed-only preview. Built lazily: when the row is expanded the
    /// closure is never called, so a collapsed-state-only string is not computed
    /// (nor a markdown body parsed) for an expanded row.
    Summary(SummaryBuilder<'a>),
    /// An identifier shown in both fold states — a file path, a tool label.
    /// Stays an element because it can carry its own font / colour / selection.
    Title(AnyElement),
    /// Nothing stretches; the slot becomes a spacer that pushes `trailing` right.
    Spacer,
}

/// One fold header row's content, in the three-slot grammar.
pub(super) struct FoldHeader<'a> {
    /// Fixed-width label at the left — agent name, "Thinking", the ⚙ marker, a
    /// tool icon + kind. `None` for a header that leads straight into its
    /// stretch slot (the conclusion, a diff path).
    leading: Option<AnyElement>,
    stretch: StretchSlot<'a>,
    /// Fixed-width, right-anchored, in order — rollup glyph, tool count, tool
    /// status badge, `+N −M`, the plan's live-step dot and dismiss button.
    ///
    /// Trailing content is **fold-state-independent**: a badge reads the same
    /// expanded or collapsed. Only the `stretch` slot's summary is collapsed-only.
    trailing: Vec<AnyElement>,
}

impl<'a> FoldHeader<'a> {
    /// A header whose stretch slot holds a collapsed-only summary.
    pub(super) fn with_summary(f: impl FnOnce() -> Option<SummaryLine> + 'a) -> Self {
        Self {
            leading: None,
            stretch: StretchSlot::Summary(Box::new(f)),
            trailing: Vec::new(),
        }
    }

    /// A header whose stretch slot holds an identifier shown in both states.
    pub(super) fn with_title(title: AnyElement) -> Self {
        Self {
            leading: None,
            stretch: StretchSlot::Title(title),
            trailing: Vec::new(),
        }
    }

    /// A header with no stretching content — label and trailing only.
    pub(super) fn bare() -> Self {
        Self {
            leading: None,
            stretch: StretchSlot::Spacer,
            trailing: Vec::new(),
        }
    }

    pub(super) fn leading(mut self, el: AnyElement) -> Self {
        self.leading = Some(el);
        self
    }

    pub(super) fn trailing(mut self, el: AnyElement) -> Self {
        self.trailing.push(el);
        self
    }
}

/// Whether the folded content is this element's own child or a sibling row.
enum FoldBody<'a> {
    /// Built as a child when expanded — text blocks, tool cards, diffs. Lazy:
    /// a collapsed row never builds its body (GPUI has no partial redraw, so a
    /// discarded body is wasted work on every frame the row is visible).
    Owned(BodyBuilder<'a>),
    /// The content is separate rows in the virtualized list, hidden by
    /// `rows::project` — the response bar and the tool-group bar. Such a header
    /// cannot own a body, which is why they could not reuse the block assembly
    /// before this module existed.
    SiblingRows,
}

/// Applies a header's collapse flip to the view.
type ToggleFn = Box<dyn Fn(&mut AgentChatView, &mut Context<AgentChatView>) + 'static>;

/// What flipping a header's chevron changes. Every conversation header keys into
/// the pane's [`FoldState`](super::FoldState); the plan region is collapsed by its
/// own view flag, so it supplies the flip directly instead. Keeping that the
/// *only* alternative — rather than opening the toggle to a closure everywhere —
/// means a header cannot quietly grow its own fold state.
pub(super) enum FoldToggle {
    /// A key in the pane's fold state — routed through `toggle_fold`.
    Fold(FoldKey),
    /// Collapse state that lives outside the fold state (the plan region's
    /// `plan_collapsed`).
    External(ToggleFn),
}

impl FoldToggle {
    pub(super) fn external(
        f: impl Fn(&mut AgentChatView, &mut Context<AgentChatView>) + 'static,
    ) -> Self {
        Self::External(Box::new(f))
    }

    fn into_fn(self) -> ToggleFn {
        match self {
            Self::Fold(key) => {
                Box::new(move |view: &mut AgentChatView, cx| view.toggle_fold(key.clone(), cx))
            }
            Self::External(f) => f,
        }
    }
}

impl From<FoldKey> for FoldToggle {
    fn from(key: FoldKey) -> Self {
        Self::Fold(key)
    }
}

/// A collapsible header row: the disclosure scaffold plus a [`FoldHeader`], and
/// optionally the body it owns. Built through [`FoldRow::section`] or
/// [`FoldRow::block`] so the body-ownership axis is named at every call site.
pub(super) struct FoldRow<'a> {
    id: ElementId,
    toggle: FoldToggle,
    expanded: bool,
    target: ToggleTarget,
    header: FoldHeader<'a>,
    body: FoldBody<'a>,
    chrome: Option<ChromeFn<'a>>,
}

impl<'a> FoldRow<'a> {
    /// A section bar whose content is sibling rows in the list (response bar,
    /// tool-group bar).
    pub(super) fn section(
        id: impl Into<ElementId>,
        toggle: impl Into<FoldToggle>,
        expanded: bool,
        header: FoldHeader<'a>,
    ) -> Self {
        Self {
            id: id.into(),
            toggle: toggle.into(),
            expanded,
            target: ToggleTarget::Row,
            header,
            body: FoldBody::SiblingRows,
            chrome: None,
        }
    }

    /// A block that owns its body. `body` runs only when expanded.
    pub(super) fn block(
        id: impl Into<ElementId>,
        toggle: impl Into<FoldToggle>,
        expanded: bool,
        header: FoldHeader<'a>,
        body: impl FnOnce(&mut Context<AgentChatView>) -> AnyElement + 'a,
    ) -> Self {
        Self {
            id: id.into(),
            toggle: toggle.into(),
            expanded,
            target: ToggleTarget::Row,
            header,
            body: FoldBody::Owned(Box::new(body)),
            chrome: None,
        }
    }

    /// Bind the toggle to the chevron alone, leaving the rest of the header
    /// inert for text selection.
    pub(super) fn toggle_on_chevron(mut self) -> Self {
        self.target = ToggleTarget::Chevron;
        self
    }

    /// Style the header row itself (the diff block's hunk background + padding).
    /// Runs on the assembled row, so it cannot disturb slot geometry.
    pub(super) fn chrome(mut self, f: impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'a) -> Self {
        self.chrome = Some(Box::new(f));
        self
    }

    pub(super) fn render(self, dim: f32, cx: &mut Context<AgentChatView>) -> AnyElement {
        let Self {
            id,
            toggle,
            expanded,
            target,
            header,
            body,
            chrome,
        } = self;
        let FoldHeader {
            leading,
            stretch,
            trailing,
        } = header;
        let has_leading = leading.is_some();

        // The one place the stretch slot's geometry is decided.
        let stretch_el: Option<AnyElement> = match stretch {
            StretchSlot::Summary(build) if !expanded => {
                build().map(|line| summary_element(line, has_leading, dim, cx))
            }
            StretchSlot::Summary(_) | StretchSlot::Spacer => None,
            StretchSlot::Title(title) => Some(
                stretch_container(has_leading)
                    .child(title)
                    .into_any_element(),
            ),
        };

        let mut row = disclosure_row(id, toggle, expanded, target, dim, cx);
        if let Some(leading) = leading {
            row = row.child(leading);
        }
        // No stretching content → a bare spacer, so `trailing` still sits at the
        // right edge instead of hugging the label.
        row = match stretch_el {
            Some(el) => row.child(el),
            None => row.child(div().flex_1()),
        };
        for el in trailing {
            row = row.child(el);
        }
        let row = match chrome {
            Some(f) => f(row),
            None => row,
        };

        let body_el = match body {
            FoldBody::Owned(build) if expanded => Some(build(cx)),
            FoldBody::Owned(_) | FoldBody::SiblingRows => None,
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .child(row)
            .children(body_el)
            .into_any_element()
    }
}

/// The stretch slot's container: takes the leftover width and truncates to one
/// ellipsized line by layout (`flex_1` + `min_w_0` + `overflow_hidden` +
/// `whitespace_nowrap` + `text_ellipsis`) rather than a character cap — the same
/// idiom the activity-bar title and the plan region's live-step preview use, so
/// an over-long header reads as truncated instead of hard-clipped mid-glyph. When
/// a `leading` label precedes it, [`theme::AGENT_CHAT_SUMMARY_GAP`] separates the
/// two so they don't run together at the tight row gap.
fn stretch_container(has_leading: bool) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .when(has_leading, |el| el.ml(px(theme::AGENT_CHAT_SUMMARY_GAP)))
}

/// Render a [`SummaryLine`] into the stretch slot.
fn summary_element(
    line: SummaryLine,
    has_leading: bool,
    dim: f32,
    cx: &Context<AgentChatView>,
) -> AnyElement {
    stretch_container(has_leading)
        .when(matches!(line.tone, SummaryTone::Reasoning), |el| {
            el.italic()
        })
        .text_color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(line.text))
        .into_any_element()
}

/// The disclosure scaffold shared by every fold header: a full-width borderless
/// row that applies `toggle` and leads with the chevron. `target` picks where the
/// click lives.
fn disclosure_row(
    base: ElementId,
    toggle: FoldToggle,
    expanded: bool,
    target: ToggleTarget,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> Stateful<Div> {
    // One base id yields both the row's click target and the chevron glyph's
    // identity — distinct yet stable across renders.
    let chevron: Disclosure = disclosure((base.clone(), "chevron"), expanded)
        .color(theme::dim_toward_gray(theme::agent_chat_fg_subtle(cx), dim));
    let row = div()
        .id((base, "row"))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP));
    let toggle = toggle.into_fn();
    match target {
        ToggleTarget::Row => row
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| toggle(this, cx)))
            .child(chevron),
        // Bind the click to the chevron itself; the row carries no click
        // handler, so selectable header content stays freely selectable.
        ToggleTarget::Chevron => row
            .child(chevron.on_toggle(cx.listener(move |this, _ev, _window, cx| toggle(this, cx)))),
    }
}

/// The standard trailing-slot glyph summarizing a run's outcome. Every header
/// that represents a whole response carries exactly one — the response bar, the
/// tool-group bar, and a top-level assistant block (which *is* the whole response
/// when the run is trivial enough that no bar is emitted). Blocks nested under a
/// bar stay glyph-free so a collapsed turn shows one verdict, not two.
pub(super) fn rollup_glyph(rollup: Rollup, t: &theme::DarudaTheme, cx: &gpui::App) -> AnyElement {
    let (glyph, color) = match rollup {
        // Amber "executing tool" accent so an in-progress run reads stronger
        // than a settled glyph.
        Rollup::Running => ("●", t.status_executing_tool_dark),
        Rollup::Ok => ("✓", t.file_diff_stat_add),
        // Partial = some failed, some succeeded → warning, not a hard failure.
        Rollup::Partial => ("⚠", t.banner_warning_text),
        Rollup::Failed => ("✗", t.banner_error_text),
    };
    // Blink the running dot on the shared 2-tick pulse so it reads as live;
    // settled glyphs stay solid.
    let opacity = if matches!(rollup, Rollup::Running) {
        pulse_opacity(cx)
    } else {
        1.0
    };
    div()
        .flex_none()
        .opacity(opacity)
        .text_color(color)
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(glyph))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::SummaryLine;

    #[test]
    fn summary_from_markdown_flattens_inline_markup() {
        // The only markdown entry point, so every header preview is flattened —
        // a raw `**` can no longer reach one header while another shows prose.
        let line = SummaryLine::from_markdown("**Planning the change** and more")
            .expect("non-empty body yields a summary");
        assert_eq!(line.text, "Planning the change and more");
    }

    #[test]
    fn summary_from_markdown_is_none_without_a_visible_line() {
        assert!(SummaryLine::from_markdown("   \n\t\n").is_none());
    }

    #[test]
    fn plain_summary_is_verbatim() {
        assert_eq!(SummaryLine::plain("3 tool calls").text, "3 tool calls");
    }
}
