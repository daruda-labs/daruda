//! Unpairing and token removal reach outside daruda, so each asks first and
//! changes nothing until the dialog's OK.

use super::*;
use daruda_config::remote::RemoteRecipient;
use gpui::{BorrowAppContext as _, TestAppContext, VisualTestContext, WindowHandle};

const SLACK: &str = "slack-main";

#[test]
fn credential_accounts_are_distinct() {
    assert_ne!(Secret::Bot.account(), Secret::App.account());
}

fn paired_slack() -> daruda_config::Config {
    let mut channel = ChannelConfig::new(SLACK.to_owned(), ChannelKind::Slack);
    channel.recipient = Some(RemoteRecipient {
        user_id: "U1".to_owned(),
        conversation_id: "D1".to_owned(),
        scope_id: "T1".to_owned(),
    });
    let mut config = daruda_config::Config::default();
    config.remote.channels = vec![channel];
    config
}

/// The workspace draws the dialog layer, so a bare view never shows one.
struct Host {
    view: Entity<ChannelSettings>,
}

impl gpui::Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        gpui::div()
            .size_full()
            .child(self.view.clone())
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}

fn build(cx: &mut TestAppContext) -> (WindowHandle<gpui_component::Root>, Entity<ChannelSettings>) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|cx| {
        SettingsStore::init(cx);
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_for_testing(paired_slack()));
    });
    let slot = std::cell::RefCell::new(None);
    let wh = cx.add_window(|window, cx| {
        let view = cx.new(|cx| ChannelSettings::new(window, cx));
        *slot.borrow_mut() = Some(view.clone());
        let host = cx.new(|_| Host { view });
        gpui_component::Root::new(host, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    (wh, view)
}

fn recipient(cx: &mut TestAppContext) -> Option<RemoteRecipient> {
    cx.read(|cx| {
        SettingsStore::global(cx)
            .user()
            .remote
            .channels
            .iter()
            .find(|c| c.id == SLACK)
            .and_then(|c| c.recipient.clone())
    })
}

/// Run `request` in the window, then report whether a dialog is up.
fn opens_a_dialog(
    cx: &mut TestAppContext,
    wh: WindowHandle<gpui_component::Root>,
    view: &Entity<ChannelSettings>,
    request: impl FnOnce(&mut ChannelSettings, &mut Window, &mut Context<ChannelSettings>),
) -> bool {
    let view = view.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        view.update(cx, |view, cx| request(view, window, cx));
        crate::ui::WindowExt::has_active_dialog(window, cx)
    })
    .unwrap()
}

#[gpui::test]
fn unpairing_waits_for_the_dialog_then_forgets_the_chat(cx: &mut TestAppContext) {
    let (wh, view) = build(cx);
    assert!(opens_a_dialog(cx, wh, &view, |v, window, cx| {
        v.request_unpair(SLACK, window, cx)
    }));
    assert!(recipient(cx).is_some(), "nothing changes before OK");

    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(recipient(&mut vcx).is_none());
}

/// OK here deletes a real credential-store entry, so only the ask is driven.
#[gpui::test]
fn removing_a_token_waits_for_the_dialog(cx: &mut TestAppContext) {
    let (wh, view) = build(cx);
    assert!(opens_a_dialog(cx, wh, &view, |v, window, cx| {
        v.request_remove_secret(SLACK, Secret::Bot, window, cx)
    }));
    let busy = view.read_with(cx, |v, _| {
        v.rows
            .iter()
            .find(|r| r.config.id == SLACK)
            .is_some_and(|r| r.bot.busy)
    });
    assert!(!busy, "no credential-store work starts before OK");
}
