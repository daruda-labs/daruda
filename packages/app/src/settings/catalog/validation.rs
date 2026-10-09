//! Pure rules shared by catalog collection and its regression tests.

use crate::lane::session_host;

pub(in crate::settings) fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// What is wrong with an agent catalog row's transport-specific field, or
/// `None` when nothing is. `"ssh"` checks `host`, `"docker"` checks
/// `container`, and any other `kind` (`"raw"`, or an unrecognized/absent
/// select value) has no extra field to check.
///
/// Both go through [`session_host::checked_bare_word`], the same validator
/// the session-host registry editor and `SessionHostModal` use: the value
/// lands unquoted in the launch command `daruda_config`'s assembler builds,
/// so non-emptiness alone would let a typed host carry its own `ssh` flags
/// or a `;` into that command line. Pure and GPUI-free so it is directly
/// unit-testable — the save loop in `SettingsView::validate` is the only
/// caller.
pub(in crate::settings) fn agent_row_transport_error(
    kind: &str,
    host: &str,
    container: &str,
) -> Option<session_host::SessionHostError> {
    let (value, field) = match kind {
        "ssh" => (host, session_host::SessionHostField::Target),
        "docker" => (container, session_host::SessionHostField::Container),
        _ => return None,
    };
    session_host::checked_bare_word(value, field).err()
}

#[cfg(test)]
mod tests;
