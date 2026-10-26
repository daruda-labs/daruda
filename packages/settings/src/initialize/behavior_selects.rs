use super::*;

pub(super) struct BehaviorSelects {
    pub(super) keep_awake_mode_select: Entity<SelectState>,
    pub(super) cursor_style_select: Entity<SelectState>,
}

impl BehaviorSelects {
    pub(super) fn new(config: &Config, window: &mut Window, cx: &mut gpui::App) -> Self {
        let cursor_style_str: SharedString = match config.cursor.style {
            daruda_config::CursorStyle::Block => "block".into(),
            daruda_config::CursorStyle::Underline => "underline".into(),
            daruda_config::CursorStyle::Bar => "bar".into(),
        };
        let keep_awake_str: SharedString =
            crate::spec::keep_awake_option(config.desktop.keep_awake).into();
        let keep_awake_mode_select = cx.new(|cx| {
            select::state_with_options(
                [
                    daruda_config::KeepAwakeMode::Off,
                    daruda_config::KeepAwakeMode::Agent,
                    daruda_config::KeepAwakeMode::On,
                ]
                .into_iter()
                .map(|mode| {
                    SelectOption::new(
                        crate::spec::keep_awake_option(mode),
                        crate::labels::keep_awake_mode_label(mode),
                    )
                })
                .collect(),
                Some(&keep_awake_str),
                window,
                cx,
            )
        });
        let cursor_style_select = cx.new(|cx| {
            select::state_with_options(
                vec![
                    SelectOption::new("block", s::settings::cursor_block()),
                    SelectOption::new("underline", s::settings::cursor_underline()),
                    SelectOption::new("bar", s::settings::cursor_bar()),
                ],
                Some(&cursor_style_str),
                window,
                cx,
            )
        });

        Self {
            keep_awake_mode_select,
            cursor_style_select,
        }
    }
}

#[cfg(test)]
mod tests;
