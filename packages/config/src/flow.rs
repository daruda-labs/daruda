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
/// Cap on budget units — one per runner call or correction turn. The defence
/// against a repair loop that keeps re-deriving the same nodes.
pub const DEFAULT_MAX_NODE_RUNS: u32 = 100;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FlowConfig {
    /// Minutes before a run is stopped. `0` disables the deadline.
    pub timeout_minutes: u32,
    /// Maximum budget units in one run. `0` disables the count.
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
