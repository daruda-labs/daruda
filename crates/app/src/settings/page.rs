//! Draw a page from [`super::layout`]: its cards and rows in table order,
//! with the hand-drawn blocks the table names.

use gpui::{AnyElement, Div, Focusable as _, IntoElement, ParentElement as _};

use super::layout::{self, Card, CustomCard, CustomRow, Row};
use super::presentation::{card, card_content, config_only_row, page_stack};
use super::search::Target;
use super::{SettingsEvent, SettingsView, TextSetting};
use crate::surface::strings as s;
use daruda_config::BuiltinSection;

impl SettingsView {
    /// `section`'s page, or `None` when the page is drawn wholly by hand.
    pub(super) fn render_layout_page(
        &self,
        section: BuiltinSection,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let cards = layout::page(section)?;
        let mut body = page_stack();
        for spec in cards {
            body = body.child(self.render_layout_card(section, spec, cx));
        }
        Some(body.into_any_element())
    }

    /// The first text input `section` shows, in page order. A folded Advanced
    /// card's inputs are not on screen, so they are skipped.
    pub(super) fn first_visible_input(
        &self,
        section: BuiltinSection,
        cx: &gpui::App,
    ) -> Option<gpui::FocusHandle> {
        let cards = layout::page(section)?;
        cards
            .iter()
            .filter(|card| {
                !matches!(card, Card::Advanced(_)) || self.advanced_open.contains(&section)
            })
            .flat_map(|card| card.rows().iter())
            .flat_map(|row| match row {
                Row::Setting(target) => std::slice::from_ref(target),
                Row::Under(_, children) => children,
                Row::Custom(_) => &[],
            })
            .find_map(|target| match target {
                Target::Text(t) => Some(
                    (super::spec::text_spec(*t).field)(self)
                        .read(cx)
                        .focus_handle(cx),
                ),
                _ => None,
            })
    }

    fn render_layout_card(
        &self,
        section: BuiltinSection,
        spec: &Card,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        match spec {
            Card::Rows { title, rows } => {
                let mut group = card(title(), cx);
                for row in *rows {
                    group = group.child(self.render_layout_row(row, cx));
                }
                group
            }
            Card::Advanced(rows) => {
                let rows = rows
                    .iter()
                    .map(|row| self.render_layout_row(row, cx))
                    .collect();
                self.advanced_card(section, rows, cx)
            }
            Card::Custom(kind) => self.render_custom_card(*kind, cx),
        }
    }

    fn render_layout_row(&self, row: &Row, cx: &mut gpui::Context<Self>) -> Div {
        match row {
            Row::Setting(target) => self.render_target_row(*target, cx),
            Row::Under(parent, children) => {
                let rows: Vec<Div> = children
                    .iter()
                    .map(|target| self.render_target_row(*target, cx))
                    .collect();
                self.dependent_rows(*parent, rows, cx)
            }
            Row::Custom(kind) => self.render_custom_row(*kind, cx),
        }
    }

    /// The live control for one setting, the same on its page and in search.
    pub(super) fn render_target_row(&self, target: Target, cx: &mut gpui::Context<Self>) -> Div {
        match target {
            Target::Text(TextSetting::ShellProgram) => {
                self.text_row_wide(TextSetting::ShellProgram, cx)
            }
            Target::Text(t) => self.text_row(t, cx),
            Target::Select(v) => self.select_row(v, cx),
            Target::Bool(b) => self.switch_row(b, cx),
            Target::StatusBarItem(item) => self.status_bar_item_row(item, cx),
            Target::Page(section) => self.link_row(
                gpui::ElementId::Name(format!("settings-page-link-{}", section.slug()).into()),
                super::navigation::label(section),
                super::navigation::description(section),
                s::settings_search_open(),
                section,
                cx,
            ),
        }
    }

    fn render_custom_row(&self, kind: CustomRow, cx: &mut gpui::Context<Self>) -> Div {
        match kind {
            CustomRow::CustomColors => config_only_row(
                s::settings_label_custom_colors(),
                s::settings_hint_custom_colors(),
                "colors",
                cx,
            ),
            CustomRow::ProjectShell => self.event_row(
                "settings-open-project-config",
                s::settings_label_project_shell(),
                s::settings_hint_project_shell(),
                s::settings_button_open_project_config(),
                || SettingsEvent::OpenProjectConfig,
                cx,
            ),
            CustomRow::DarudaLink => {
                let state = if self.orchestrator_enabled {
                    s::settings_toggle_on()
                } else {
                    s::settings_toggle_off()
                };
                self.link_row(
                    "settings-daruda-orchestrator-link",
                    s::settings_daruda_link_label(&state),
                    s::settings_daruda_link_hint(),
                    s::settings_daruda_link_button(),
                    BuiltinSection::Orchestrator,
                    cx,
                )
            }
            CustomRow::RemoteLink => self.link_row(
                "settings-orchestrator-remote-link",
                s::settings_nav_remote_control(),
                s::settings_used_by_remote_hint(),
                s::settings_used_by_remote_button(),
                BuiltinSection::RemoteControl,
                cx,
            ),
            CustomRow::TelegramBody => card_content(self.telegram_body(cx)),
        }
    }

    fn render_custom_card(&self, kind: CustomCard, cx: &mut gpui::Context<Self>) -> Div {
        match kind {
            CustomCard::AgentCatalog => card(s::settings_section_agent_catalog(), cx)
                .child(card_content(self.render_agent_catalog(cx))),
            CustomCard::RemoteIntegrations => card(s::settings_group_integrations(), cx)
                .child(card_content(self.remote_channel_settings.clone())),
            CustomCard::AboutVersion => self.about_version(cx),
        }
    }
}
