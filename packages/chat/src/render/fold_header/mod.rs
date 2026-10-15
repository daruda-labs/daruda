//! The single source for every collapsible header in the agent chat: the
//! disclosure scaffold, the three-slot header grammar, and the body-ownership
//! kinds (section, block, and card).
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
//! nine plus the plan region, whose collapse state is a view flag rather than a
//! [`FoldKey`] and so arrives as [`FoldToggle::External`].

use gpui::{
    AnyElement, Context, Div, ElementId, IntoElement, SharedString, Stateful, Window, div,
    prelude::*, px,
};

use crate::agent_chat_helpers::{Rollup, summary_preview_line};
use crate::fold::FoldKey;
use crate::view::AgentChatView;
use crate::view::list_sync::CollapseAnchor;
use daruda_ui::theme;
use daruda_ui::{Disclosure, DisclosureAxis, disclosure};

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
pub(in crate::render) struct SummaryLine {
    text: String,
    tone: SummaryTone,
}

impl SummaryLine {
    /// The first non-empty line of a markdown body, inline markup flattened via
    /// [`summary_preview_line`]. `None` when there is nothing to summarize.
    pub(in crate::render) fn from_markdown(text: &str) -> Option<Self> {
        Some(Self {
            text: summary_preview_line(text)?,
            tone: SummaryTone::Prose,
        })
    }

    /// An already-composed phrase that is not markdown — a localized count, a
    /// tool title. Named `plain` so a markdown body can't reach this path by
    /// accident.
    pub(in crate::render) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: SummaryTone::Prose,
        }
    }

    /// Mark this summary as agent reasoning (italic).
    pub(in crate::render) fn reasoning(mut self) -> Self {
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
    /// A collapsed-only preview — the header's proxy for a body the fold hides,
    /// which the body itself restates once expanded. Built lazily: when the row
    /// is expanded the closure is never called, so a collapsed-state-only string
    /// is not computed (nor a markdown body parsed) for an expanded row.
    Summary(SummaryBuilder<'a>),
    /// An identifier shown in both fold states — a file path, a tool label.
    /// Stays an element because it can carry its own font / colour / selection.
    Title(AnyElement),
    /// Nothing stretches; the slot becomes a spacer that pushes `trailing` right.
    Spacer,
}

impl<'a> StretchSlot<'a> {
    /// The builder this slot runs in the given fold state, if it has one.
    ///
    /// Split out of `render` so which state builds a summary is a decision a
    /// test can make without a window. [`Self::Title`] is absent on purpose: it
    /// is an element the caller already built, not a summary this slot builds,
    /// so `render` places it before consulting this.
    fn builder_for(self, expanded: bool) -> Option<SummaryBuilder<'a>> {
        match self {
            Self::Summary(build) if !expanded => Some(build),
            Self::Summary(_) | Self::Title(_) | Self::Spacer => None,
        }
    }
}

/// One fold header row's content, in the three-slot grammar.
pub(in crate::render) struct FoldHeader<'a> {
    /// Fixed-width label at the left — agent name, "Thinking", a tool icon +
    /// kind. `None` for a header that leads straight into its stretch slot (the
    /// conclusion, a group's count, a diff path).
    leading: Option<AnyElement>,
    stretch: StretchSlot<'a>,
    /// Fixed-width, right-anchored, in order — rollup glyph, tool count, tool
    /// status badge, `+N −M`, the plan's live-step dot and dismiss button.
    ///
    /// Trailing content is **fold-state-independent**: a badge reads the same
    /// expanded or collapsed. The `stretch` slot is the only one that varies —
    /// it is hidden once expanded ([`StretchSlot::Summary`]).
    trailing: Vec<AnyElement>,
}

impl<'a> FoldHeader<'a> {
    /// A header whose stretch slot holds a collapsed-only summary.
    pub(in crate::render) fn with_summary(f: impl FnOnce() -> Option<SummaryLine> + 'a) -> Self {
        Self {
            leading: None,
            stretch: StretchSlot::Summary(Box::new(f)),
            trailing: Vec::new(),
        }
    }

    /// A header whose stretch slot holds an identifier shown in both states.
    pub(in crate::render) fn with_title(title: AnyElement) -> Self {
        Self {
            leading: None,
            stretch: StretchSlot::Title(title),
            trailing: Vec::new(),
        }
    }

    /// [`Self::with_title`] for a title that is itself clickable (the diff
    /// block's file path, which opens the file). A gpui `div` is
    /// `display: block`, so a lone child of [`stretch_container`] fills the
    /// slot's whole width — harmless for a label, wrong for a hitbox: it would
    /// swallow clicks on the empty run out to the trailing slot. Sitting the
    /// title in a flex row sizes it to its own content; `min_w_0` keeps a long
    /// one shrinking into the slot's ellipsis instead of pushing `trailing` off
    /// the row. Named as its own constructor so the geometry stays here rather
    /// than being restated by each interactive header.
    pub(in crate::render) fn with_interactive_title(title: impl IntoElement) -> Self {
        Self::with_title(interactive_title(title))
    }

    /// A header with no stretching content — label and trailing only.
    pub(in crate::render) fn bare() -> Self {
        Self {
            leading: None,
            stretch: StretchSlot::Spacer,
            trailing: Vec::new(),
        }
    }

    pub(in crate::render) fn leading(mut self, el: AnyElement) -> Self {
        self.leading = Some(el);
        self
    }

    pub(in crate::render) fn trailing(mut self, el: AnyElement) -> Self {
        self.trailing.push(el);
        self
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

/// Shrink-wrap an interactive title to its own content inside the block-level
/// [`stretch_container`]. See [`FoldHeader::with_interactive_title`] for why.
fn interactive_title(title: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .min_w_0()
        .child(title)
        .into_any_element()
}

/// Render a [`SummaryLine`] into the stretch slot.
fn summary_element(
    line: SummaryLine,
    has_leading: bool,
    dim: f32,
    cx: &Context<AgentChatView>,
) -> AnyElement {
    let color = theme::agent_chat_fg_subtle(cx);
    stretch_container(has_leading)
        .when(matches!(line.tone, SummaryTone::Reasoning), |el| {
            el.italic()
        })
        .text_color(theme::dim_toward_gray(color, dim))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(line.text))
        .into_any_element()
}

mod boundary;
mod row;
mod scaffold;
pub(in crate::render) use boundary::{interrupted_row, rollup_glyph, window_boundary_row};
pub(in crate::render) use row::{FoldRow, FoldToggle};
#[cfg(test)]
mod tests;
