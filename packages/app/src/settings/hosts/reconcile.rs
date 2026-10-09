//! Pure session-host identity and removal-history reconciliation.

const MAX_SESSION_HOST_TOMBSTONES: usize = 20;

/// The string value carried inside a [`daruda_config::SessionHostKind`] —
/// `target` for `Ssh`, `container` for `Docker`. Used to populate a
/// [`daruda_config::SessionHostTombstone::value`] display field, which
/// duplicates what `kind` already carries structurally (see that field's
/// doc in `daruda_config::session_host`).
pub(in crate::settings) fn session_host_kind_value(kind: &daruda_config::SessionHostKind) -> &str {
    match kind {
        daruda_config::SessionHostKind::Ssh { target } => target,
        daruda_config::SessionHostKind::Docker { container } => container,
    }
}

/// The id a saved session-host row carries into `config.toml`: the row's own
/// id, unless the row's Type was changed — i.e. a persisted entry holds that
/// id under the other [`daruda_config::SessionHostKind`] variant.
///
/// A Type change has to retire the id because a lane's `registry_id` resolves
/// on id *and* kind (`lane::session_host` treats a kind mismatch as "not
/// found", never coercing an SSH lane into a Docker one). Keeping the id would
/// leave every linked lane silently unresolvable with nothing recorded;
/// retiring it makes [`reconcile_session_host_tombstones`] log the removal, so
/// the lane reports Orphaned and heals if an equivalent host is registered
/// again.
pub(in crate::settings) fn session_host_entry_id(
    previous_entries: &[daruda_config::SessionHostEntry],
    row_id: daruda_store::project::SessionHostId,
    kind: &daruda_config::SessionHostKind,
) -> daruda_store::project::SessionHostId {
    let retyped = previous_entries.iter().any(|entry| {
        entry.id == row_id && std::mem::discriminant(&entry.kind) != std::mem::discriminant(kind)
    });
    if retyped {
        daruda_store::project::SessionHostId::new()
    } else {
        row_id
    }
}

/// Diff `previous_entries`/`current_entries` (matched by
/// [`daruda_store::project::SessionHostId`]) against `previous_tombstones` to
/// produce the tombstone list committed with the catalog:
///
/// 1. Every entry present in `previous_entries` but missing from
///    `current_entries` was removed by this edit — append a fresh tombstone for
///    it (`redirected_to: None`, `removed_at`).
/// 2. Trim to the most recently removed [`MAX_SESSION_HOST_TOMBSTONES`]
///    (oldest evicted first).
/// 3. Every entry in `current_entries` that is genuinely new (its id was not
///    in `previous_entries`) is matched by exact `(kind, value)` —
///    `SessionHostKind` equality already covers both, since a kind's inner
///    field *is* its value — against the surviving unresolved tombstones
///    (`redirected_to: None`). The most recently removed match gets
///    `redirected_to` set to the new entry's id; an older tie is left
///    unresolved by this tie-break.
///
/// Pure and GPUI-free so it is directly unit-testable — `SettingsView::validate`
/// is the only caller.
pub(in crate::settings) fn reconcile_session_host_tombstones(
    previous_entries: &[daruda_config::SessionHostEntry],
    previous_tombstones: &[daruda_config::SessionHostTombstone],
    current_entries: &[daruda_config::SessionHostEntry],
    removed_at: u64,
) -> Vec<daruda_config::SessionHostTombstone> {
    let mut tombstones = previous_tombstones.to_vec();

    for old in previous_entries {
        if current_entries.iter().any(|e| e.id == old.id) {
            continue;
        }
        tombstones.push(daruda_config::SessionHostTombstone {
            old_id: old.id,
            kind: old.kind.clone(),
            value: session_host_kind_value(&old.kind).to_string(),
            removed_at,
            redirected_to: None,
        });
    }
    if tombstones.len() > MAX_SESSION_HOST_TOMBSTONES {
        tombstones.sort_by_key(|t| t.removed_at);
        let excess = tombstones.len() - MAX_SESSION_HOST_TOMBSTONES;
        tombstones.drain(0..excess);
    }

    for new in current_entries {
        if previous_entries.iter().any(|e| e.id == new.id) {
            continue;
        }
        let redirect = tombstones
            .iter_mut()
            .filter(|t| t.redirected_to.is_none() && t.kind == new.kind)
            .max_by_key(|t| t.removed_at);
        if let Some(t) = redirect {
            t.redirected_to = Some(new.id);
        }
    }

    tombstones
}

#[cfg(test)]
mod tests;
