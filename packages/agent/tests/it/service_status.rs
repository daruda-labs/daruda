//! Tests for the public `daruda_agent::service_status` API.

use daruda_agent::FetchError;
use daruda_agent::service_status::*;
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
