use super::*;
use agent_client_protocol::schema::v1::Notice;

#[test]
fn wire_notice_preserves_plain_text_and_open_severity() {
    for severity in ["info", "warning", "error", "_vendor/custom", "future"] {
        let notice: Notice = serde_json::from_value(serde_json::json!({
            "severity": severity,
            "title": "**Not markdown** <title>",
            "description": "A detail\nwith another line"
        }))
        .unwrap();
        let view = SessionNoticeView::from(notice);
        assert_eq!(view.severity.as_str(), severity);
        assert_eq!(view.title, "**Not markdown** <title>");
        assert_eq!(
            view.description.as_deref(),
            Some("A detail\nwith another line")
        );
    }
}

#[test]
fn omitted_and_null_descriptions_stay_absent() {
    for value in [
        serde_json::json!({"severity": "info", "title": "Ready"}),
        serde_json::json!({"severity": "info", "title": "Ready", "description": null}),
    ] {
        let notice: Notice = serde_json::from_value(value).unwrap();
        assert_eq!(SessionNoticeView::from(notice).description, None);
    }
}
