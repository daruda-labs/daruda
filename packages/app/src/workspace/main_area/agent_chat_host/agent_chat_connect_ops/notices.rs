//! Present ACP advisories through the workspace reporting pipeline.

use crate::surface::strings as s;
use crate::workspace::{Workspace, main_area::pane_tree::PaneId};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

impl Workspace {
    /// Surface a session's non-fatal advisory through the app's error pipeline
    /// (toast → details modal → log) instead of the log alone.
    ///
    /// Every advisory reports the same shape of thing: something the user or
    /// their config asked for did not happen, and the session is fine anyway —
    /// a resume that could not replay history, a configured mode the agent
    /// refused, a config option it would not set. Each is the answer to an
    /// action the user just took, so it is neither noise nor something they
    /// can be expected to find in a log file.
    ///
    /// Reporting here rather than in the view keeps the one-way flow intact
    /// (`Workspace` owns error reporting) and covers every advisory: the other
    /// `apply_event` call site only ever feeds a synthetic terminal error.
    pub(in crate::workspace) fn report_agent_notice(
        &mut self,
        pane_id: PaneId,
        event: &daruda_acp::AcpEvent,
        cx: &mut Context<Self>,
    ) {
        if let daruda_acp::AcpEvent::SessionNotice { session_id, notice } = event {
            use daruda_acp::NoticeSeverityView;
            let severity = match &notice.severity {
                NoticeSeverityView::Info | NoticeSeverityView::Other(_) => ErrorSeverity::Info,
                NoticeSeverityView::Warning => ErrorSeverity::Warning,
                NoticeSeverityView::Error => ErrorSeverity::Error,
            };
            self.report_error(
                ErrorReport::new(notice.title.clone())
                    .message(notice.description.clone().unwrap_or_default())
                    .severity(severity)
                    .with_context("acp_notice_severity", notice.severity.as_str())
                    .with_context("acp_session_id", session_id)
                    .with_context("pane_id", pane_id.to_string())
                    .at(file!(), line!())
                    .build(),
                cx,
            );
            return;
        }
        let message = match event {
            // The adapter's own diagnostic, not authored copy — the same
            // treatment a captured login failure gets.
            daruda_acp::AcpEvent::Notice(message) => message.clone(),
            daruda_acp::AcpEvent::LegacyDelegation => s::agent_chat::legacy_delegation_notice(),
            _ => return,
        };
        self.report_error(
            ErrorReport::new(s::agent_chat::session_notice())
                .message(message.clone())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup(notice_dedup_key(pane_id, &message))
                .build(),
            cx,
        );
    }
}

/// Dedup key for a session advisory's toast.
///
/// Keyed by pane *and* content. A pane-only key looks right and quietly loses
/// information: dedup matches only against a toast still on screen, so a
/// connect that reports both "this conversation could not be restored" and
/// "the configured mode was refused" would show whichever landed first and
/// drop the other — the one case where both facts matter.
///
/// The digest only has to be stable within a run, which is all a live toast
/// queue spans; it keeps a multi-line adapter diagnostic out of the key that
/// lands in the log.
pub(super) fn notice_dedup_key(pane_id: PaneId, message: &str) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.hash(&mut hasher);
    format!("agent_chat.notice.{pane_id}.{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::notice_dedup_key;
    /// A repeat of the same advisory should collapse onto the toast already on
    /// screen rather than stacking.
    #[test]
    fn the_same_advisory_from_one_pane_shares_a_key() {
        assert_eq!(
            notice_dedup_key(7, "could not restore this conversation"),
            notice_dedup_key(7, "could not restore this conversation")
        );
    }

    /// The case a pane-only key would break: a connect can report that history
    /// could not be restored *and* that the configured mode would not apply.
    /// Both matter, and dedup only ever matches a toast still on screen — so a
    /// shared key would silently swallow the second one.
    #[test]
    fn two_different_advisories_from_one_pane_do_not_collapse() {
        assert_ne!(
            notice_dedup_key(7, "could not restore this conversation"),
            notice_dedup_key(7, "the configured mode was refused")
        );
    }

    /// Two panes reporting the same thing are two separate facts about two
    /// separate sessions.
    #[test]
    fn the_same_advisory_from_two_panes_does_not_collapse() {
        assert_ne!(
            notice_dedup_key(7, "the configured mode was refused"),
            notice_dedup_key(8, "the configured mode was refused")
        );
    }
}
