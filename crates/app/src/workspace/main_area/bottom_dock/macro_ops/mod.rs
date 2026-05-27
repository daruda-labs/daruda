//! Workspace operations on `PanelsState` — load/save bottom-dock
//! panels (macros, future widget types).
//!
//! Persistence is handled by the `daruda_store::panels` module; this module is
//! the Workspace-side bridge: it owns the data_dir lookup, provides
//! load-or-seed on first launch, and emits `cx.notify()` after
//! mutations so the render layer reflects the change.

use std::path::Path;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::panels::{
    ButtonWidget, MacroKey, PanelTab, PanelsState, TabId, TabLayout, WidgetId, new_tab_id,
    new_widget_id,
};
use gpui::{Context, KeyBinding, Keystroke, SharedString, Window};

use crate::workspace::{RunMacroByShortcut, Workspace};

/// Build the byte payload for a button widget. `auto_enter` appends
/// `\r` (CR — what a real Enter keystroke produces; never `\n`,
/// which would skip line discipline interpretation in cooked mode).
fn button_payload(btn: &ButtonWidget) -> String {
    if btn.auto_enter {
        format!("{}\r", btn.send)
    } else {
        btn.send.clone()
    }
}

/// Remove the tab with `tab_id` from `tabs`, returning the removed
/// tab on success. `None` when the id was not present (caller should
/// no-op rather than save). Free function so the lookup-and-remove
/// rule is testable without a GPUI `Context`.
fn remove_tab(tabs: &mut Vec<PanelTab>, tab_id: &str) -> Option<PanelTab> {
    let idx = tabs.iter().position(|t| t.id == tab_id)?;
    Some(tabs.remove(idx))
}

/// Pick a fallback `active_tab_id` after deleting the tab whose order
/// was `removed_order`. Strategy:
///   1. Closest tab with a strictly higher order.
///   2. Failing that, the closest tab with a lower order.
///   3. `None` when no tabs remain.
fn pick_fallback_active(tabs: &[PanelTab], removed_order: u32) -> Option<TabId> {
    let mut higher = tabs
        .iter()
        .filter(|t| t.order > removed_order)
        .min_by_key(|t| t.order);
    if higher.is_none() {
        higher = tabs
            .iter()
            .filter(|t| t.order < removed_order)
            .max_by_key(|t| t.order);
    }
    higher.map(|t| t.id.clone())
}

/// Replace a button widget's content while preserving its id and
/// position. Returns `true` on mutation; `false` when either id is
/// missing or the widget at `widget_id` is not a Button (Unknown
/// widgets are skipped — daruda doesn't know their schema).
///
/// `new_btn.id` is overwritten with the existing widget's id so the
/// caller doesn't have to thread it through the modal form.
fn update_button_in_place(
    tabs: &mut [PanelTab],
    tab_id: &str,
    widget_id: &str,
    mut new_btn: ButtonWidget,
) -> bool {
    let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) else {
        return false;
    };
    let Some(widget) = tab.widgets.iter_mut().find(|w| w.id() == Some(widget_id)) else {
        return false;
    };
    let MacroKey::Button(_) = widget else {
        return false;
    };
    new_btn.id = widget_id.to_string();
    *widget = MacroKey::Button(new_btn);
    true
}

/// Remove a widget by id. Returns `true` when removed, `false`
/// when either id was missing.
fn remove_widget_in_place(tabs: &mut [PanelTab], tab_id: &str, widget_id: &str) -> bool {
    let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) else {
        return false;
    };
    let before = tab.widgets.len();
    tab.widgets.retain(|w| w.id() != Some(widget_id));
    tab.widgets.len() != before
}

/// Move `from_id` to the slot of `to_id` and renumber every tab's
/// `order` 0..n in their resulting visual sequence. Returns `true`
/// when the list was mutated (caller persists + notifies), `false`
/// for no-ops (same id, missing id).
///
/// First sorts by `order` so the relative-position math doesn't
/// depend on Vec insertion order — calls into `add_panel_tab` /
/// `delete_panel_tab` may have left the Vec out of sync with the
/// `order` field. After the reorder, both axes line up: Vec index
/// equals `order`.
fn reorder_in_place(tabs: &mut Vec<PanelTab>, from_id: &str, to_id: &str) -> bool {
    if from_id == to_id {
        return false;
    }
    // Sort the Vec by current order so visual position == Vec index.
    tabs.sort_by_key(|t| t.order);
    let from_idx = tabs.iter().position(|t| t.id == from_id);
    let to_idx = tabs.iter().position(|t| t.id == to_id);
    let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) else {
        return false;
    };
    // "Drop source onto target" means the source claims target's slot
    // and target shifts toward source's old position. After remove():
    //   * from < to → to_idx slid left by one, but we want source to
    //     land on target's *original* index, so insert at to_idx (now
    //     the slot just past the shifted target).
    //   * from > to → to_idx unchanged; insert at to_idx places source
    //     ahead of target.
    // Either branch resolves to to_idx, so the conditional is gone.
    let item = tabs.remove(from_idx);
    tabs.insert(to_idx, item);
    for (i, t) in tabs.iter_mut().enumerate() {
        t.order = i as u32;
    }
    true
}

/// Apply a rename to `tabs` in place. Returns `true` when the tabs
/// list was mutated (caller should persist + notify), `false` when
/// the change was rejected. Rejection cases:
///   * Trimmed `new_name` is empty.
///   * `tab_id` doesn't exist in `tabs`.
///   * Trimmed `new_name` equals the current name (no-op).
///
/// Free function so the rule set is unit-testable without a GPUI
/// `Context`.
fn rename_in_place(tabs: &mut [PanelTab], tab_id: &str, new_name: &str) -> bool {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) else {
        return false;
    };
    if tab.name == trimmed {
        return false;
    }
    tab.name = trimmed.to_string();
    true
}

/// Construct a fresh `PanelTab` for `add_panel_tab`. Returns `None`
/// when the trimmed name is empty so the caller can early-return
/// without mutating state. Pulled out as a free function so the
/// tab-building rules (ULID assignment, max(order)+1 placement,
/// FlexWrap layout, no widgets) are testable without a GPUI
/// `Context`.
fn build_new_tab(name: &str, existing: &[PanelTab]) -> Option<PanelTab> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let order = existing
        .iter()
        .map(|t| t.order)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
    Some(PanelTab {
        id: new_tab_id(),
        name: trimmed.to_string(),
        order,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: Vec::new(),
    })
}

/// Find the first widget whose shortcut matches `shortcut`. Returns
/// `(tab_id, widget_id)` so the caller can dispatch via
/// `Workspace::run_widget`. Skips `MacroKey::Unknown` (no shortcut
/// metadata exposed). When two macros share the same shortcut, the
/// **first one in tab/widget visit order** wins — document this so
/// users understand what happens with conflicts.
fn find_widget_by_shortcut(panels: &PanelsState, shortcut: &str) -> Option<(TabId, WidgetId)> {
    for tab in &panels.tabs {
        for widget in &tab.widgets {
            let MacroKey::Button(btn) = widget else {
                continue;
            };
            if btn.shortcut.as_deref() == Some(shortcut) {
                return Some((tab.id.clone(), btn.id.clone()));
            }
        }
    }
    None
}

/// Validate that every whitespace-separated part of `shortcut` is a
/// well-formed keystroke. We pre-flight here because `KeyBinding::new`
/// **panics** on parse error — a single malformed user-defined
/// shortcut would otherwise crash daruda when the binding gets
/// registered.
fn is_valid_shortcut(shortcut: &str) -> bool {
    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed
        .split_whitespace()
        .all(|part| Keystroke::parse(part).is_ok())
}

/// Re-bind every macro shortcut to a `RunMacroByShortcut` action.
/// Called from `Workspace::new_with_project` (initial bind) and
/// `apply_panels_reload` (after external/internal panels.json
/// changes). GPUI bindings are additive (last-wins) — stale
/// bindings whose shortcut no longer matches any macro become no-ops
/// in the handler, so we don't bother trying to "unbind" them.
///
/// Per-macro guards:
///   * Empty shortcut → skip.
///   * Invalid shortcut syntax → log warning, skip (would otherwise
///     panic inside `KeyBinding::new`).
pub(in crate::workspace) fn register_macro_shortcuts(panels: &PanelsState, cx: &mut gpui::App) {
    for tab in &panels.tabs {
        for widget in &tab.widgets {
            let MacroKey::Button(btn) = widget else {
                continue;
            };
            let Some(shortcut) = btn.shortcut.as_deref().filter(|s| !s.is_empty()) else {
                continue;
            };
            if !is_valid_shortcut(shortcut) {
                LogWriter::log(
                    ErrorReport::new("Macro shortcut invalid; binding skipped")
                        .severity(ErrorSeverity::Info)
                        .message(format!(
                            "macro '{}' has invalid shortcut '{}'",
                            btn.label, shortcut
                        ))
                        .at(file!(), line!())
                        .with_context("macro", &btn.label)
                        .with_context("shortcut", shortcut)
                        .dedup("panels.shortcut.invalid")
                        .build(),
                );
                continue;
            }
            cx.bind_keys([KeyBinding::new(
                shortcut,
                RunMacroByShortcut(SharedString::from(shortcut.to_string())),
                None,
            )]);
        }
    }
}

/// Load `panels.json` from `data_dir`. If the file is missing or
/// fails to parse, return `seed_default()` and immediately persist it
/// so the file appears on disk for subsequent launches and external
/// editors.
///
/// After a successful load, `migrate_builtin_flags` is applied to
/// back-fill `builtin: true` on any button whose `send` matches a
/// known seed payload (handles panels.json files written before the
/// `builtin` field was introduced). When migration changes anything
/// the updated state is written back atomically so subsequent launches
/// are no-ops.
pub(in crate::workspace) fn load_or_seed_panels(data_dir: &Path) -> PanelsState {
    if let Some(mut state) = daruda_store::panels::load_panels_in(data_dir) {
        if daruda_store::panels::migrate_builtin_flags(&mut state)
            && let Err(e) = daruda_store::panels::save_panels_in(data_dir, &state)
        {
            LogWriter::log(
                ErrorReport::new("Failed to save panels.json")
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context(
                        "path",
                        daruda_store::observability::system_info::redact_home(
                            daruda_store::panels::panels_path_in(data_dir),
                        ),
                    )
                    .with_context("phase", "init.migrate")
                    .dedup("panels.save")
                    .build(),
            );
        }
        return state;
    }
    let seed = daruda_store::panels::seed_default();
    if let Err(e) = daruda_store::panels::save_panels_in(data_dir, &seed) {
        LogWriter::log(
            ErrorReport::new("Failed to write panels.json")
                .severity(ErrorSeverity::Error)
                .from_error(&e)
                .at(file!(), line!())
                .with_context(
                    "path",
                    daruda_store::observability::system_info::redact_home(
                        daruda_store::panels::panels_path_in(data_dir),
                    ),
                )
                .with_context("phase", "init.seed")
                .dedup("panels.save")
                .build(),
        );
    }
    seed
}

impl Workspace {
    /// Persist `self.panels` to disk **and re-register every macro
    /// shortcut**. Re-binding here (rather than relying on the
    /// watcher to fire `apply_panels_reload`) is required because
    /// the reload path detects daruda's own write as a no-op (disk
    /// equals memory) and skips re-registration — without this hook,
    /// a shortcut just edited via the UI would not take effect until
    /// daruda restarted.
    pub(in crate::workspace) fn save_panels(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = daruda_store::panels::save_panels_in(&self.data_dir, &self.panels) {
            let report = ErrorReport::new("Failed to save panels.json")
                .severity(ErrorSeverity::Error)
                .from_error(&e)
                .at(file!(), line!())
                .with_context(
                    "path",
                    daruda_store::observability::system_info::redact_home(
                        daruda_store::panels::panels_path_in(&self.data_dir),
                    ),
                )
                .with_context("phase", "user.save")
                .dedup("panels.save")
                .build();
            self.report_error(report, cx);
        }
        register_macro_shortcuts(&self.panels, cx);
    }

    /// Append a new tab with the given name (trimmed), switch focus
    /// to it, persist. No-op when the trimmed name is empty — the
    /// modal layer should refuse to submit, but this guard prevents
    /// programmatic callers / malformed external edits from creating
    /// unnamed tabs.
    ///
    /// The new tab starts empty; users edit `panels.json` to add
    /// macros (B-7+ adds an in-app editor).
    pub(in crate::workspace) fn add_panel_tab(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(tab) = build_new_tab(&name, &self.panels.tabs) else {
            return;
        };
        let id = tab.id.clone();
        self.panels.tabs.push(tab);
        self.panels.active_tab_id = Some(id);
        self.save_panels(cx);
        cx.notify();
    }

    /// Rename a tab. No-op when the rename rules in `rename_in_place`
    /// reject the change (see that function for the full set of
    /// guards).
    pub(in crate::workspace) fn rename_panel_tab(
        &mut self,
        tab_id: TabId,
        new_name: String,
        cx: &mut Context<Self>,
    ) {
        if !rename_in_place(&mut self.panels.tabs, &tab_id, &new_name) {
            return;
        }
        self.save_panels(cx);
        cx.notify();
    }

    /// Append a new button widget to the tab. The widget gets a fresh
    /// ULID assigned (`btn.id` is overwritten — callers can fill it
    /// with anything, the modal does so with `new_widget_id()` already
    /// for clarity but this guard makes Create-via-import safe too).
    /// No-op when `tab_id` is missing.
    pub(in crate::workspace) fn add_widget(
        &mut self,
        tab_id: TabId,
        mut btn: ButtonWidget,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.panels.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        btn.id = new_widget_id();
        tab.widgets.push(MacroKey::Button(btn));
        self.save_panels(cx);
        cx.notify();
    }

    /// Replace the button widget identified by `widget_id` with a new
    /// definition (preserving the id). No-op when either id is missing
    /// or the widget is not a Button (Unknown widgets are not
    /// editable; users edit `panels.json` to remove them).
    pub(in crate::workspace) fn update_widget(
        &mut self,
        tab_id: TabId,
        widget_id: WidgetId,
        new_btn: ButtonWidget,
        cx: &mut Context<Self>,
    ) {
        if !update_button_in_place(&mut self.panels.tabs, &tab_id, &widget_id, new_btn) {
            return;
        }
        self.save_panels(cx);
        cx.notify();
    }

    /// Delete a widget by id. No-op when either id is missing.
    pub(in crate::workspace) fn delete_widget(
        &mut self,
        tab_id: TabId,
        widget_id: WidgetId,
        cx: &mut Context<Self>,
    ) {
        if !remove_widget_in_place(&mut self.panels.tabs, &tab_id, &widget_id) {
            return;
        }
        self.save_panels(cx);
        cx.notify();
    }

    /// Move `from_id` to land at the position currently held by
    /// `to_id` in the `panels.tabs` list (visual order driven by the
    /// `order` field). Renumbers `order` 0..n on every remaining
    /// tab so subsequent reorders stay deterministic. No-op when
    /// either id is missing or both are the same.
    pub(in crate::workspace) fn reorder_panel_tab(
        &mut self,
        from_id: TabId,
        to_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if !reorder_in_place(&mut self.panels.tabs, &from_id, &to_id) {
            return;
        }
        self.save_panels(cx);
        cx.notify();
    }

    /// Delete a tab. If the tab was active, the active id falls back
    /// to the next adjacent tab (preferring the same `order` axis):
    ///   * the closest tab with a higher order, or
    ///   * the closest tab with a lower order, or
    ///   * `None` when this was the only tab.
    pub(in crate::workspace) fn delete_panel_tab(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(removed) = remove_tab(&mut self.panels.tabs, &tab_id) else {
            return;
        };
        if self.panels.active_tab_id.as_ref() == Some(&tab_id) {
            self.panels.active_tab_id = pick_fallback_active(&self.panels.tabs, removed.order);
        }
        self.save_panels(cx);
        cx.notify();
    }

    /// Switch the active panel tab and persist. No-op if `tab_id` is
    /// not present (e.g. the tab was just deleted on another window).
    /// Deactivates the built-in Input panel if it was active.
    pub(in crate::workspace) fn set_active_panel_tab(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if !self.panels.tabs.iter().any(|t| t.id == tab_id) {
            return;
        }
        let already_active =
            !self.terminal_input_visible && self.panels.active_tab_id.as_ref() == Some(&tab_id);
        if already_active {
            return;
        }
        self.terminal_input_visible = false;
        self.panels.active_tab_id = Some(tab_id);
        self.save_panels(cx);
        self.bottom_dock.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Switch the bottom dock to the built-in "Input" panel.
    pub(in crate::workspace) fn activate_bottom_input(&mut self, cx: &mut Context<Self>) {
        if self.terminal_input_visible {
            return;
        }
        self.terminal_input_visible = true;
        self.bottom_dock.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Send the current terminal input text to the focused pane.
    /// Appends `\r` so the shell receives the command as if Enter was
    /// pressed, then clears the input field.
    ///
    /// `gpui_component::Input`'s multi-line `enter` action inserts a
    /// `\n` into the textarea *before* it emits the `PressEnter`
    /// event (see `gpui_component/src/input/state.rs::enter`), so the
    /// raw `state.value()` always has at least one trailing newline
    /// when Cmd+Enter triggers this path. Trimming trailing `\r`/`\n`
    /// before the embedded-newline → `\r` conversion is what keeps
    /// the shell from receiving two (or three, after `ICRNL` echo)
    /// Enter keystrokes.
    pub(in crate::workspace) fn send_terminal_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = self.terminal_input.read(cx).value().to_string();
        let trimmed = raw.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return;
        }
        // Convert embedded newlines to CR so each line is treated as a
        // separate command by the shell's line discipline; then a
        // single trailing `\r` submits the final line.
        let mut payload: String = trimmed
            .chars()
            .map(|c| if c == '\n' { '\r' } else { c })
            .collect();
        payload.push('\r');
        self.send_to_focused_pane(payload.as_bytes(), cx);
        self.terminal_input
            .update(cx, |s, cx_state| s.set_value("", window, cx_state));
    }

    /// Click handler for a widget — dispatches based on the widget
    /// kind. Today only `Button` is implemented; unknown / future
    /// types are silent no-ops (the JSON survives via
    /// `MacroKey::Unknown` round-trip; the click simply does nothing
    /// until a daruda version that understands the type loads it).
    pub(in crate::workspace) fn run_widget(
        &mut self,
        tab_id: TabId,
        widget_id: WidgetId,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.panels.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(widget) = tab
            .widgets
            .iter()
            .find(|w| w.id() == Some(widget_id.as_str()))
        else {
            return;
        };
        match widget {
            MacroKey::Button(btn) => {
                let payload = button_payload(btn);
                self.send_to_focused_pane(payload.as_bytes(), cx);
            }
            MacroKey::Unknown(_) => {
                // Forward-compat: a widget type defined by a newer
                // daruda — we round-trip the JSON but cannot dispatch.
            }
        }
    }

    fn send_to_focused_pane(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let focused_id = self.main_area.focused_pane_id;
        let Some(view) = self
            .main_area
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.terminal_view().cloned())
        else {
            return;
        };
        view.update(cx, |view, _| view.send_input(bytes));
        self.bump_activity(focused_id);
        cx.notify();
    }

    /// Reload panels from disk. Compares structurally (PanelsState:
    /// PartialEq) to suppress no-op events — both daruda's own
    /// atomic-rename saves (disk content matches what's already in
    /// memory) and external editor touches that didn't actually
    /// change anything skip the notify, so the watcher can't loop on
    /// itself.
    ///
    /// Re-registers macro shortcuts on every reload so a freshly
    /// added (or renamed) shortcut takes effect without restart.
    pub fn apply_panels_reload(&mut self, cx: &mut Context<Self>) {
        let Some(mut reloaded) = daruda_store::panels::load_panels_in(&self.data_dir) else {
            return;
        };
        if daruda_store::panels::migrate_builtin_flags(&mut reloaded)
            && let Err(e) = daruda_store::panels::save_panels_in(&self.data_dir, &reloaded)
        {
            let report = ErrorReport::new("Failed to save panels.json")
                .severity(ErrorSeverity::Error)
                .from_error(&e)
                .at(file!(), line!())
                .with_context(
                    "path",
                    daruda_store::observability::system_info::redact_home(
                        daruda_store::panels::panels_path_in(&self.data_dir),
                    ),
                )
                .with_context("phase", "reload.migrate")
                .dedup("panels.save")
                .build();
            self.report_error(report, cx);
        }
        if reloaded == self.panels {
            return;
        }
        self.panels = reloaded;
        register_macro_shortcuts(&self.panels, cx);
        cx.notify();
    }

    /// Dispatch a macro click via its keyboard shortcut. The keymap
    /// system fires `RunMacroByShortcut(<shortcut>)`; we look the
    /// shortcut back up against the current panels state and dispatch
    /// `run_widget`. Stale bindings whose macro no longer exists land
    /// here harmlessly (no-op).
    ///
    /// Suppressed while any modal is open — including the
    /// MacroEditModal in recording mode (so the user-pressed
    /// keystroke can be captured as the new shortcut instead of
    /// firing the *previous* binding for the same combo). Also
    /// avoids surprise macro fires while the user is interacting
    /// with any other modal (Rename, Delete, etc.).
    pub(in crate::workspace) fn on_run_macro_by_shortcut(
        &mut self,
        action: &RunMacroByShortcut,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // Skip when any Dialog is open — the user is editing a modal,
        // they probably didn't intend the macro shortcut.
        use crate::ui::WindowExt as _;
        if window.has_active_dialog(cx) {
            return;
        }
        let Some((tab_id, widget_id)) = find_widget_by_shortcut(&self.panels, action.0.as_ref())
        else {
            return;
        };
        self.run_widget(tab_id, widget_id, cx);
    }
}

#[cfg(test)]
mod tests;
