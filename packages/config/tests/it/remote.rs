//! Public API tests for remote.

use daruda_config::remote::*;

#[test]
fn legacy_config_leaves_new_channels_inert() {
    let cfg: daruda_config::Config =
        toml::from_str("[telegram]\nenabled = true\nauthorized_chat_id = 42\n").unwrap();
    assert!(cfg.telegram.enabled);
    assert!(cfg.remote.channels.is_empty());
}

#[test]
fn round_trip_preserves_opaque_ids_and_explicit_presence_setting() {
    let source = "[[channels]]\nid = 'work'\nkind = 'slack'\nenabled = true\nonly_when_away = false\n[channels.recipient]\nuser_id = 'U1'\nconversation_id = 'D1'\nscope_id = 'T1'\n";
    let cfg: RemoteConfig = toml::from_str(source).unwrap();
    assert!(cfg.validate().is_ok());
    assert!(!cfg.channels[0].only_when_away);
    assert_eq!(
        toml::from_str::<RemoteConfig>(&toml::to_string(&cfg).unwrap()).unwrap(),
        cfg
    );
    let mut wrong = cfg.channels[0].recipient.clone().unwrap();
    wrong.user_id = "U2".into();
    assert!(!cfg.channels[0].accepts(&wrong));
    wrong = cfg.channels[0].recipient.clone().unwrap();
    wrong.scope_id = "T2".into();
    assert!(!cfg.channels[0].accepts(&wrong));
}

#[test]
fn duplicate_ids_and_invalid_credential_keys_are_rejected() {
    let channel = ChannelConfig::new("work".into(), ChannelKind::Slack);
    assert!(
        RemoteConfig {
            channels: vec![channel.clone(), channel]
        }
        .validate()
        .is_err()
    );
    let invalid = ChannelConfig::new("../token".into(), ChannelKind::Discord);
    assert!(
        RemoteConfig {
            channels: vec![invalid]
        }
        .validate()
        .is_err()
    );
}
