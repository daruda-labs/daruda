//! Markdown editor wrapper over `gpui_component::input::Input`. Two
//! factories with different `InputMode`s:
//!
//! - `make_markdown_state` — `CodeEditor("markdown")` mode with line
//!   numbers + indent guides + tree-sitter syntax highlight. For
//!   structured-markdown surfaces (settings previews, READMEs).
//! - `make_markdown_prose_state` — `AutoGrow(rows, rows)` mode. Plain
//!   textarea feel: fixed `rows`-tall visual height that respects the
//!   row count (CodeEditor mode collapses to a 1-line min_height — see
//!   `input/element.rs:895-908` — so it needs an explicit `.h(...)`,
//!   while AutoGrow naturally uses `rows × line_height` as its
//!   minimum). The task-edit prompt + notes use this.

use crate::ui::theme;
use gpui::{App, AppContext as _, Entity, Hsla, SharedString, Styled as _, Window, px};
use gpui_component::Sizable as _;
use gpui_component::input::{Input, InputState, ScrollWheelBehavior};

/// Re-export so app code (the diff viewer) can build per-row editor
/// decorations without importing `gpui_component` directly.
pub use gpui_component::input::LineDecoration;

/// Render `state` as a bordered editor with the daruda input
/// background. `small()` (not the wrapper default `xsmall`) is
/// intentional — for multi-line prompt / notes surfaces a slightly
/// larger text size is more readable. `.bg(MODAL_INPUT_BG)` overrides
/// gpui_component's default (which pulls `theme.background` and is
/// slightly lighter than the bespoke `TextInput`) so every input on
/// the TaskEdit pane shares the same surface color.
pub fn markdown_editor(state: &Entity<InputState>, cx: &App) -> Input {
    Input::new(state)
        .small()
        .bordered(true)
        .bg(theme::current(cx).modal_input_bg)
        // Follow the config-driven editor font (`font.editor_size`) like the
        // file-viewer editor; `.small()` still owns the padding/height, this
        // overrides only the text size via `refine_style`.
        .text_size(px(theme::editor_font_size(cx)))
}

/// Build an `Entity<InputState>` configured for markdown editing in
/// code-editing style — line-number gutter on, syntax highlight on.
/// Sets the initial text + `rows` baseline so the editor renders with
/// a reasonable height even when its container does not constrain it.
/// Use this for buffers that read as code / structured markdown
/// (READMEs viewed in a settings pane, future MCP config previews,
/// etc.).
pub fn make_markdown_state(
    initial: &str,
    placeholder: impl Into<SharedString>,
    rows: usize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let state = cx.new(|cx| {
        InputState::new(window, cx)
            .code_editor("markdown")
            .placeholder(placeholder)
            .rows(rows)
    });
    apply_initial(state.clone(), initial, window, cx);
    state
}

/// Prose-style markdown buffer — `AutoGrow(rows, rows)`. Renders as a
/// plain bordered textarea of fixed `rows` visual height; longer
/// content scrolls inside the input itself. No line numbers, no
/// language-tagged syntax highlight — prose markdown is easier on the
/// eye without `#` / `**` / etc. being tokenized.
pub fn make_markdown_prose_state(
    initial: &str,
    placeholder: impl Into<SharedString>,
    rows: usize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let state = cx.new(|cx| {
        InputState::new(window, cx)
            .auto_grow(rows, rows)
            .placeholder(placeholder)
    });
    apply_initial(state.clone(), initial, window, cx);
    state
}

/// `set_value` requires `&mut Window` and a `Context<InputState>` so
/// we cannot run it from inside `cx.new`'s closure (that only sees
/// `Context<Self>`, not the workspace `App`).
fn apply_initial(state: Entity<InputState>, initial: &str, window: &mut Window, cx: &mut App) {
    if initial.is_empty() {
        return;
    }
    let initial = initial.to_string();
    state.update(cx, |state, cx| state.set_value(initial, window, cx));
}

/// Shared chrome for the borderless, read-only code/diff editors: no
/// appearance ring, no size-default padding (gutter flush left), the built-in
/// scrollbar suppressed (hosts overlay their own thin daruda thumb), the
/// config-driven editor font size, and the state's own read-only flag
/// forwarded back into the element.
///
/// The read-only forward matters because `Input::render` rewrites
/// `state.disabled = self.disabled` every frame, so an element left at the
/// builder default (`false`) would clobber a `set_disabled(true)` the host
/// applied to the state on the next paint. The base text colour is pinned
/// because `Input` never sets one, so tree-sitter-uncaptured runs (whitespace,
/// unmapped captures) would inherit gpui's default black `text_style` and
/// vanish on a dark theme; `fg` is the host's slot for it — each caller picks
/// it to match whatever background it actually paints on (see
/// [`file_viewer_editor`] vs [`code_diff_viewer`]).
fn code_editor_chrome(state: &Entity<InputState>, fg: Hsla, cx: &App) -> Input {
    Input::new(state)
        .appearance(false)
        .input_padding(false)
        .show_scrollbar(false)
        .disabled(state.read(cx).is_disabled())
        .text_size(px(theme::editor_font_size(cx)))
        .text_color(fg)
}

/// Render `state` as a full-size code editor for the file-viewer pane (raw +
/// diff). Standalone in its own pane, so it scrolls both axes
/// ([`ScrollWheelBehavior::Both`], the default) and fills the body via
/// `.flex()` at the call site. Paints on the UI theme's fixed
/// `file_viewer_bg` editor surface, so the fallback text colour is matched to
/// the *UI* theme's light/dark bit (`HighlightTheme::editor_foreground`, set
/// light-aware by `apply_daruda_palette`).
pub fn file_viewer_editor(state: &Entity<InputState>, cx: &App) -> Input {
    use gpui_component::ActiveTheme as _;
    let fg = cx
        .theme()
        .highlight_theme
        .style
        .editor_foreground
        .unwrap_or_else(|| theme::palette::syntax_theme().default);
    code_editor_chrome(state, fg, cx)
}

/// Render `state` as a diff editor **embedded in an outer vertical scroller**
/// (the agent-chat tool-card transcript). Shares [`file_viewer_editor`]'s
/// chrome but differs on scroll: the wheel scrolls
/// [`ScrollWheelBehavior::Horizontal`] only (a vertical swipe belongs to the
/// transcript list), and the built-in scrollbar is *kept* (unlike the file
/// viewer, which overlays its own vertical thumb) so a long non-wrapped diff
/// line gets a real draggable horizontal bar. The bar auto-hides the vertical
/// axis — the embed is sized to its full height, so there is no vertical
/// overflow. The caller pins an explicit height (`rows × row_h`).
pub fn code_diff_viewer(state: &Entity<InputState>, cx: &App) -> Input {
    // Paints on `theme::agent_chat_bg` (the terminal-preset background
    // mirrored into the pane), not the UI theme's editor surface — so the
    // fallback fg's light/dark pick tracks that background via the same
    // `agent_chat_syntax_is_light` judgment `Workspace::agent_chat_theme_params`
    // uses for the diff's tree-sitter spans, keeping both in lockstep.
    let fg = theme::palette::syntax_theme_of(
        theme::active_syntax_palette(cx),
        theme::agent_chat_syntax_is_light(cx),
    )
    .default;
    code_editor_chrome(state, fg, cx)
        .scroll_wheel(ScrollWheelBehavior::Horizontal)
        .show_scrollbar(true)
}
