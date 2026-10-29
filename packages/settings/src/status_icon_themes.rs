//! Host-provided artwork choices, including unavailable saved selections.
use crate::{SettingsView, strings as s};
use daruda_config::StatusIconTheme;
use daruda_ui::select::SelectOption;
use gpui::{Context, SharedString, Window};

pub(super) fn options(
    themes: &[(String, String)],
    selected: &StatusIconTheme,
) -> Vec<SelectOption> {
    let mut choices = vec![
        SelectOption::new("dots", s::settings::status_icon_theme_dots()),
        SelectOption::new("icons", s::settings::status_icon_theme_icons()),
    ];
    choices.extend(
        themes
            .iter()
            .map(|(id, name)| SelectOption::new(id.clone(), format!("{name} · {id}"))),
    );
    if let StatusIconTheme::Custom(id) = selected
        && !themes.iter().any(|(choice, _)| choice == id)
    {
        choices.push(SelectOption::new(
            id.clone(),
            format!("{} · {id}", s::settings::status_icon_theme_unavailable()),
        ));
    }
    choices
}
impl SettingsView {
    pub fn set_status_icon_themes(
        &mut self,
        themes: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_icon_themes = themes;
        let selected = crate::store::SettingsStore::global(cx)
            .user()
            .left_dock
            .status_icon_theme
            .clone();
        self.refresh_status_icon_themes(&selected, window, cx);
        cx.notify();
    }
    pub(super) fn refresh_status_icon_themes(
        &self,
        selected: &StatusIconTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choices = options(&self.status_icon_themes, selected);
        let value = SharedString::from(selected.as_str().to_owned());
        self.status_icon_theme_select.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_value(&value, window, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_selection_stays_selectable_without_hiding_builtins() {
        let selected = StatusIconTheme::Custom("user/gone".into());
        let unavailable = options(&[], &selected);
        let entry = unavailable
            .iter()
            .find(|o| o.value.as_ref() == "user/gone")
            .unwrap();
        assert!(
            entry
                .label
                .contains(&s::settings::status_icon_theme_unavailable())
        );
        assert_eq!(
            options(&[("user/gone".into(), "Returned".into())], &selected).len(),
            3
        );
    }
}
