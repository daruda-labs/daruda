//! Wrapper for persistence-coupled mutations. Use `mutate_durable[_in]`
//! instead of calling `mark_dirty_and_save` directly so the mutation
//! and the persist are structurally paired — forgetting the persist
//! becomes a compile error (you can't construct durable state without
//! going through the wrapper), and a sibling lint script enforces no
//! direct `mark_dirty_and_save` calls outside this file and the
//! definition site.
//!
//! # Empty-closure convention
//!
//! Some ops perform multi-stage state changes whose intermediate
//! mutable borrows (e.g. `self.groups.iter_mut().find(...)`, complex
//! pool renumbering across `dnd_ops.rs`) cannot survive being moved
//! across a closure boundary. At those sites the mutation runs first,
//! then the wrapper is called with an empty closure as a persist
//! marker:
//!
//! ```ignore
//! self.groups.push(SerializedGroup { /* ... */ });
//! self.mutate_durable(cx, |_, _| {});  // schedules persist
//! ```
//!
//! The structural coupling weakens at these sites — the persist is
//! paired with the preceding mutation by convention, not by the
//! closure boundary — but the lint still guarantees the persist call
//! is the only path that schedules `mark_dirty_and_save`. New ops
//! should prefer running the mutation inside the closure; reach for
//! the empty-closure form only when the borrow checker rejects it.

use gpui::{Context, Window};

use crate::workspace::Workspace;

impl Workspace {
    /// Run `f` then schedule a persist. Use for any mutation that
    /// changes persisted workspace state (projects, groups, lanes,
    /// tabs, panes, dock geometry, window title, claude session
    /// bindings).
    pub(in crate::workspace) fn mutate_durable<F, R>(&mut self, cx: &mut Context<Self>, f: F) -> R
    where
        F: FnOnce(&mut Self, &mut Context<Self>) -> R,
    {
        let r = f(self, cx);
        self.mark_dirty_and_save(cx);
        r
    }

    /// Same as [`Self::mutate_durable`] but threads `&mut Window`
    /// through to `f`. Use at action handlers and `ws_menu_item`
    /// closures whose signature is `|this, window, cx|`.
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
