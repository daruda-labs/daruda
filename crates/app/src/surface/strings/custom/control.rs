use crate::surface::strings as s;

/// The finished-run line under a completion ping's header, from its
/// already-formatted segments (duration, "3 files edited", …).
pub(crate) fn remote_run_summary(segments: &[String]) -> String {
    s::control::run_summary(segments.join(" · "))
}
