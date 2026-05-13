//! Anthropic service-status indicator from
//! `https://status.claude.com/api/v2/status.json`.
//!
//! The Usage tab renders this as a small pill at the top —
//! green when "operational", yellow / orange / red as the public
//! Statuspage indicator climbs. This is the *upstream* health check;
//! distinct from local Claude Code session state which lives in
//! [`crate::status`].
//!
//! Naming: the file is `service_status.rs` (not `status.rs`) because
//! `crate::status` already owns `SessionStatus`. Same crate, two
//! orthogonal "status" concepts; the longer module name keeps them
//! straight at every call site.

use std::time::SystemTime;

use serde_json::Value;

use crate::http::{FetchError, get_json};

/// Public Statuspage health indicator. Maps directly onto the
/// `status.indicator` string in the upstream response, with
/// `Unknown` covering both the explicit "unknown" string and any
/// indicator value Anthropic adds without us redeploying.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StatusIndicator {
    /// Operational. Renderer ignores the response description and
    /// shows a hard-coded "Operational" label, matching the
    /// Übersicht widget — Statuspage tends to leave stale "All
    /// systems normal" descriptions even on the green path.
    None,
    /// Some degraded performance.
    Minor,
    /// Partial outage.
    Major,
    /// Major outage.
    Critical,
    /// Indicator missing or unrecognised. Renderer dims the pill
    /// and labels it "Status unavailable" so a parser miss doesn't
    /// look like green.
    #[default]
    Unknown,
}

impl StatusIndicator {
    /// Map the upstream `status.indicator` string. Anything outside
    /// the four documented values collapses to `Unknown` rather
    /// than panicking — the upstream contract has shifted before
    /// (e.g. capitalization changes) and we'd rather render a
    /// dimmed pill than crash the right panel.
    ///
    /// Named `from_indicator_str` (not `from_str`) so it doesn't
    /// shadow `std::str::FromStr`; our semantics differ
    /// (unknown → `Unknown` variant, not `Err`).
    pub fn from_indicator_str(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "minor" => Self::Minor,
            "major" => Self::Major,
            "critical" => Self::Critical,
            _ => Self::Unknown,
        }
    }
}

/// Snapshot of the public service-status response. `description`
/// echoes Anthropic's free-form status message ("Increased 4xx
/// errors on /messages", etc.); the renderer uses it for
/// `Minor`/`Major`/`Critical` and ignores it for `None`/`Unknown`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceStatus {
    pub indicator: StatusIndicator,
    /// Free-form status string from the upstream response.
    /// Empty when the API omits or nulls the field.
    pub description: String,
    /// Wall-clock when the response landed. `None` before first
    /// successful fetch — renderer falls back to `Unknown` styling.
    pub fetched_at: Option<SystemTime>,
}

/// Fetch and parse the public service-status response. Synchronous
/// — callers wrap in `cx.background_executor().spawn(...)`.
pub fn fetch_service_status() -> Result<ServiceStatus, FetchError> {
    let body = get_json("https://status.claude.com/api/v2/status.json", &[])?;
    parse_service_status(&body)
}

/// Parse the JSON body returned by `/api/v2/status.json`. Split
/// out for unit tests with mock fixtures.
pub fn parse_service_status(value: &Value) -> Result<ServiceStatus, FetchError> {
    let s = value
        .get("status")
        .ok_or_else(|| FetchError::Parse("missing `status` object".into()))?;
    let indicator = s
        .get("indicator")
        .and_then(Value::as_str)
        .map(StatusIndicator::from_indicator_str)
        .unwrap_or_default();
    let description = s
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(ServiceStatus {
        indicator,
        description,
        fetched_at: Some(SystemTime::now()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_operational_response() {
        let body = json!({
            "status": { "indicator": "none", "description": "All Systems Operational" },
        });
        let parsed = parse_service_status(&body).unwrap();
        assert_eq!(parsed.indicator, StatusIndicator::None);
        assert_eq!(parsed.description, "All Systems Operational");
        assert!(parsed.fetched_at.is_some());
    }

    #[test]
    fn parse_each_indicator_variant() {
        for (raw, want) in [
            ("none", StatusIndicator::None),
            ("minor", StatusIndicator::Minor),
            ("major", StatusIndicator::Major),
            ("critical", StatusIndicator::Critical),
        ] {
            let body = json!({ "status": { "indicator": raw, "description": "" } });
            assert_eq!(parse_service_status(&body).unwrap().indicator, want);
        }
    }

    #[test]
    fn parse_unknown_indicator_collapses_to_unknown() {
        let body = json!({ "status": { "indicator": "weird_new_value", "description": "" } });
        let parsed = parse_service_status(&body).unwrap();
        assert_eq!(parsed.indicator, StatusIndicator::Unknown);
    }

    #[test]
    fn parse_missing_status_object_errors() {
        let body = json!({});
        let err = parse_service_status(&body).unwrap_err();
        assert!(matches!(err, FetchError::Parse(_)));
    }

    #[test]
    fn parse_missing_indicator_field_yields_unknown() {
        let body = json!({ "status": { "description": "weird" } });
        let parsed = parse_service_status(&body).unwrap();
        assert_eq!(parsed.indicator, StatusIndicator::Unknown);
        assert_eq!(parsed.description, "weird");
    }

    #[test]
    fn parse_missing_description_yields_empty_string() {
        let body = json!({ "status": { "indicator": "minor" } });
        let parsed = parse_service_status(&body).unwrap();
        assert_eq!(parsed.indicator, StatusIndicator::Minor);
        assert!(parsed.description.is_empty());
    }

    #[test]
    fn from_indicator_str_matches_documented_values() {
        assert_eq!(
            StatusIndicator::from_indicator_str("none"),
            StatusIndicator::None
        );
        assert_eq!(
            StatusIndicator::from_indicator_str("minor"),
            StatusIndicator::Minor
        );
        assert_eq!(
            StatusIndicator::from_indicator_str("major"),
            StatusIndicator::Major
        );
        assert_eq!(
            StatusIndicator::from_indicator_str("critical"),
            StatusIndicator::Critical
        );
        assert_eq!(
            StatusIndicator::from_indicator_str(""),
            StatusIndicator::Unknown
        );
        assert_eq!(
            StatusIndicator::from_indicator_str("None"),
            StatusIndicator::Unknown
        ); // case-sensitive
    }
}
