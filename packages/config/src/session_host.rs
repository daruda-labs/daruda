//! Session host registry — named, reusable SSH/Docker targets a lane's
//! `daruda_store::project::LaneSessionHost::Ssh`/`Docker` can reference by
//! [`SessionHostId`] instead of repeating the same target/container as free
//! text on every lane.
//!
//! Schema only: this module defines the catalog/tombstone shapes but does
//! not itself resolve a lane's `registry_id` back to its catalog row or
//! chase [`SessionHostTombstone`] redirects — that logic lives on top of
//! this data model in `daruda::lane::session_host` (`effective_session_host`,
//! `resolve_catalog_id`).

use daruda_store::project::SessionHostId;
use serde::{Deserialize, Serialize};

/// One user-registered host, addressable by [`SessionHostId`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHostEntry {
    pub id: SessionHostId,
    pub label: String,
    pub kind: SessionHostKind,
}

/// The connection shape a [`SessionHostEntry`] carries — mirrors the two
/// remote `daruda_store::project::LaneSessionHost` variants, minus the
/// per-lane working directory (`session_path`), which stays on the lane
/// since the same registered host can be reused at a different path by
/// more than one lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionHostKind {
    Ssh { target: String },
    Docker { container: String },
}

/// Record of a registry entry that was removed, kept so a lane still
/// carrying the deleted [`SessionHostId`] in its `registry_id` can show what
/// it used to point at instead of silently reverting to plain free text.
/// `redirected_to` is set when the removal was actually a merge into another
/// surviving entry, so referencing lanes can be repointed there instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHostTombstone {
    pub old_id: SessionHostId,
    pub kind: SessionHostKind,
    pub value: String,
    /// Unix timestamp (seconds) the entry was removed.
    pub removed_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirected_to: Option<SessionHostId>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
