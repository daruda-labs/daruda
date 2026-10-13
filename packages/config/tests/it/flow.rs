//! Public API tests for flow.

use daruda_config::flow::*;

/// A user who configures nothing still gets the two defences that can
/// always be enforced — that is what makes them a last line of defence
/// — and no cost ceiling, because a default one would be denominated
/// in a guess and the engine warns on every run whose agent reports a
/// different currency.
#[test]
fn the_enforceable_ceilings_are_on_by_default_and_the_guessable_one_is_not() {
    let cfg = FlowConfig::default();
    assert_eq!(cfg.timeout().unwrap().as_secs(), 90 * 60);
    assert_eq!(cfg.max_node_runs(), Some(DEFAULT_MAX_NODE_RUNS));
    assert!(cfg.cost_limit().is_none());
}

/// Each axis turns off on its own, and setting one leaves the others at
/// their defaults.
#[test]
fn zero_disables_one_axis_without_touching_the_rest() {
    let cfg: FlowConfig = toml::from_str("timeout_minutes = 0").expect("parse");
    assert!(cfg.timeout().is_none());
    assert_eq!(cfg.max_node_runs(), Some(DEFAULT_MAX_NODE_RUNS));
}

/// The amount and the currency are read as one value, so a limit can
/// never reach the engine without the unit it is measured in.
#[test]
fn a_cost_limit_carries_its_currency() {
    let cfg: FlowConfig = toml::from_str("max_cost = 2.5\ncost_currency = \"EUR\"").expect("parse");
    assert_eq!(cfg.cost_limit(), Some((2.5, "EUR".to_owned())));
}
