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
use gpui::{App, AppContext as _, Entity, SharedString, Styled as _, Window, px};
use gpui_component::Sizable as _;
use gpui_component::input::{Input, InputState};

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

/// Render `state` as a full-size code editor for the file-viewer raw pane.
/// `appearance(false)` hides the focus-ring border; the parent sets the bg.
/// `input_padding(false)` drops the size-default inner padding so the
/// line-number gutter sits flush left like the markdown-raw / preview
/// renderers (the body frame owns any surrounding spacing).
pub fn file_viewer_editor(state: &Entity<InputState>, cx: &App) -> Input {
    use gpui_component::ActiveTheme as _;
    // Suppress the built-in scrollbar; the file viewer overlays a thin
    // daruda thumb so raw and diff match the other viewer modes.
    //
    // Pin the base text color. `Input` never sets one, so code-editor runs
    // left uncoloured by tree-sitter (uncaptured identifiers, whitespace,
    // any capture without a `SyntaxColors` mapping) inherit gpui's default
    // black `window.text_style()` and vanish on the dark theme. The
    // `HighlightTheme::editor_foreground` slot is upstream's home for this,
    // but gpui_component never applies it as a text color — the host must.
    // Read it back from the live theme (set light-aware by
    // `apply_daruda_palette`) rather than the fixed dark-variant default,
    // so uncaptured runs stay legible on the light theme too.
    let fg = cx
        .theme()
        .highlight_theme
        .style
        .editor_foreground
        .unwrap_or_else(|| theme::palette::syntax_theme().default);
    // Size the editor text from the config-driven file-viewer font
    // (`font.editor_size`). `input.rs` applies `input_text_size(self.size)`
    // and then `refine_style(self.style)`, so this explicit `text_size`
    // (which lands in `self.style`) wins over the Sizable default — the
    // raw + diff editor and its line-number gutter render at the
    // configured size instead of gpui_component's `text_sm`.
    Input::new(state)
        .appearance(false)
        .input_padding(false)
        .show_scrollbar(false)
        .text_size(px(theme::editor_font_size(cx)))
        .text_color(fg)
}
