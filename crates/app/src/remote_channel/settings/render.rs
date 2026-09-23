use super::{ChannelSettings, Secret, SecretInput};
use crate::remote_channel::runtime::Status;
use crate::{
    surface::strings as s,
    ui::{self, Disableable, IconName, theme},
};
use daruda_config::remote::ChannelKind;
use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px};

fn status_label(status: Status) -> String {
    match status {
        Status::Disabled => s::remote_disabled(),
        Status::MissingCredentials => s::remote_missing_credentials(),
        Status::Connecting => s::remote_connecting(),
        Status::Connected => s::remote_connected(),
        Status::Retrying => s::remote_retrying(),
        Status::Failed => s::remote_failed(),
        Status::HeldElsewhere => s::remote_held_elsewhere(),
    }
}

impl ChannelSettings {
    fn token_row(
        &self,
        id: &str,
        index: usize,
        secret: Secret,
        input: &SecretInput,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let save_id = id.to_owned();
        let remove_id = id.to_owned();
        let label = match secret {
            Secret::Bot => s::remote_bot_token(),
            Secret::App => s::remote_app_token(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(ui::field_row(
                label,
                ui::input(&input.input, cx, index as isize),
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .child(
                        ui::button(("remote-save", index), s::remote_save_token())
                            .icon(IconName::Check)
                            .disabled(input.busy)
                            .tab_stop(true)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.store_secret(&save_id, secret, false, window, cx)
                            })),
                    )
                    .when(input.configured, |row| {
                        row.child(
                            div()
                                .text_color(theme::current(cx).text_muted)
                                .child(s::remote_token_saved()),
                        )
                        .child(
                            ui::button_icon_danger(("remote-remove", index), ui::icons::DELETE, cx)
                                .disabled(input.busy)
                                .tooltip(s::remote_clear_token())
                                .tab_stop(true)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.store_secret(&remove_id, secret, true, window, cx)
                                })),
                        )
                    }),
            )
    }
}

impl Render for ChannelSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(theme::current(cx).text_primary);
        for (index, row) in self.rows.iter().enumerate() {
            let enabled_id = row.config.id.clone();
            let presence_id = row.config.id.clone();
            let next_enabled = !row.config.enabled;
            let next_presence = !row.config.only_when_away;
            let pair_id = row.config.id.clone();
            let unpair_id = row.config.id.clone();
            let copy_id = row.config.id.clone();
            let name = match row.config.kind {
                ChannelKind::Slack => s::remote_slack(),
                ChannelKind::Discord => s::remote_discord(),
            };
            let ready = row.bot.configured && row.app.as_ref().is_none_or(|app| app.configured);
            let mut section = div()
                .id(("remote-channel", index))
                .w_full()
                .flex()
                .flex_col()
                .gap(px(theme::MODAL_PANEL_GAP))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_between()
                        .items_center()
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(name))
                        .child(
                            div()
                                .text_color(theme::current(cx).text_muted)
                                .child(status_label(row.status)),
                        ),
                )
                .child(
                    div()
                        .text_color(theme::current(cx).text_muted)
                        .child(row.config.id.clone()),
                )
                .child(ui::field_row(
                    s::remote_enabled(),
                    ui::switch(("remote-enabled", index), row.config.enabled, cx)
                        .tooltip(s::remote_enabled())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_enabled(&enabled_id, next_enabled, cx)
                        })),
                ))
                .child(ui::field_row(
                    s::remote_only_when_away(),
                    ui::switch(("remote-presence", index), row.config.only_when_away, cx)
                        .tooltip(s::remote_only_when_away())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_presence(&presence_id, next_presence, cx)
                        })),
                ))
                .child(self.token_row(&row.config.id, index * 2, Secret::Bot, &row.bot, cx));
            if let Some(app) = &row.app {
                section = section.child(self.token_row(
                    &row.config.id,
                    index * 2 + 1,
                    Secret::App,
                    app,
                    cx,
                ));
            }
            if let Some(recipient) = &row.config.recipient {
                section = section.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(theme::MODAL_FOOTER_GAP))
                        .child(s::remote_paired(
                            &recipient.user_id,
                            &recipient.conversation_id,
                        ))
                        .child(
                            ui::button(("remote-unpair", index), s::remote_unpair())
                                .child(ui::icons::icon(ui::icons::CLOSE))
                                .tab_stop(true)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.unpair(&unpair_id, cx)),
                                ),
                        ),
                );
            } else {
                section = section.child(
                    ui::button(("remote-pair", index), s::remote_pair())
                        .child(ui::icons::icon(ui::icons::ADD))
                        .disabled(!row.config.enabled || !ready)
                        .tab_stop(true)
                        .on_click(cx.listener(move |this, _, _, cx| this.pair(&pair_id, cx))),
                );
            }
            if let Some(code) = &row.pair_code {
                section = section.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::MODAL_FOOTER_GAP))
                        .child(format!("!pair {code}"))
                        .child(
                            ui::button_icon(("remote-copy-pair", index), ui::icons::COPY, cx)
                                .tooltip(s::remote_copy_pair())
                                .tab_stop(true)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.copy_pair(&copy_id, cx)),
                                ),
                        ),
                );
            }
            body = body.child(section);
        }
        body.when_some(self.error.clone(), |body, error| {
            body.child(ui::alert::error("remote-settings-error", error))
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn statuses_have_distinct_labels() {
        use super::Status;
        let labels = [
            Status::Disabled,
            Status::MissingCredentials,
            Status::Connecting,
            Status::Connected,
            Status::Retrying,
            Status::HeldElsewhere,
        ]
        .map(super::status_label);
        assert_eq!(
            labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            labels.len()
        );
    }
}
