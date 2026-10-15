//! Fold ownership, keyed card assembly, and footer navigation.

use super::scaffold::{ToggleTarget, disclosure_row};
use super::*;

/// Each variant owns the toggle and body needed for a valid assembly.
enum FoldBody<'a> {
    SiblingRows(FoldToggle),
    Owned {
        toggle: FoldToggle,
        build: BodyBuilder<'a>,
    },
    Card {
        key: FoldKey,
        build: BodyBuilder<'a>,
    },
}

/// Applies a header's collapse flip to the view. Carries the click's live
/// `&mut Window`: expanding a card materializes its embed editors, and building
/// an editor needs a window the handler is already inside (see
/// `agent_chat_pane::window_access`).
pub(super) type ToggleFn =
    Box<dyn Fn(&mut AgentChatView, &mut Window, &mut Context<AgentChatView>) + 'static>;

/// What flipping a header's chevron changes. Every conversation header keys into
/// the pane's [`FoldState`](crate::fold::FoldState); the plan region is collapsed by its
/// own view flag, so it supplies the flip directly instead. Keeping that the
/// *only* alternative — rather than opening the toggle to a closure everywhere —
/// means a header cannot quietly grow its own fold state.
pub(in crate::render) enum FoldToggle {
    /// A key in the pane's fold state — routed through `toggle_fold`.
    Fold(FoldKey),
    /// Collapse state that lives outside the fold state (the plan region's
    /// `plan_collapsed`).
    External(ToggleFn),
}

impl FoldToggle {
    pub(in crate::render) fn external(
        f: impl Fn(&mut AgentChatView, &mut Window, &mut Context<AgentChatView>) + 'static,
    ) -> Self {
        Self::External(Box::new(f))
    }

    pub(super) fn into_fn(self) -> ToggleFn {
        match self {
            Self::Fold(key) => Box::new(move |view: &mut AgentChatView, window, cx| {
                view.toggle_fold(key.clone(), window, cx)
            }),
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
/// optionally the body it owns. Built through [`FoldRow::section`],
/// [`FoldRow::block`], or [`FoldRow::card`] so the body-ownership axis is named
/// at every call site.
pub(in crate::render) struct FoldRow<'a> {
    id: ElementId,
    expanded: bool,
    target: ToggleTarget,
    header: FoldHeader<'a>,
    body: FoldBody<'a>,
    chrome: Option<ChromeFn<'a>>,
}

impl<'a> FoldRow<'a> {
    /// A section bar whose content is sibling rows in the list (response bar,
    /// tool-group bar).
    pub(in crate::render) fn section(
        id: impl Into<ElementId>,
        toggle: impl Into<FoldToggle>,
        expanded: bool,
        header: FoldHeader<'a>,
    ) -> Self {
        Self {
            id: id.into(),
            expanded,
            target: ToggleTarget::Row,
            header,
            body: FoldBody::SiblingRows(toggle.into()),
            chrome: None,
        }
    }

    /// A block that owns its body. `body` runs only when expanded.
    pub(in crate::render) fn block(
        id: impl Into<ElementId>,
        toggle: impl Into<FoldToggle>,
        expanded: bool,
        header: FoldHeader<'a>,
        body: impl FnOnce(&mut Context<AgentChatView>) -> AnyElement + 'a,
    ) -> Self {
        Self {
            id: id.into(),
            expanded,
            target: ToggleTarget::Row,
            header,
            body: FoldBody::Owned {
                toggle: toggle.into(),
                build: Box::new(body),
            },
            chrome: None,
        }
    }

    /// A keyed card owns its body and always offers a matching collapse footer.
    pub(in crate::render) fn card(
        id: impl Into<ElementId>,
        key: FoldKey,
        expanded: bool,
        header: FoldHeader<'a>,
        body: impl FnOnce(&mut Context<AgentChatView>) -> AnyElement + 'a,
    ) -> Self {
        Self {
            id: id.into(),
            expanded,
            target: ToggleTarget::Row,
            header,
            body: FoldBody::Card {
                key,
                build: Box::new(body),
            },
            chrome: None,
        }
    }

    /// Bind the toggle to the chevron alone, leaving the rest of the header
    /// inert for text selection.
    pub(in crate::render) fn toggle_on_chevron(mut self) -> Self {
        self.target = ToggleTarget::Chevron;
        self
    }

    /// Style the header row itself (the diff block's hunk background + padding).
    /// Runs on the assembled row, so it cannot disturb slot geometry.
    pub(in crate::render) fn chrome(
        mut self,
        f: impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'a,
    ) -> Self {
        self.chrome = Some(Box::new(f));
        self
    }

    pub(in crate::render) fn render(self, dim: f32, cx: &mut Context<AgentChatView>) -> AnyElement {
        let Self {
            id,
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
            StretchSlot::Title(title) => Some(
                stretch_container(has_leading)
                    .child(title)
                    .into_any_element(),
            ),
            slot => slot
                .builder_for(expanded)
                .and_then(|build| build())
                .map(|line| summary_element(line, has_leading, dim, cx)),
        };

        let (toggle, build, footer_key) = match body {
            FoldBody::SiblingRows(toggle) => (toggle, None, None),
            FoldBody::Owned { toggle, build } => (toggle, Some(build), None),
            FoldBody::Card { key, build } => (
                FoldToggle::Fold(key.clone()),
                Some(build),
                expanded.then_some(key),
            ),
        };
        let footer = footer_key.map(|key| {
            (
                key,
                std::rc::Rc::new(std::cell::Cell::new(CollapseAnchor::Unmeasured)),
            )
        });
        let footer_id = (id.clone(), "collapse");
        let mut row = disclosure_row(id, toggle, expanded, target, dim, cx);
        if let Some((_, anchor)) = &footer {
            let captured = anchor.clone();
            row = row.relative().child(
                gpui::canvas(
                    move |bounds, _, _| captured.set(CollapseAnchor::WindowTop(bounds.origin.y)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        }
        if let Some(leading) = leading {
            row = row.child(leading);
        }
        // No stretching content → a bare spacer, so `trailing` still sits at the
        // right edge instead of hugging the label.
        row = match stretch_el {
            Some(el) => row.child(el),
            None => row.child(div().flex_1()),
        };
        // Trailing items carry their own, wider gap: the row's is tuned for the
        // chevron sitting close to its label, which leaves a count touching the
        // mark beside it.
        if !trailing.is_empty() {
            row = row.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::AGENT_CHAT_TRAILING_GAP))
                    .children(trailing),
            );
        }
        let row = match chrome {
            Some(f) => f(row),
            None => row,
        };

        let body_el = if expanded {
            build.map(|build| build(cx))
        } else {
            None
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_CHAT_MSG_GAP))
            .child(row)
            .children(body_el)
            .children(
                footer.map(|(key, anchor)| collapse_footer_button(footer_id, key, anchor, dim, cx)),
            )
            .into_any_element()
    }
}

/// A card footer invokes the same fold state as its header.
/// Shared by tools, nested subagents, and retained compaction summaries.
fn collapse_footer_button(
    id: impl Into<ElementId>,
    key: FoldKey,
    header_top: std::rc::Rc<std::cell::Cell<CollapseAnchor>>,
    dim: f32,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let selector_key = key.clone();
    div()
        .debug_selector(move || format!("fold-footer-{selector_key:?}"))
        .flex()
        .child(
            daruda_ui::compact_button(daruda_ui::button_on_surface(
                id,
                crate::strings::agent_chat::collapse_card(),
                &theme::PaneSurfaceTokens::agent_chat(cx).dimmed(dim),
                cx,
            ))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.collapse_card(key.clone(), header_top.get(), window, cx)
            })),
        )
        .into_any_element()
}
