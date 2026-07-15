//! Hover-reveal copy button for rendered-markdown code blocks.
//!
//! Ports zed's `CopyButton` feedback pattern (checkmark on click, then a 2s
//! revert driven by a *targeted* `cx.notify` — never `window.refresh`, per the
//! render-cost containment rule) onto daruda's `button_bare` + `IconName`
//! chrome, mirroring the sibling mermaid copy button.
//!
//! The button renders inside the vendored `TextView` code-block actions overlay
//! (`crates/gpui_component/src/text/node.rs`), which is hover-revealed against
//! the `"gpui-code-block"` group, so the button itself stays presentation-neutral
//! (always visible within its conditionally-visible parent).
//!
//! State (the "just copied" instant) is owned by GPUI keyed state, keyed by the
//! caller-supplied `id`, so there is no per-block entity to manage — the same
//! GPUI-owned-view-state exception the markdown selection state relies on.

use std::time::{Duration, Instant};

use gpui::{App, ClipboardItem, ElementId, IntoElement, SharedString, Window};

use crate::ui::{IconName, button_bare};

/// How long the button shows the copied (✓) state before reverting to the
/// copy icon.
const COPIED_STATE_DURATION: Duration = Duration::from_secs(2);

/// Per-button "just copied" feedback state, owned by GPUI keyed state.
struct CopyState {
    copied_at: Option<Instant>,
}

impl CopyState {
    /// True while the last copy is still within the feedback window.
    fn is_copied(&self) -> bool {
        self.copied_at
            .map(|t| t.elapsed() < COPIED_STATE_DURATION)
            .unwrap_or(false)
    }
}

/// A copy-to-clipboard button for a rendered-markdown code block. Clicking
/// writes `code` to the clipboard and flips the icon to a ✓ (with a "Copied"
/// tooltip) for [`COPIED_STATE_DURATION`], then reverts.
///
/// `id` must be stable per code block across renders, or the keyed feedback
/// state resets. The markdown wiring derives it from the block's own id plus a
/// content hash.
pub fn code_copy_button<I: Into<ElementId>>(
    id: I,
    code: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<I> {
    let id = id.into();
    let state = window.use_keyed_state(id.clone(), cx, |_, _| CopyState { copied_at: None });
    let is_copied = state.read(cx).is_copied();

    let (icon, tooltip) = if is_copied {
        (
            IconName::Check,
            crate::surface::strings::code_block_copied(),
        )
    } else {
        (IconName::Copy, crate::surface::strings::code_block_copy())
    };

    button_bare(id)
        .icon(icon)
        .tooltip(tooltip)
        .on_click(move |_, _window, cx| {
            // Keep the click from bubbling to an ancestor click handler (e.g. a
            // fold/row toggle behind the block). It cannot cancel a text
            // selection — that starts on mouse-down, before this mouse-up click.
            cx.stop_propagation();
            state.update(cx, |s, _| s.copied_at = Some(Instant::now()));
            cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
            // Revert the ✓ after the feedback window via a targeted notify on
            // the state entity (never `window.refresh` — pitfall #10).
            let state_id = state.entity_id();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(COPIED_STATE_DURATION).await;
                // SILENT-OK: app/window gone → the revert-notify is moot.
                cx.update(|cx| cx.notify(state_id))
            })
            .detach();
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn just_copied_reads_as_copied() {
        let state = CopyState {
            copied_at: Some(Instant::now()),
        };
        assert!(state.is_copied());
    }

    #[test]
    fn expired_copy_reads_as_not_copied() {
        let state = CopyState {
            copied_at: Some(Instant::now() - COPIED_STATE_DURATION - Duration::from_millis(1)),
        };
        assert!(!state.is_copied());
    }

    #[test]
    fn never_copied_reads_as_not_copied() {
        let state = CopyState { copied_at: None };
        assert!(!state.is_copied());
    }
}
