//! Welcome screen — shown on first launch or when no saved session.
//!
//! Displays the daruda logo, "Open Folder" button, recent projects
//! list, and "New Empty Window" button.

use crate::ui::theme;
use crate::window_registry::WindowRegistry;
use gpui::{
    App, Context, FocusHandle, IntoElement, MouseButton, Render, SharedString, Window, actions,
    div, prelude::*, px,
};

use crate::surface::strings as s;

actions!(welcome, [OpenFolder, NewEmptyWindow]);

/// Events emitted by the welcome screen.
pub enum WelcomeEvent {
    OpenFolder,
    OpenProject(std::path::PathBuf),
    NewEmpty,
}

impl gpui::EventEmitter<WelcomeEvent> for WelcomeScreen {}

/// Welcome screen entity.
pub struct WelcomeScreen {
    focus_handle: FocusHandle,
    recent: Vec<daruda_store::project::RecentEntry>,
}

impl WelcomeScreen {
    pub fn new(
        recent: Vec<daruda_store::project::RecentEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        WindowRegistry::register_welcome(window.window_handle(), weak, cx);
        cx.on_release(move |_: &mut WelcomeScreen, cx: &mut App| {
            WindowRegistry::clear_welcome(cx);
        })
        .detach();
        Self {
            focus_handle: cx.focus_handle(),
            recent,
        }
    }
}

impl Render for WelcomeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_recent = !self.recent.is_empty();

        let t = theme::current(cx);
        let welcome_text = t.welcome_text;
        let faint_text = t.faint_text;
        let muted_text = t.muted_text;
        let button_bg = t.welcome_button_bg;
        let button_border = t.welcome_button_border;
        let button_hover_bg = t.welcome_button_hover_bg;
        let recent_hover_bg = t.welcome_recent_hover_bg;
        let panel_bg = t.welcome_bg;

        let title = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(theme::WELCOME_GAP_TIGHT))
            .child(
                div()
                    .text_size(px(theme::WELCOME_TITLE_FONT_SIZE))
                    .text_color(welcome_text)
                    .child(s::WELCOME_TITLE),
            )
            .child(
                div()
                    .text_size(px(theme::WELCOME_VERSION_FONT_SIZE))
                    .text_color(faint_text)
                    .child(s::WELCOME_VERSION),
            );

        let open_folder_btn = div()
            .id("open-folder")
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .px(px(theme::WELCOME_BUTTON_PAD_X))
            .py(px(theme::WELCOME_BUTTON_PAD_Y))
            .bg(button_bg)
            .border_1()
            .border_color(button_border)
            .rounded(px(theme::WELCOME_BUTTON_RADIUS))
            .text_size(px(theme::WELCOME_BUTTON_FONT_SIZE))
            .text_color(welcome_text)
            .cursor_pointer()
            .hover(move |d| d.bg(button_hover_bg))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _, _window, cx| {
                    cx.emit(WelcomeEvent::OpenFolder);
                }),
            )
            .child(s::WELCOME_OPEN_FOLDER);

        let recent_section = if has_recent {
            let entries = self
                .recent
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let name = SharedString::from(entry.name.clone());
                    let path_display = SharedString::from(
                        entry
                            .root
                            .parent()
                            .and_then(|p| {
                                // Abbreviate home dir to ~
                                let home = dirs::home_dir()?;
                                p.strip_prefix(&home)
                                    .ok()
                                    .map(|rel| format!("~/{}", rel.display()))
                                    .or_else(|| Some(p.display().to_string()))
                            })
                            .unwrap_or_default(),
                    );

                    div()
                        .id(("recent", i))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::WELCOME_GAP_LOOSE))
                        .w_full()
                        .px(px(theme::WELCOME_RECENT_PAD_X))
                        .py(px(theme::WELCOME_RECENT_PAD_Y))
                        .rounded(px(theme::WELCOME_RECENT_RADIUS))
                        .cursor_pointer()
                        .hover(move |d| d.bg(recent_hover_bg))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _window, cx| {
                                if let Some(entry) = this.recent.get(i) {
                                    cx.emit(WelcomeEvent::OpenProject(entry.root.clone()));
                                }
                            }),
                        )
                        .child(
                            div()
                                .text_size(px(theme::WELCOME_RECENT_FONT_SIZE))
                                .text_color(welcome_text)
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(px(theme::WELCOME_RECENT_FONT_SIZE))
                                .text_color(faint_text)
                                .child(path_display),
                        )
                })
                .collect::<Vec<_>>();

            div()
                .flex()
                .flex_col()
                .gap(px(theme::WELCOME_GAP_TIGHT))
                .w_full()
                .child(
                    div()
                        .text_size(px(theme::WELCOME_HEADING_FONT_SIZE))
                        .text_color(muted_text)
                        .child(s::WELCOME_RECENT),
                )
                .children(entries)
        } else {
            div()
                .text_size(px(theme::WELCOME_HEADING_FONT_SIZE))
                .text_color(faint_text)
                .child(s::WELCOME_NO_RECENT)
        };

        let new_empty_btn = div()
            .id("new-empty")
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .px(px(theme::WELCOME_BUTTON_PAD_X))
            .py(px(theme::WELCOME_BUTTON_PAD_Y))
            .rounded(px(theme::WELCOME_BUTTON_RADIUS))
            .text_size(px(theme::WELCOME_BUTTON_FONT_SIZE))
            .text_color(muted_text)
            .cursor_pointer()
            .hover(move |d| d.bg(button_hover_bg).text_color(welcome_text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _, _window, cx| {
                    cx.emit(WelcomeEvent::NewEmpty);
                }),
            )
            .child(s::WELCOME_NEW_EMPTY);

        let changelog = div()
            .text_size(px(theme::WELCOME_VERSION_FONT_SIZE))
            .text_color(faint_text)
            .child(s::WELCOME_CHANGELOG_OPEN_POLICY);

        // Main layout — centered panel.
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(panel_bg)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(theme::WELCOME_GAP))
                    .w(px(theme::WELCOME_PANEL_WIDTH))
                    .p(px(theme::WELCOME_PANEL_PAD))
                    .child(title)
                    .child(open_folder_btn)
                    .child(recent_section)
                    .child(new_empty_btn)
                    .child(changelog),
            )
    }
}
