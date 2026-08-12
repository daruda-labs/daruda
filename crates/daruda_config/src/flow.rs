//! `[flow]` section — the three runaway defences a flow run starts with.
//!
//! `daruda_flow`'s `Budget` treats each axis as `Option`, where `None` is
//! unlimited; the design calls these "the last line of defence", so the
//! two that can always be enforced are on by default and only an explicit
//! `0` turns one off.
//!
//! The cost ceiling is the exception and defaults to off. A cost is only
//! comparable in a currency, and the currency is whatever the agent
//! happens to report — a default limit in the wrong one never applies and
//! makes the engine warn on every single run, which is worse than having
//! no limit at all. A user who knows what their agent bills in sets both
//! fields together.

use serde::{Deserialize, Serialize};

/// Wall-clock cap for one run. Long enough for a multi-node flow with
/// repairs, short enough that a wedged adapter does not run overnight.
pub const DEFAULT_TIMEOUT_MINUTES: u32 = 90;
/// Cap on runner calls — reruns and fix sessions included. The defence
/// against a repair loop that keeps re-deriving the same nodes.
pub const DEFAULT_MAX_NODE_RUNS: u32 = 100;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FlowConfig {
    /// Minutes before a run is stopped. `0` disables the deadline.
    pub timeout_minutes: u32,
    /// Maximum runner calls in one run. `0` disables the count.
    pub max_node_runs: u32,
    /// Cost ceiling in `cost_currency`. `0` disables it, which is the
    /// default — see the module doc.
    pub max_cost: f64,
    /// The currency `max_cost` is denominated in. Only meaningful when
    /// `max_cost` is non-zero, which is why the two are read together
    /// through [`FlowConfig::cost_limit`] rather than separately.
    pub cost_currency: String,
}

impl Default for FlowConfig {
    fn default() -> Self {
        Self {
            timeout_minutes: DEFAULT_TIMEOUT_MINUTES,
            max_node_runs: DEFAULT_MAX_NODE_RUNS,
            max_cost: 0.0,
            cost_currency: "USD".to_owned(),
        }
    }
}

impl FlowConfig {
    /// The deadline as a duration, or `None` when disabled.
    pub fn timeout(&self) -> Option<std::time::Duration> {
        (self.timeout_minutes > 0)
            .then(|| std::time::Duration::from_secs(u64::from(self.timeout_minutes) * 60))
    }

    pub fn max_node_runs(&self) -> Option<u32> {
        (self.max_node_runs > 0).then_some(self.max_node_runs)
    }

    /// The amount and its currency, or `None` when no ceiling is set. One
    /// value because an amount without a currency does not compare, and a
    /// currency without an amount bounds nothing.
    pub fn cost_limit(&self) -> Option<(f64, String)> {
        (self.max_cost > 0.0).then(|| (self.max_cost, self.cost_currency.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let cfg: FlowConfig =
            toml::from_str("max_cost = 2.5\ncost_currency = \"EUR\"").expect("parse");
        assert_eq!(cfg.cost_limit(), Some((2.5, "EUR".to_owned())));
    }
}
