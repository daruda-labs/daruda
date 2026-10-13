//! Lane and dock-view serialization.

use daruda_store::project::SessionHostId;
use daruda_store::project::lane::*;

#[test]
fn right_dock_tabs_round_trip() {
    for (stored, expected) in [
        ("\"usage\"", RightDockView::Usage),
        ("\"skills\"", RightDockView::Skills),
        ("\"tools\"", RightDockView::Tools),
    ] {
        let parsed: RightDockView = serde_json::from_str(stored).expect(stored);
        assert_eq!(parsed, expected, "{stored}");
        assert_eq!(
            serde_json::to_string(&parsed).expect("serialise"),
            stored,
            "round trip"
        );
    }
}

/// A file saved while Tasks / Flows were tabs must still load.
#[test]
fn a_former_page_tab_restores_to_the_default_tab() {
    for stored in ["\"tasks\"", "\"flows\""] {
        let parsed: RightDockView = serde_json::from_str(stored).expect(stored);
        assert_eq!(parsed, RightDockView::default(), "{stored}");
    }
    assert!(serde_json::from_str::<RightDockView>("\"nope\"").is_err());
}

#[test]
fn workspace_page_round_trips() {
    for (page, stored) in [
        (WorkspacePage::Tasks, "\"tasks\""),
        (WorkspacePage::Flows, "\"flows\""),
    ] {
        assert_eq!(serde_json::to_string(&page).unwrap(), stored);
        assert_eq!(serde_json::from_str::<WorkspacePage>(stored).unwrap(), page);
    }
}

#[test]
fn lane_session_host_round_trips_every_variant_with_registry_id() {
    let cases = [
        LaneSessionHost::Local,
        LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        },
        LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(SessionHostId::new()),
        },
        LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: None,
        },
        LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: Some(SessionHostId::new()),
        },
    ];
    for case in cases {
        let json = serde_json::to_string(&case).expect("serialize");
        let back: LaneSessionHost = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, case, "{json}");
    }
}

/// Regression anchor: a lane JSON file persisted before the registry
/// existed has no `registry_id` key at all — it must still deserialize,
/// with `registry_id` defaulting to `None`.
#[test]
fn missing_registry_id_key_deserializes_as_none() {
    let json = r#"{"type":"ssh","target":"vm-work","session_path":"/srv/app"}"#;
    let host: LaneSessionHost = serde_json::from_str(json).expect("deserialize");
    assert_eq!(
        host,
        LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        }
    );

    let json = r#"{"type":"docker","container":"dev-1","session_path":"/workspace"}"#;
    let host: LaneSessionHost = serde_json::from_str(json).expect("deserialize");
    assert_eq!(
        host,
        LaneSessionHost::Docker {
            container: "dev-1".into(),
            session_path: "/workspace".into(),
            registry_id: None,
        }
    );
}
