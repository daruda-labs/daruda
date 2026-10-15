use super::*;
use crate::telegram::global::{bot::Ownership, install_for_test};
use futures::FutureExt as _;

#[gpui::test]
fn queued_messages_keep_their_owner_and_recipient_without_retargeting(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        SettingsStore::init(cx);
        let mut config = daruda_config::Config::default();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config.clone());
        let _outbound = install_for_test(true, Some(42), cx);
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        cx.global_mut::<TelegramBridge>().outbound_tx = sender;
        cx.global::<TelegramBridge>()
            .send_notice("normal".into(), cx);
        let normal = receiver.next().now_or_never().flatten().unwrap();
        assert!(prepare(normal, cx).is_some());
        cx.global::<TelegramBridge>()
            .send_notice("old recipient".into(), cx);
        let pending = receiver.next().now_or_never().flatten().unwrap();
        config.telegram.authorized_chat_id = Some(99);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config.clone());
        assert!(prepare(pending, cx).is_none());
        cx.global::<TelegramBridge>()
            .send_notice("new recipient before next poll".into(), cx);
        let pending = receiver.next().now_or_never().flatten().unwrap();
        assert_eq!(pending.chat_id, 99);
        assert!(prepare(pending, cx).is_some());
        config.telegram.authorized_chat_id = Some(42);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        cx.global::<TelegramBridge>()
            .send_notice("old bot".into(), cx);
        let pending = receiver.next().now_or_never().flatten().unwrap();
        let original = cx.global::<TelegramBridge>().bot.ready().unwrap().clone();
        cx.global_mut::<TelegramBridge>().bot = Ownership::Ready(Arc::new(ReadyBot {
            token: original.token.clone(),
            inbox: original.inbox.clone(),
            generation: uuid::Uuid::new_v4(),
        }));
        assert!(prepare(pending, cx).is_none());
    });
}
