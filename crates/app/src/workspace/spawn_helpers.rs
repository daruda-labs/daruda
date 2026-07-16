//! Background-task helpers for `Workspace`. [`spawn_bg_work_and_mutate`]
//! folds the common async shape — run blocking work on the background
//! executor, then mutate on the foreground inside `this.update(cx, ...)` —
//! into one call, absorbing the `await` and `this.update` `Result` handling.

use gpui::{Context, Task};

use super::Workspace;

/// Run `bg` on the background executor, then call `on_result` on the
/// foreground GPUI thread inside `this.update(cx, …)`. Returns the spawned
/// [`Task<()>`]; callers store it (drop cancels in-flight work) or
/// `.detach()` it (fire-and-forget, the `git_ops` convention).
///
/// `R` is the blocking work's return, passed to `on_result`. `F` is
/// `Send + 'static` to cross threads; `G` is not `Send` (GPUI foreground
/// tasks stay on the main thread). The returned task no-ops if the
/// workspace was dropped mid-flight — teardown is the expected terminal
/// state, so it is `SILENT-OK` for the lint script.
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
        let result = cx.background_executor().spawn(async move { bg() }).await;
        // SILENT-OK: workspace teardown is the expected terminal state for this generic helper
        let _ = this.update(cx, |ws, cx| on_result(ws, result, cx));
    })
}
