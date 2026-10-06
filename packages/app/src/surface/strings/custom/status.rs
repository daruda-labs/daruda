use crate::surface::strings as s;

/// Resolve the user-visible label for a service-status snapshot.
///
/// Mirrors the Übersicht widget rule: `None` ignores the upstream
/// description (always show the hard-coded "Operational"), other
/// indicators prefer the upstream description and fall back to a
/// canned default when it's empty. `Unknown` ignores description
/// entirely — a parse miss shouldn't surface garbage strings.
pub(crate) fn service_status_label(status: &daruda_agent::ServiceStatus) -> String {
    use daruda_agent::StatusIndicator;
    match status.indicator {
        StatusIndicator::None => s::status::operational().to_string(),
        StatusIndicator::Unknown => s::status::unknown().to_string(),
        StatusIndicator::Minor => {
            if status.description.is_empty() {
                s::status::minor_default().to_string()
            } else {
                status.description.clone()
            }
        }
        StatusIndicator::Major => {
            if status.description.is_empty() {
                s::status::major_default().to_string()
            } else {
                status.description.clone()
            }
        }
        StatusIndicator::Critical => {
            if status.description.is_empty() {
                s::status::critical_default().to_string()
            } else {
                status.description.clone()
            }
        }
    }
}
