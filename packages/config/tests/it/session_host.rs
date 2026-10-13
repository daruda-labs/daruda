//! Public API tests for session host.

use daruda_config::session_host::*;
use daruda_store::project::SessionHostId;

fn ssh_entry() -> SessionHostEntry {
    SessionHostEntry {
        id: SessionHostId::new(),
        label: "Build box".to_string(),
        kind: SessionHostKind::Ssh {
            target: "vm-work".to_string(),
        },
    }
}

fn docker_entry() -> SessionHostEntry {
    SessionHostEntry {
        id: SessionHostId::new(),
        label: "Dev container".to_string(),
        kind: SessionHostKind::Docker {
            container: "dev-1".to_string(),
        },
    }
}

#[test]
fn session_host_entry_round_trips_through_toml() {
    for entry in [ssh_entry(), docker_entry()] {
        let toml_str = toml::to_string(&entry).expect("serialize");
        let back: SessionHostEntry = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, entry, "{toml_str}");
    }
}

#[test]
fn session_host_tombstone_round_trips_through_toml() {
    let tombstone = SessionHostTombstone {
        old_id: SessionHostId::new(),
        kind: SessionHostKind::Ssh {
            target: "old-box".to_string(),
        },
        value: "old-box".to_string(),
        removed_at: 1_700_000_000,
        redirected_to: Some(SessionHostId::new()),
    };
    let toml_str = toml::to_string(&tombstone).expect("serialize");
    let back: SessionHostTombstone = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, tombstone, "{toml_str}");

    // A removal with no merge target (`redirected_to: None`) round-trips too.
    let tombstone_no_redirect = SessionHostTombstone {
        redirected_to: None,
        ..tombstone
    };
    let toml_str = toml::to_string(&tombstone_no_redirect).expect("serialize");
    let back: SessionHostTombstone = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, tombstone_no_redirect, "{toml_str}");
}
