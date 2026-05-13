//! Background-task helpers for `Workspace`.
//!
//! The dominant async shape across `*_ops.rs` is:
//!
//! 1. snapshot a few read-only fields off `&self`,
//! 2. run blocking work on `cx.background_executor()`,
//! 3. drop back onto the foreground inside `this.update(cx, ...)`
//!    and mutate.
//!
//! [`spawn_bg_work_and_mutate`] folds that pattern into one call so
//! callers only declare the blocking work and the foreground-side
//! mutation; `await`, `detach`, and the `this.update` `Result`
//! handling are absorbed by the helper.

use gpui::{Context, Task};

use super::Workspace;

/// Run `bg` on the background executor, then call `on_result` on
/// the foreground GPUI thread inside `this.update(cx, …)`. Returns
/// the spawned [`Task<()>`]; callers either store it in a struct
/// field (so dropping the workspace cancels in-flight work) or
/// `.detach()` it (fire-and-forget — the existing convention for
/// `git_status_ops`-style spawns).
///
/// Type parameters:
/// - `R` — value the blocking work returns. Lands as the second
///   argument to `on_result`.
/// - `F` — the blocking work itself. Runs on the background
///   executor; must be `Send + 'static` so it can move across
///   threads.
/// - `G` — the foreground continuation. Takes `&mut Workspace`,
///   the `R` produced by `F`, and the workspace `Context`. Not
///   `Send` because GPUI's foreground tasks stay on the main
///   thread.
///
/// The returned task silently no-ops if the workspace was dropped
/// between the background work finishing and the foreground update
/// — that matches the `let _ = this.update(...)` pattern used by
/// every existing call site.
pub(in crate::workspace) fn spawn_bg_work_and_mutate<R, F, G>(
    cx: &mut Context<Workspace>,
    bg: F,
    on_result: G,
) -> Task<()>
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
    G: FnOnce(&mut Workspace, R, &mut Context<Workspace>) + 'static,
{
    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move { bg() })
            .await;
        let _ = this.update(cx, |ws, cx| on_result(ws, result, cx));
    })
}
