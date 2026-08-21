//! Flows tab — what the active lane's flow runs are doing, and the one
//! thing there is to do about them.
//!
//! Scoped to the active lane, unlike the status bar chip. A run's history
//! lives in `<lane>/.daruda/flow-runs/`, so a panel spanning lanes could
//! not show a coherent past beside the present; the chip is what answers
//! the cross-lane question.
//!
//! Unlike the chip's popover this surface does not dismiss, which is why
//! it — and not the popover — is where a run's permission question gets
//! answered.
//!
//! Three sections, one per file: the flow files a lane can run
//! ([`files`]), the runs going now ([`live`]), and the ones that already
//! finished ([`past`]). This module only assembles them in that order.

use gpui::{AnyElement, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::layout::{Dock, RightDockSnapshot};

mod files;
mod live;
mod past;

use self::files::{flow_row, new_flow_button};
use self::live::{empty_state, run_row};
use self::past::{past_row, retention_note};

pub(in crate::workspace) fn render(
    snap: &RightDockSnapshot,
    cx: &mut gpui::Context<Dock>,
) -> AnyElement {
    // The dock root sets no text colour or size, so each panel states its
    // own — every sibling here does, and the two spots that did not
    // rendered near-black against the dock.
    let mut body = super::right_panel_body().text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE));
    // The files first, then what they are doing: a person comes here to
    // open a flow at least as often as to watch one, and until now the only
    // way in was knowing the command palette had an entry for it.
    body = body.child(
        crate::ui::SectionHeader::new(strings::right_panel_flows_heading())
            .actions(new_flow_button(snap)),
    );
    if snap.flow_files.is_empty() {
        body = body.child(
            crate::ui::placeholder_text(strings::right_panel_flows_empty())
                .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                .text_color(theme::current(cx).text_subtle),
        );
    } else {
        body = body.children(
            snap.flow_files
                .iter()
                .map(|found| flow_row(found, snap, cx)),
        );
    }
    body = body.child(crate::ui::Divider::horizontal());
    body = body.child(crate::ui::SectionHeader::new(
        strings::right_panel_flow_live_heading(),
    ));
    if snap.flows.is_empty() {
        body = body.child(empty_state(cx));
    } else {
        body = body.children(snap.flows.iter().map(|run| run_row(run, snap, cx)));
    }
    if let Some(history) = snap.flow_history.as_ref() {
        body = body.child(crate::ui::Divider::horizontal());
        body = body.child(crate::ui::SectionHeader::new(
            strings::right_panel_flow_past_heading(),
        ));
        if history.runs().is_empty() {
            body = body.child(
                crate::ui::placeholder_text(strings::right_panel_flow_past_empty())
                    .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                    .text_color(theme::current(cx).text_subtle),
            );
        } else {
            body = body
                .children(history.runs().iter().map(|run| past_row(run, snap, cx)))
                .child(retention_note(cx));
        }
    }
    body.into_any_element()
}
