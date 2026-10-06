//! Pair persisted workspace mutations with their save call.
//!
//! Use `mutate_durable[_in]`; a sibling lint bans direct
//! `mark_dirty_and_save` elsewhere. Prefer mutating inside the closure. If a
//! borrow cannot cross the closure boundary, perform the mutation first and use
//! an empty closure as the explicit persist marker.

use gpui::{Context, Window};

use crate::workspace::Workspace;

impl Workspace {
    /// Run `f` then schedule persistence for durable workspace state.
    pub(in crate::workspace) fn mutate_durable<F, R>(&mut self, cx: &mut Context<Self>, f: F) -> R
    where
        F: FnOnce(&mut Self, &mut Context<Self>) -> R,
    {
        let r = f(self, cx);
        self.mark_dirty_and_save(cx);
        r
    }

    /// Window-threading variant for action handlers and menu closures.
    pub(in crate::workspace) fn mutate_durable_in<F, R>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    {
        let r = f(self, window, cx);
        self.mark_dirty_and_save(cx);
        r
    }
}

#[cfg(test)]
mod tests {
    // Behavioral coverage lives in workspace/tests/durable.rs —
    // mutate_durable_marks_dirty and mutate_durable_returns_inner_value.
}
