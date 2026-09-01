//! The row that reveals tool runs hidden by a response's tail window.

use gpui::{AnyElement, Context, SharedString};

use super::fold_header::window_boundary_row;
use crate::surface::strings as s;
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;

/// The boundary row's copy. Each state names an action, and the open one names
/// the window it returns to rather than the steps it would re-hide: `Hide N
/// earlier steps` is readable as a *description* of the current state precisely
/// when those steps are on screen, which is the misread that made the row's two
/// states indistinguishable.
fn tail_more_label(hidden_steps: usize, kept_steps: usize, collapsed: bool) -> String {
    if collapsed {
        s::agent_chat_tail_more_show(hidden_steps)
    } else {
        s::agent_chat_tail_more_collapse(kept_steps)
    }
}

pub(super) fn tail_more_bar(
    this: &AgentChatView,
    run_start: usize,
    hidden_steps: usize,
    kept_steps: usize,
    collapsed: bool,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    window_boundary_row(
        SharedString::from(format!("agent-chat-tail-{run_start}")),
        FoldKey::Tail(run_start),
        !collapsed,
        SharedString::from(tail_more_label(hidden_steps, kept_steps, collapsed)),
        this.dim_amount,
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each state names an action, and they name *different* counts: closed
    /// promises the hidden steps, open promises the window it returns to. A
    /// label built from the hidden count in both states is what read as a
    /// description of the current state once the steps were on screen.
    #[test]
    fn the_boundary_label_names_the_hidden_count_closed_and_the_kept_count_open() {
        let closed = tail_more_label(6, 5, true);
        let open = tail_more_label(6, 5, false);
        assert_ne!(
            closed, open,
            "an open boundary must not repeat the closed promise"
        );
        assert!(
            closed.contains('6'),
            "closed names the hidden steps: {closed}"
        );
        assert!(
            open.contains('5') && !open.contains('6'),
            "open names the kept steps, not the hidden ones: {open}"
        );
        // Singular is handled on both sides, like the filtered-row placeholder.
        assert_ne!(tail_more_label(1, 5, true), tail_more_label(2, 5, true));
        assert_ne!(tail_more_label(6, 1, false), tail_more_label(6, 2, false));
    }
}
