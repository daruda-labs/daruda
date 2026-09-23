//! Channel settings state and actions, with all keychain work off the UI thread.

use super::{global::RemoteChannels, keychain, runtime::Status};
use crate::{settings_store::SettingsStore, surface::strings as s, ui::InputState};
use daruda_config::remote::{ChannelConfig, ChannelKind};
use gpui::{AppContext, Context, Entity, Window};

mod render;

#[derive(Clone, Copy)]
enum Secret {
    Bot,
    App,
}

impl Secret {
    fn account(self) -> &'static str {
        match self {
            Self::Bot => "bot_token",
            Self::App => "app_token",
        }
    }
}

struct SecretInput {
    input: Entity<InputState>,
    configured: bool,
    busy: bool,
}

struct Row {
    config: ChannelConfig,
    bot: SecretInput,
    app: Option<SecretInput>,
    pair_code: Option<String>,
    status: Status,
}

pub struct ChannelSettings {
    rows: Vec<Row>,
    error: Option<String>,
    /// Held, not dropped: an `observe_global` unsubscribes the moment its
    /// `Subscription` is.
    _subscriptions: [gpui::Subscription; 2],
}

impl ChannelSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut configs = SettingsStore::global(cx).user().remote.channels.clone();
        for kind in [ChannelKind::Slack, ChannelKind::Discord] {
            if !configs.iter().any(|config| config.kind == kind) {
                configs.push(ChannelConfig::new(format!("{}-main", kind.slug()), kind));
            }
        }
        let rows = configs
            .into_iter()
            .map(|config| {
                let bot = Self::secret_input(window, cx);
                let app =
                    (config.kind == ChannelKind::Slack).then(|| Self::secret_input(window, cx));
                let status = RemoteChannels::status(&config.id, cx);
                Row {
                    config,
                    bot,
                    app,
                    pair_code: None,
                    status,
                }
            })
            .collect::<Vec<_>>();
        let probes = rows
            .iter()
            .map(|row| (row.config.id.clone(), row.config.kind))
            .collect::<Vec<_>>();
        cx.spawn(async move |this, cx| {
            let states = cx
                .background_executor()
                .spawn(async move {
                    probes
                        .into_iter()
                        .map(|(id, kind)| {
                            let service = keychain::channel_service(&id);
                            let bot = keychain::read(&service, "bot_token").is_some();
                            let app = kind == ChannelKind::Slack
                                && keychain::read(&service, "app_token").is_some();
                            (id, bot, app)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            // SILENT-OK: a closed settings view no longer needs credential-presence labels.
            let _ = this.update(cx, |this, cx| {
                for (id, bot, app) in states {
                    if let Some(row) = this.rows.iter_mut().find(|row| row.config.id == id) {
                        row.bot.configured = bot;
                        if let Some(input) = &mut row.app {
                            input.configured = app;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        // Config edits and worker status both arrive as global mutations, so
        // the view follows them by subscription rather than polling.
        let subscriptions = [
            cx.observe_global::<SettingsStore>(|this, cx| this.refresh(cx)),
            cx.observe_global::<RemoteChannels>(|this, cx| this.refresh(cx)),
        ];
        Self {
            rows,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn secret_input(window: &mut Window, cx: &mut Context<Self>) -> SecretInput {
        SecretInput {
            input: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(s::remote_token_placeholder())
            }),
            configured: false,
            busy: false,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let config = SettingsStore::global(cx).user_arc();
        let mut changed = false;
        for row in &mut self.rows {
            let next = config
                .remote
                .channels
                .iter()
                .find(|c| c.id == row.config.id)
                .cloned()
                .unwrap_or_else(|| ChannelConfig::new(row.config.id.clone(), row.config.kind));
            let status = RemoteChannels::status(&row.config.id, cx);
            changed |= row.config != next || row.status != status;
            if next.recipient.is_some() {
                row.pair_code = None;
            }
            row.config = next;
            row.status = status;
        }
        if changed {
            cx.notify();
        }
    }

    fn configure(
        &mut self,
        id: &str,
        edit: impl FnOnce(&mut ChannelConfig),
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter().find(|row| row.config.id == id) else {
            return;
        };
        let mut config = SettingsStore::global(cx).user().remote.clone();
        if !config.channels.iter().any(|channel| channel.id == id) {
            config.channels.push(row.config.clone());
        }
        if let Some(channel) = config.channels.iter_mut().find(|channel| channel.id == id) {
            edit(channel);
        }
        let result = config.validate().and_then(|()| {
            cx.global_mut::<SettingsStore>().apply_patch(
                daruda_config::SettingsPatch::RemoteChannels(config.channels),
            )
        });
        match result {
            Ok(()) => {
                self.error = None;
                if cx.has_global::<RemoteChannels>() {
                    super::global::reconcile(cx);
                }
                self.refresh(cx);
            }
            Err(error) => self.report_error(&error, cx),
        }
    }

    fn set_enabled(&mut self, id: &str, value: bool, cx: &mut Context<Self>) {
        self.configure(id, |config| config.enabled = value, cx);
    }

    fn set_presence(&mut self, id: &str, value: bool, cx: &mut Context<Self>) {
        self.configure(id, |config| config.only_when_away = value, cx);
    }

    fn pair(&mut self, id: &str, cx: &mut Context<Self>) {
        if !cx.has_global::<RemoteChannels>() {
            return;
        }
        let code = RemoteChannels::generate_pair_code(id, cx);
        if let Some(row) = self.rows.iter_mut().find(|row| row.config.id == id) {
            row.pair_code = code;
        }
        cx.notify();
    }

    /// Confirm-first entry for [`Self::unpair`]: the paired chat stops
    /// reaching daruda, and pairing again needs a fresh code.
    fn request_unpair(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_owned();
        crate::workspace::dialog_helpers::confirm_destructive(
            cx.weak_entity(),
            s::remote_confirm_unpair_title(),
            s::settings_confirm_unpair_body(),
            s::settings_confirm_ok_unpair(),
            move |this, _window, cx| this.unpair(&id, cx),
            window,
            cx,
        );
    }

    /// Confirm-first entry for removing a stored token, which deletes it
    /// from the system credential store.
    fn request_remove_secret(
        &mut self,
        id: &str,
        secret: Secret,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = id.to_owned();
        crate::workspace::dialog_helpers::confirm_destructive(
            cx.weak_entity(),
            s::remote_confirm_remove_token_title(),
            s::settings_confirm_remove_token_body(),
            s::settings_confirm_ok_remove_token(),
            move |this, window, cx| this.store_secret(&id, secret, true, window, cx),
            window,
            cx,
        );
    }

    fn unpair(&mut self, id: &str, cx: &mut Context<Self>) {
        self.configure(id, |config| config.recipient = None, cx);
    }

    fn copy_pair(&self, id: &str, cx: &mut Context<Self>) {
        if let Some(code) = self
            .rows
            .iter()
            .find(|row| row.config.id == id)
            .and_then(|row| row.pair_code.as_ref())
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(format!("!pair {code}")));
        }
    }

    fn input_mut(&mut self, id: &str, secret: Secret) -> Option<&mut SecretInput> {
        let row = self.rows.iter_mut().find(|row| row.config.id == id)?;
        match secret {
            Secret::Bot => Some(&mut row.bot),
            Secret::App => row.app.as_mut(),
        }
    }

    fn store_secret(
        &mut self,
        id: &str,
        secret: Secret,
        remove: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.input_mut(id, secret) else {
            return;
        };
        if input.busy {
            return;
        }
        let value = input.input.read(cx).value().to_string();
        if !remove && value.trim().is_empty() {
            return;
        }
        input.busy = true;
        let id = id.to_owned();
        let app = cx.to_async();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let service = keychain::channel_service(&id);
            let result = cx
                .background_executor()
                .spawn(async move {
                    if remove {
                        keychain::delete(&service, secret.account())
                    } else {
                        keychain::write(&service, secret.account(), value.trim())
                    }
                })
                .await;
            if result.is_ok() {
                app.update(|cx| RemoteChannels::restart(&id, cx));
            }
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                if let Some(input) = this.input_mut(&id, secret) {
                    input.busy = false;
                    if result.is_ok() {
                        input.configured = !remove;
                        input
                            .input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                }
                match result {
                    Ok(()) => {
                        this.error = None;
                    }
                    Err(error) => this.report_error(&error.to_string(), cx),
                }
                cx.notify();
            }) {
                super::log_error(
                    "Remote credential settings view closed",
                    &error,
                    "remote.settings.closed",
                );
            }
        })
        .detach();
    }

    fn report_error(&mut self, error: &str, cx: &mut Context<Self>) {
        super::log_error(
            "Remote channel settings failed",
            &error,
            "remote.settings.failed",
        );
        self.error = Some(s::remote_error(error));
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
