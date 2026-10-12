use super::{reconcile_session_host_tombstones, session_host_entry_id};
use daruda_config::{SessionHostEntry, SessionHostKind, SessionHostTombstone};
use daruda_store::project::SessionHostId;

fn ssh_entry(id: SessionHostId, label: &str, target: &str) -> SessionHostEntry {
    SessionHostEntry {
        id,
        label: label.to_string(),
        kind: SessionHostKind::Ssh {
            target: target.to_string(),
        },
    }
}

#[test]
fn an_unchanged_catalog_produces_no_new_tombstones() {
    let id = SessionHostId::new();
    let entries = vec![ssh_entry(id, "Box", "vm-work")];
    let tombstones = reconcile_session_host_tombstones(&entries, &[], &entries, 100);
    assert!(tombstones.is_empty());
}

#[test]
fn a_removed_entry_gets_a_fresh_tombstone() {
    let id = SessionHostId::new();
    let previous = vec![ssh_entry(id, "Box", "vm-work")];
    let tombstones = reconcile_session_host_tombstones(&previous, &[], &[], 100);
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].old_id, id);
    assert_eq!(
        tombstones[0].kind,
        SessionHostKind::Ssh {
            target: "vm-work".into()
        }
    );
    assert_eq!(tombstones[0].value, "vm-work");
    assert_eq!(tombstones[0].removed_at, 100);
    assert_eq!(tombstones[0].redirected_to, None);
}

/// A row surviving with the same id but an edited target/label is an
/// in-place edit, not a removal — the catalog's own id-based resolution
/// already picks up the new value (see `lane::session_host::resolve_catalog_id`),
/// so no tombstone should be recorded for it.
#[test]
fn an_edited_entry_that_keeps_its_id_is_not_tombstoned() {
    let id = SessionHostId::new();
    let previous = vec![ssh_entry(id, "Box", "old-target")];
    let current = vec![ssh_entry(id, "Renamed", "new-target")];
    let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
    assert!(tombstones.is_empty());
}

/// A row that keeps its id while switching Type stops resolving for every
/// lane linked to it (id *and* kind must match), so the id is retired and
/// the removal recorded — with no redirect, since bridging kinds would
/// turn an SSH lane into a Docker one.
#[test]
fn a_retyped_row_retires_its_id_and_gets_tombstoned() {
    let row_id = SessionHostId::new();
    let previous = vec![ssh_entry(row_id, "Box", "vm-work")];
    let retyped = SessionHostKind::Docker {
        container: "dev-1".into(),
    };
    let saved_id = session_host_entry_id(&previous, row_id, &retyped);
    assert_ne!(saved_id, row_id);

    let current = vec![SessionHostEntry {
        id: saved_id,
        label: "Box".to_string(),
        kind: retyped,
    }];
    let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].old_id, row_id);
    assert_eq!(
        tombstones[0].kind,
        SessionHostKind::Ssh {
            target: "vm-work".into()
        }
    );
    assert_eq!(tombstones[0].redirected_to, None);
}

/// Editing only the value keeps the same kind variant, which still
/// resolves — retiring the id there would orphan lanes for nothing.
#[test]
fn an_edited_value_keeps_the_rows_id() {
    let row_id = SessionHostId::new();
    let previous = vec![ssh_entry(row_id, "Box", "vm-work")];
    let kind = SessionHostKind::Ssh {
        target: "vm-other".into(),
    };
    assert_eq!(session_host_entry_id(&previous, row_id, &kind), row_id);
}

#[test]
fn a_row_with_no_persisted_entry_keeps_its_id() {
    let row_id = SessionHostId::new();
    let kind = SessionHostKind::Docker {
        container: "dev-1".into(),
    };
    assert_eq!(session_host_entry_id(&[], row_id, &kind), row_id);
}

#[test]
fn a_new_entry_matching_kind_and_value_redirects_the_matching_tombstone() {
    let old_id = SessionHostId::new();
    let new_id = SessionHostId::new();
    let previous = vec![ssh_entry(old_id, "Box", "vm-work")];
    let current = vec![ssh_entry(new_id, "Box (recreated)", "vm-work")];
    let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].old_id, old_id);
    assert_eq!(tombstones[0].redirected_to, Some(new_id));
}

/// A Docker entry must never redirect an SSH tombstone (or vice versa)
/// even if the string value happens to collide.
#[test]
fn a_kind_mismatch_never_redirects() {
    let old_id = SessionHostId::new();
    let new_id = SessionHostId::new();
    let previous = vec![ssh_entry(old_id, "Box", "shared-name")];
    let current = vec![SessionHostEntry {
        id: new_id,
        label: "Container".into(),
        kind: SessionHostKind::Docker {
            container: "shared-name".into(),
        },
    }];
    let tombstones = reconcile_session_host_tombstones(&previous, &[], &current, 100);
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].redirected_to, None);
}

/// Two live tombstones share `(kind, value)` — only the most recently
/// removed one gets redirected; the older one stays unresolved rather
/// than being touched.
#[test]
fn ties_redirect_only_the_most_recently_removed_tombstone() {
    let older_id = SessionHostId::new();
    let newer_id = SessionHostId::new();
    let recreated_id = SessionHostId::new();
    let previous_tombstones = vec![
        SessionHostTombstone {
            old_id: older_id,
            kind: SessionHostKind::Ssh {
                target: "vm-work".into(),
            },
            value: "vm-work".into(),
            removed_at: 50,
            redirected_to: None,
        },
        SessionHostTombstone {
            old_id: newer_id,
            kind: SessionHostKind::Ssh {
                target: "vm-work".into(),
            },
            value: "vm-work".into(),
            removed_at: 75,
            redirected_to: None,
        },
    ];
    let current = vec![ssh_entry(recreated_id, "Box", "vm-work")];
    let tombstones = reconcile_session_host_tombstones(&[], &previous_tombstones, &current, 100);
    let older = tombstones.iter().find(|t| t.old_id == older_id).unwrap();
    let newer = tombstones.iter().find(|t| t.old_id == newer_id).unwrap();
    assert_eq!(older.redirected_to, None);
    assert_eq!(newer.redirected_to, Some(recreated_id));
}

/// A tombstone that already resolved to a surviving entry is never
/// re-targeted by a second recreation of the same value.
#[test]
fn an_already_resolved_tombstone_is_never_redirected_again() {
    let old_id = SessionHostId::new();
    let first_redirect = SessionHostId::new();
    let second_id = SessionHostId::new();
    let previous_tombstones = vec![SessionHostTombstone {
        old_id,
        kind: SessionHostKind::Ssh {
            target: "vm-work".into(),
        },
        value: "vm-work".into(),
        removed_at: 50,
        redirected_to: Some(first_redirect),
    }];
    let current = vec![ssh_entry(second_id, "Box again", "vm-work")];
    let tombstones = reconcile_session_host_tombstones(&[], &previous_tombstones, &current, 100);
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].redirected_to, Some(first_redirect));
}

#[test]
fn the_tombstone_list_is_trimmed_to_the_most_recent_twenty_oldest_evicted_first() {
    let previous_tombstones: Vec<SessionHostTombstone> = (0..20)
        .map(|i| SessionHostTombstone {
            old_id: SessionHostId::new(),
            kind: SessionHostKind::Ssh {
                target: format!("box-{i}"),
            },
            value: format!("box-{i}"),
            removed_at: i,
            redirected_to: None,
        })
        .collect();
    let oldest_id = previous_tombstones[0].old_id;
    let newest_removed_id = SessionHostId::new();
    let previous_entries = vec![ssh_entry(newest_removed_id, "Freshly removed", "box-fresh")];
    // Removing this one 21st-oldest entry pushes the total to 21, which
    // must evict exactly the single oldest tombstone (removed_at: 0).
    let tombstones =
        reconcile_session_host_tombstones(&previous_entries, &previous_tombstones, &[], 1_000);
    assert_eq!(tombstones.len(), 20);
    assert!(
        !tombstones.iter().any(|t| t.old_id == oldest_id),
        "the oldest tombstone must be evicted"
    );
    assert!(
        tombstones
            .iter()
            .any(|t| t.old_id == newest_removed_id && t.removed_at == 1_000),
        "the just-created tombstone must survive the trim"
    );
}
