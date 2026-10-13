//! Links and resources clicked in an agent-chat pane: classification, opening
//! in a file view or externally, and the remote-path refusal.

use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_content::link_target::{self, LinkTarget, LocalKind};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::PaneCwd;
use gpui::{App, Context, Window};
use std::path::PathBuf;

impl Workspace {
    /// True when `pane_id`'s session runs on a remote host (`PaneCwd::Remote`
    /// — an SSH/Docker session host, not "pane closed"/"unknown"). ACP paths
    /// the agent reports (`Diff.path`, tool `raw_input` paths, …) are on
    /// *that* host's filesystem, not this machine's, so any action that reads
    /// the path from local disk (the file viewer, an external editor) would
    /// either fail or — worse — silently show an unrelated local file that
    /// happens to share the same absolute path.
    fn diff_pane_is_remote(&self, pane_id: PaneId, cx: &App) -> bool {
        self.agent_chat_view(pane_id)
            .is_some_and(|view| matches!(view.read(cx).cwd(), Some(PaneCwd::Remote(_))))
    }

    /// Open a diff block's file in the pane-area file viewer. Dispatched from
    /// the agent-chat diff header (`render/diff.rs`); `path` is ACP's
    /// `Diff.path`, which the spec guarantees absolute, so no lane-root join is
    /// needed; it opens in whichever lane owns it, or for reference when none
    /// does. A no-op if `pane_id`'s lane
    /// can't be resolved (pane closed mid-click — the render that produced
    /// this callback is already gone).
    /// Toasts and returns instead if the pane's session is remote — see
    /// [`Self::diff_pane_is_remote`].
    pub(in crate::workspace) fn open_diff_in_file_view(
        &mut self,
        pane_id: PaneId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_ref_for_pane(pane_id).is_none() {
            return;
        }
        if self.diff_pane_is_remote(pane_id, cx) {
            let report = ErrorReport::new(s::agent_chat::diff_remote_path_unsupported())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("agent_chat.diff.remote_path_unsupported")
                .build();
            self.report_error(report, cx);
            return;
        }
        self.open_linked_file(path, window, cx);
    }

    /// Classify a link as `pane_id`'s session sees it — the view's own answer
    /// ([`AgentChatView::classify_link`]), which its click also acts on.
    ///
    /// [`AgentChatView::classify_link`]: daruda_chat::view::AgentChatView::classify_link
    pub(in crate::workspace) fn classify_pane_link(
        &self,
        pane_id: PaneId,
        link: &str,
        cx: &App,
    ) -> LinkTarget {
        match self.agent_chat_view(pane_id) {
            Some(view) => view.read(cx).classify_link(link),
            None => link_target::classify(link, None),
        }
    }

    pub(in crate::workspace) fn open_pane_link(
        &mut self,
        pane_id: PaneId,
        link: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let target = self.classify_pane_link(pane_id, link, cx);
        self.open_link_target(pane_id, target, window, cx)
    }

    /// [`Self::classify_pane_link`] for a tool's resource-link URI.
    pub(in crate::workspace) fn classify_pane_resource(
        &self,
        pane_id: PaneId,
        uri: &str,
        mime: Option<&str>,
        cx: &App,
    ) -> LinkTarget {
        match self.agent_chat_view(pane_id) {
            Some(view) => view.read(cx).classify_resource(uri, mime),
            None => link_target::classify_resource(uri, mime, None),
        }
    }

    /// Text opens in the pane file viewer (at the `:line` the link carried);
    /// images, binaries and directories in the OS default handler; URLs in
    /// the platform opener. A missing file and a remote path each report.
    pub(in crate::workspace) fn open_link_target(
        &mut self,
        pane_id: PaneId,
        target: LinkTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (path, line, kind) = match target {
            LinkTarget::Web { url } => {
                cx.open_url(&url);
                return true;
            }
            LinkTarget::Remote => {
                self.report_remote_path_unsupported("agent_chat.link.remote_path", cx);
                return true;
            }
            LinkTarget::Opaque => return false,
            LinkTarget::Local { path, line, kind } => (path, line, kind),
        };
        match kind {
            LocalKind::Missing => {
                let report = ErrorReport::new(s::agent_chat::link_file_missing())
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context(
                        "path",
                        daruda_store::observability::system_info::redact_home(&path),
                    )
                    .dedup("agent_chat.link.file_missing")
                    .build();
                self.report_error(report, cx);
            }
            LocalKind::Image | LocalKind::Binary | LocalKind::Directory => {
                self.open_path_with_system_default(path, cx);
            }
            LocalKind::Text => {
                // An orchestrator pane belongs to no lane, so its links do
                // not open a lane's file viewer.
                if self.lane_ref_for_pane(pane_id).is_none() {
                    return true;
                }
                // The file opens in whichever lane owns it, or for reference.
                self.open_linked_file(path, window, cx);
                if let Some(line) = line {
                    self.set_file_view_mode(
                        crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
                        window,
                        cx,
                    );
                    self.scroll_focused_file_viewer_to_line(line, window, cx);
                }
            }
        }
        true
    }

    fn report_remote_path_unsupported(&mut self, dedup: &'static str, cx: &mut Context<Self>) {
        let report = ErrorReport::new(s::agent_chat::diff_remote_path_unsupported())
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .dedup(dedup)
            .build();
        self.report_error(report, cx);
    }

    /// Open one of a pane's files externally — the user's preferred editor
    /// (Settings → External Editor), or the OS default handler when none is
    /// set. Dispatched from the agent-chat diff header and from the context
    /// menu on a file link, same shape as [`Self::open_diff_in_file_view`],
    /// including the no-op-on-missing-lane and remote-session guard.
    pub(in crate::workspace) fn open_pane_file_externally(
        &mut self,
        pane_id: PaneId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(lane) = self.lane_ref_for_pane(pane_id) else {
            return;
        };
        if self.diff_pane_is_remote(pane_id, cx) {
            self.report_remote_path_unsupported("agent_chat.diff.remote_path_unsupported", cx);
            return;
        }
        self.open_file_externally(lane, path, cx);
    }
}
