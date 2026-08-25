//! The Flows panel's first section: the flow files this lane can run.
//!
//! A row draws its flow's graph on click, runs it from the ▶, and offers
//! rename and delete on right-click. The dialogs those two open live with
//! the file operations they invoke, not here.

use gpui::{IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings;
use crate::ui::ContextMenuExt as _;
use crate::ui::Disableable as _;
use crate::ui::theme;
use crate::workspace::layout::RightDockSnapshot;

/// Same glyph the graph pane's toolbar runs from — one affordance, two places
/// it is reachable. `IconName` has no play arrow (see `flow_graph_pane`).
const ICON_PLAY: &str = "icons/ui/play-arrow.svg";

/// `[+]` in the section header. A flow made here lands in this project's own
/// directory under the app home, not in the working tree — see
/// `Workspace::create_flow`.
pub(super) fn new_flow_button(snap: &RightDockSnapshot) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    crate::ui::button_bare("flow-new")
        .icon(crate::ui::IconName::Plus)
        .tooltip(strings::flow_new_tooltip())
        .on_click(move |_, window, cx| {
            let Some(ws) = workspace.upgrade() else {
                return;
            };
            let weak = ws.downgrade();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                weak,
                strings::flow_new_title(),
                strings::flow_new_placeholder(),
                None,
                move |ws, value, window, cx| {
                    let Some(name) = value else {
                        return;
                    };
                    ws.create_flow(&name, window, cx);
                },
                window,
                cx,
            );
        })
}

/// Rename and delete, on the row they act on. Both are file operations — the
/// contents are S4's business, and neither touches the run history, which
/// records a resolved spec rather than the file it came from.
///
/// Origin does not gate either one. A flow under `.daruda/flows/` is committed
/// *in order to* be authored there, so making it read-only would lock the one
/// place a shared flow lives; what origin changes is what the dialog says, not
/// what it is allowed to do.
fn flow_row_menu(
    path: std::path::PathBuf,
    name: String,
    origin: crate::workspace::flow_paths::FlowOrigin,
    ws: gpui::WeakEntity<crate::workspace::Workspace>,
) -> Vec<crate::ui::PopupMenuItem> {
    use crate::workspace::render::ws_popup_menu_item;

    let rename_path = path.clone();
    let rename_from = name.clone();
    let rename = ws_popup_menu_item(
        ws.clone(),
        strings::flow_row_menu_rename(),
        false,
        move |_, window, cx| {
            let weak = cx.entity().downgrade();
            let path = rename_path.clone();
            let initial = rename_from.clone();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                weak,
                strings::flow_rename_title(),
                strings::flow_new_placeholder(),
                Some(&initial),
                move |ws, value, _window, cx| {
                    let Some(to) = value else {
                        return;
                    };
                    ws.rename_flow(&path, &to, cx);
                },
                window,
                cx,
            );
        },
    );

    let delete_path = path;
    let delete_name = name;
    let delete = ws_popup_menu_item(
        ws,
        strings::flow_row_menu_delete(),
        false,
        move |_, window, cx| {
            let weak = cx.entity().downgrade();
            crate::workspace::flow_file_ops::ask_before_deleting(
                delete_path.clone(),
                &delete_name,
                origin,
                weak,
                window,
                cx,
            );
        },
    );
    vec![rename, delete]
}

/// The ▶ on a row: run this flow without going through the picker's list.
///
/// Off while an open graph pane of this flow holds unsaved edits, for the reason
/// the toolbar's is: a run reads the file. The panel cannot see a pane's form,
/// so the answer arrives in the snapshot (`flows_with_unsaved_edits`).
///
/// The row is clickable too — it opens the graph — so the press has to stop
/// here. A press and not `occlude()`: a click starts only while the element's
/// own hitbox reads as hovered, and gpui runs bubble listeners in reverse
/// registration order, so this wrapper (painted after the row) gets the
/// mouse-down first and the row never arms its click. `occlude()` would do it
/// too, by truncating the hit test — but the same truncation is what feeds
/// `hover`, so the row's highlight would drop out the moment the pointer
/// reached this button. Right-click is deliberately let through: the row's own
/// rename/delete menu is the right answer there.
fn run_button(
    found: &crate::workspace::flow_paths::FoundFlow,
    snap: &RightDockSnapshot,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let path = found.path.clone();
    let unsaved = snap.flows_with_unsaved_edits.contains(&found.path);
    let id = SharedString::from(format!("flow-run-{}", found.path.display()));
    let selector = id.to_string();
    div()
        .flex_none()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // What this wrapper is for cannot be seen without a real hit test, so
        // the test that presses it needs to find it.
        .debug_selector(move || selector)
        .child(
            crate::ui::button_bare(id)
                .icon(crate::ui::Icon::empty().path(ICON_PLAY))
                .tooltip(if unsaved {
                    strings::flow_needs_save()
                } else {
                    strings::flow_run_tooltip()
                })
                .disabled(unsaved)
                .on_click(move |_, window, cx| {
                    let path = path.clone();
                    match workspace.update(cx, |ws, cx| {
                        ws.run_flow_at(
                            &path,
                            crate::workspace::command::flow_picker::FlowPurpose::Run,
                            // The whole flow: this row names a file, not a
                            // node, and a pin is a thing you can only see on
                            // the graph.
                            crate::workspace::flow_request::FlowSelection::default(),
                            window,
                            cx,
                        )
                    }) {
                        Ok(()) => {}
                        Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                            daruda_store::observability::error_report::ErrorReport::new(
                                "Flows panel: workspace gone while running a flow",
                            )
                            .severity(
                                daruda_store::observability::error_report::ErrorSeverity::Warning,
                            )
                            .at(file!(), line!())
                            .with_context("error", format!("{e}"))
                            .dedup("right_dock.flow.run")
                            .build(),
                        ),
                    }
                }),
        )
}

/// One flow file: its name, where it came from, and a click that draws it.
///
/// The origin is not decoration — the repository's `.daruda/flows/` and the
/// person's own folder can hold the same name, and a row that did not say
/// which would open the other one without a word.
pub(super) fn flow_row(
    found: &crate::workspace::flow_paths::FoundFlow,
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();
    let ws_for_menu = snap.workspace.clone();
    let path = found.path.clone();
    let menu_path = found.path.clone();
    let name = crate::workspace::flow_paths::flow_label(&found.path);
    let menu_name = name.clone();
    let menu_origin = found.origin;
    let origin = crate::workspace::flow_paths::origin_label(found.origin);
    let row_hover_bg = t.skill_row_hover_bg;
    div()
        .id(SharedString::from(format!(
            "flow-file-{}",
            found.path.display()
        )))
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .px(px(theme::SKILL_ROW_PAD_X))
        .py(px(theme::SKILL_ROW_PAD_Y))
        .rounded(px(theme::SKILL_ROW_RADIUS))
        .cursor_pointer()
        .hover(move |s| s.bg(row_hover_bg))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.text_body)
                .child(name),
        )
        .child(
            div()
                .flex_shrink()
                .min_w_0()
                .truncate()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(t.text_subtle)
                .child(origin),
        )
        .child(run_button(found, snap))
        .on_click(move |_, window, cx| {
            let path = path.clone();
            match workspace.update(cx, |ws, cx| ws.open_flow_graph(&path, window, cx)) {
                Ok(()) => {}
                Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                    daruda_store::observability::error_report::ErrorReport::new(
                        "Flows panel: workspace gone while opening a flow graph",
                    )
                    .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup("right_dock.flow.graph")
                    .build(),
                ),
            }
        })
        // `.context_menu()` returns a wrapper that only implements
        // `ParentElement`/`Styled`, so it has to come after every
        // `Stateful`/`InteractiveElement` call above.
        .context_menu(crate::ui::menu_builder(move |menu, _window, _cx| {
            flow_row_menu(
                menu_path.clone(),
                menu_name.clone(),
                menu_origin,
                ws_for_menu.clone(),
            )
            .into_iter()
            .fold(menu, |m, item| m.item(item))
        }))
}
