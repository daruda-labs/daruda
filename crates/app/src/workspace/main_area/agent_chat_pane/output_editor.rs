//! Which tool-output blocks render through a read-only editor, and how tall
//! that embed is.
//!
//! [`output_editor_source`] is the single classifier: a block qualifies only
//! when its whole content is verbatim, non-markdown bytes, so genuine markdown
//! (links, images, mermaid cards) never loses its renderer.

use daruda_acp::ToolOutputBlock;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{AnyWindowHandle, AppContext as _, Context, Entity, Pixels, SharedString, px};

use super::view::AgentChatView;
use crate::ui::highlighter::{PLAIN_LANGUAGE, language_for_name};
use crate::ui::theme;
use crate::workspace::main_area::pane_tree::PaneId;

/// A tool-output block's verbatim body plus the language it can be highlighted
/// as. Borrowed from the block.
pub(in crate::workspace) struct OutputEditorSource<'a> {
    pub(in crate::workspace) text: &'a str,
    /// Registry language name; `None` renders un-highlighted.
    pub(in crate::workspace) language: Option<&'a str>,
}

/// The verbatim body of `block`, when the whole block is one — raw shell bytes,
/// a file's contents, or a `Text` block that is nothing but a single fenced code
/// block (the shape the adapter's `markdownEscape` produces). Anything else keeps
/// its markdown renderer: prose, several fences, and a `mermaid`-tagged fence,
/// whose diagram card (`render/tool.rs`'s `mermaid_fence_element` hook) must keep
/// winning.
///
/// Only the `Text` arm inspects the body, because only `Text` might really be
/// markdown. The other two arms are typed as verbatim upstream, which is what
/// lets a read of a fence-bearing markdown file embed instead of being rejected
/// as ambiguous.
pub(in crate::workspace) fn output_editor_source(
    block: &ToolOutputBlock,
) -> Option<OutputEditorSource<'_>> {
    match block {
        // Never passed through the adapter's markdown escaping, so its bytes
        // are literal by definition — and no fence carries a language.
        ToolOutputBlock::RawText { text, .. } => Some(OutputEditorSource {
            text,
            language: None,
        }),
        ToolOutputBlock::Text {
            text,
            truncated_from,
        } => {
            let (body, tag) = single_fenced_block(text, truncated_from.is_some())?;
            if tag == Some(MERMAID_FENCE_TAG) {
                return None;
            }
            Some(OutputEditorSource {
                text: body,
                language: tag,
            })
        }
        // A file's contents, already unwrapped by
        // `daruda_acp::output_highlight` and carrying its own language — so
        // nothing here inspects the body. That is the point: a read of a
        // markdown file holds fences of its own, and gating on them is what used
        // to drop it (and every unfenced read) onto the markdown path.
        ToolOutputBlock::SourceText { text, language, .. } => Some(OutputEditorSource {
            text,
            language: language.as_deref(),
        }),
        ToolOutputBlock::Image { .. }
        | ToolOutputBlock::Media { .. }
        | ToolOutputBlock::ResourceLink { .. } => None,
    }
}

/// Fence tag whose block renders as a diagram card, not text.
const MERMAID_FENCE_TAG: &str = "mermaid";

/// Backticks a code fence needs at minimum, per CommonMark.
const MIN_FENCE_TICKS: usize = 3;

/// The body and language tag of `text` when it is *exactly* one fenced code
/// block: an opening run of at least [`MIN_FENCE_TICKS`] backticks with an
/// optional language token, a closing line of only backticks at least as long as
/// that run, and nothing outside the two.
///
/// `truncated` waives the closing fence. `daruda_acp`'s 64 KiB output cap cuts
/// from the tail, so the very blocks this embed exists for arrive with their
/// terminator already gone; demanding one would drop them on the markdown path
/// that paints every line.
///
/// A body line whose own backtick run reaches the opening length disqualifies
/// the block: two concatenated fences and a nested fence are indistinguishable
/// here, and mis-reading either would splice unrelated text into one editor. A
/// shorter run is body content, which is how an escalated wrapper (four
/// backticks around a body holding three) still qualifies.
fn single_fenced_block(text: &str, truncated: bool) -> Option<(&str, Option<&str>)> {
    let (open, rest) = text.split_once('\n')?;
    let (ticks, tag) = fence_open(open)?;
    let body = match rest.rsplit_once('\n') {
        Some((body, close)) if is_closing_fence(close, ticks) => body,
        _ if truncated => rest,
        _ => return None,
    };
    if body.lines().any(|line| backtick_run(line) >= ticks) {
        return None;
    }
    Some((body, tag))
}

/// The backtick-run length and language token of a fence-opening line — the
/// token is `None` for a bare fence, `Some(first word)` for an info string.
/// `None` overall for a line that opens no fence.
///
/// Mirrors, not shares, `daruda_acp::output_highlight`'s `unwrap_fence`: that one
/// undoes the adapter's own wrapping of a file it already knows is source, so it
/// need not read a tag, while this classifies arbitrary text and the tag decides
/// both the highlighting and whether a mermaid card wins instead.
fn fence_open(line: &str) -> Option<(usize, Option<&str>)> {
    let ticks = backtick_run(line);
    if ticks < MIN_FENCE_TICKS {
        return None;
    }
    Some((ticks, line[ticks..].split_whitespace().next()))
}

/// Length of `line`'s leading backtick run.
fn backtick_run(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b'`').count()
}

/// A line closing a fence opened with `open_ticks` backticks: nothing but
/// backticks, and at least as long as the opener. Trailing whitespace is
/// tolerated — CommonMark allows it on a closer, as does a CRLF line ending.
fn is_closing_fence(line: &str, open_ticks: usize) -> bool {
    let run = backtick_run(line);
    run >= open_ticks && run == line.trim_end().len()
}

/// Cache key for a tool call's `ix`-th output block's editor.
pub(in crate::workspace) fn output_editor_key(tool_id: &str, ix: usize) -> String {
    format!("{tool_id}#{ix}")
}

/// Content fingerprint (text + language) so the reconciler rebuilds a streamed
/// output that grew and skips one that did not change. Not cryptographic — a
/// collision would only skip a rebuild, the same trade
/// [`diff_source_fingerprint`](super::agent_chat_helpers::diff_source_fingerprint)
/// accepts.
pub(in crate::workspace) fn output_source_fingerprint(src: &OutputEditorSource<'_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.text.hash(&mut hasher);
    src.language.hash(&mut hasher);
    hasher.finish()
}

/// Text extent of a `rows`-row embed — the height its rows alone occupy. Both
/// the value [`bounded_embed_height`] caps and the content extent the embed's
/// vertical thumb measures against, so the thumb appears exactly when the cap
/// engaged.
pub(in crate::workspace) fn embed_text_height(rows: usize) -> Pixels {
    px(rows.max(1) as f32 * theme::AGENT_CHAT_EMBED_ROW_H)
}

/// Height of a `rows`-row embed: its text extent or the cap, whichever is
/// smaller, plus a strip for the custom horizontal thumb — `SCROLLBAR_W` (its
/// height) plus `SCROLLBAR_MARGIN_R` (`horizontal_thumb`'s `.bottom()` inset).
/// Below the cap that strip is empty, so the thumb clears the last text row;
/// once the cap engages the editor fills the whole box and whatever row the
/// strip lands on shows through beneath the thumb.
pub(in crate::workspace) fn bounded_embed_height(rows: usize) -> Pixels {
    let text_h = f32::from(embed_text_height(rows));
    px(text_h.min(theme::AGENT_CHAT_EMBED_MAX_H) + theme::SCROLLBAR_W + theme::SCROLLBAR_MARGIN_R)
}

/// Drop one trailing line terminator, so the editor's row count is the number
/// of lines the output actually has.
///
/// A shell command's output ends with a newline that *terminates* its last
/// line rather than starting another, but the editor's wrapper counts the empty
/// remainder as a row — which would make every output embed one row taller than
/// its content and shift the height cap by a row. The diff embed needs no such
/// trim: `build_diff_editor_model` emits exactly one line per decoration.
fn without_trailing_terminator(mut text: String) -> String {
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    text
}

/// Create + configure the read-only editor for a verbatim output block inside a
/// single window re-entry against the view's stored `window_handle`. `text` is
/// taken by value — it moves straight into the editor state, which is why the
/// reconciler owns it rather than borrowing. `language` is a fence tag or the
/// name a `SourceText` block carries; `None` (or one the registry cannot colour)
/// renders un-highlighted. Returns `None` if the owning window is gone.
pub(in crate::workspace) fn create_output_editor(
    cx: &mut Context<AgentChatView>,
    window_handle: AnyWindowHandle,
    pane_id: PaneId,
    text: String,
    language: Option<&str>,
) -> Option<Entity<crate::ui::InputState>> {
    let language = language.map_or_else(|| SharedString::from(PLAIN_LANGUAGE), language_for_name);
    let text = without_trailing_terminator(text);
    match cx.update_window(window_handle, move |_, window, cx_w| {
        cx_w.new(|cx_state| {
            // Wrapping re-wraps the whole text on every width change and makes
            // `display_rows()` width-dependent, so the chat list's measured row
            // height would move with the pane width. Off, height is a pure
            // function of content — same choice as the file viewer's editor.
            let mut state = crate::ui::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false)
                // The built-in tree-sitter path extracts styles for the visible
                // range only; a `set_highlight_override` would instead colour
                // the entire text on the main thread at creation.
                .code_editor(language)
                // A `Read` is often partial and the adapter's line numbers were
                // already stripped upstream, so a gutter "1" would not be the
                // file's line 1.
                .line_number(false);
            state.set_value(text, window, cx_state);
            state.set_disabled(true, cx_state);
            state
        })
    }) {
        Ok(editor) => Some(editor),
        Err(e) => {
            // Window gone mid-stream — drop this editor; the markdown/monospace
            // fallback renders. Logged so it isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Failed to build agent-chat output editor")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup(format!("agent_chat.output_editor.window_gone.{pane_id}"))
                    .build(),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
