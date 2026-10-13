//! Public API tests for orchestrator.

use daruda_config::orchestrator::*;
use daruda_store::accounts::AccountId;

#[test]
fn defaults_are_inert() {
    let cfg = OrchestratorConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(cfg.agent_id, None);
    assert_eq!(cfg.account_id, None);
}

#[test]
fn toml_round_trip_preserves_every_field() {
    let id = AccountId::new();
    let cfg = OrchestratorConfig {
        enabled: true,
        agent_id: Some("claude-acp".to_owned()),
        account_id: Some(id),
    };
    let serialized = toml::to_string(&cfg).expect("serialize");
    let back: OrchestratorConfig = toml::from_str(&serialized).expect("deserialize");
    assert_eq!(back, cfg);
}

#[test]
fn unspecified_fields_fall_back_to_defaults() {
    let cfg: OrchestratorConfig = toml::from_str("enabled = true\n").expect("deserialize");
    assert!(cfg.enabled);
    assert_eq!(cfg.agent_id, None);
    assert_eq!(cfg.account_id, None);
}
