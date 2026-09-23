//! The snapshot a menu is derived from. Plain data — no entities, no
//! `Window` — so [`super::sections::compose`] is a pure function over it and
//! `render()` never re-enters an entity to build a menu.

use std::path::PathBuf;

use daruda_terminal::session::interval_tree::{LineRange, MarkId};
use gpui::SharedString;

use crate::workspace::main_area::link_target::{LinkTarget, LocalKind};
use crate::workspace::main_area::pane_tree::PaneId;

/// Upper bound on a selection routed to another pane. Mirrors iTerm2's
/// `kMaxSelectedTextLengthForCustomActions` — past this the composer stalls
/// and the user almost certainly did not mean to send it.
pub(super) const SEND_SELECTION_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PaneRole {
    /// The only leaf in its tab, so closing it closes the tab and zoom is a
    /// no-op. Drives both the missing Zoom entry and the close label.
    Solo,
    InSplit {
        zoomed: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LaneAccess {
    Accessible,
    Inaccessible,
}

/// What can be done with the link under the click — decided in
/// [`super::ops`], where the pane's lane and working directory are in reach,
/// so [`super::sections::compose`] stays a pure function over the snapshot.
///
/// The variants are what the menu *offers*, not where the link came from: a
/// terminal link and a chat link that resolve the same way get the same
/// entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ClickLink {
    /// Resolves to a path this machine can reach. `kind` decides the openers
    /// on offer: text gets the viewer and the external editor, everything
    /// else only the OS default handler — the viewer would show a binary
    /// placeholder, a dead entry dressed as a live one.
    File {
        url: String,
        path: PathBuf,
        kind: LocalKind,
    },
    /// An openable URL — the browser takes it.
    Web { url: String },
    /// Neither. Only the text is worth offering.
    Opaque { url: String },
}

impl ClickLink {
    /// Classify a terminal link. The terminal resolves no paths, so its own
    /// openable/not answer is the whole question and `File` never arises.
    pub(super) fn for_terminal(url: String, openable: bool) -> Self {
        if openable {
            ClickLink::Web { url }
        } else {
            ClickLink::Opaque { url }
        }
    }

    /// Classify a chat link from where it resolves. A path wins over looking
    /// like a URL — `file://…` is both — because a resolved path is something
    /// this app can act on, and the browser would be a worse answer to a
    /// more specific question.
    pub(super) fn for_target(url: String, target: LinkTarget) -> Self {
        match target {
            LinkTarget::Local { path, kind, .. } => ClickLink::File { url, path, kind },
            LinkTarget::Web { .. } => ClickLink::Web { url },
            // A remote path is copyable, not openable from this machine.
            LinkTarget::Remote | LinkTarget::Opaque => ClickLink::Opaque { url },
        }
    }

    pub(super) fn url(&self) -> &str {
        match self {
            ClickLink::File { url, .. } | ClickLink::Web { url } | ClickLink::Opaque { url } => url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_link_is_web_only_when_the_terminal_says_it_opens() {
        assert!(matches!(
            ClickLink::for_terminal("https://example.com".into(), true),
            ClickLink::Web { .. }
        ));
        assert!(matches!(
            ClickLink::for_terminal("javascript:alert(1)".into(), false),
            ClickLink::Opaque { .. }
        ));
    }

    /// A path that also parses as a URL — `file://…` is both — must come out
    /// as the file, carrying the kind the openers are chosen by.
    #[test]
    fn a_resolved_path_outranks_looking_like_a_url() {
        let path = PathBuf::from("/repo/src/main.rs");
        let target = LinkTarget::Local {
            path: path.clone(),
            line: Some(12),
            kind: LocalKind::Text,
        };
        assert_eq!(
            ClickLink::for_target("file:///repo/src/main.rs".into(), target),
            ClickLink::File {
                url: "file:///repo/src/main.rs".into(),
                path,
                kind: LocalKind::Text,
            }
        );
    }

    #[test]
    fn an_unresolved_link_is_web_only_when_it_is_a_url() {
        let web = LinkTarget::Web {
            url: "https://example.com".into(),
        };
        assert!(matches!(
            ClickLink::for_target("https://example.com".into(), web),
            ClickLink::Web { .. }
        ));
        // A bare word the resolver declined: no file, no scheme, nothing to
        // open — but its text is still copyable.
        assert!(matches!(
            ClickLink::for_target("somewhere".into(), LinkTarget::Opaque),
            ClickLink::Opaque { .. }
        ));
    }
}

/// What sits under the click. **Not exclusive** — an annotation covers a line
/// range while a link covers cells, so both can be present at once. Two
/// independent `Option`s rather than one enum for exactly that reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClickInfo {
    pub(super) link: Option<ClickLink>,
    pub(super) annotation: Option<MarkId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SendTarget {
    pub(super) pane_id: PaneId,
    pub(super) label: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PaneMenuKind {
    Terminal {
        annotation_range: Option<LineRange>,
    },
    AgentChat {
        busy: bool,
    },
    /// `selected` is whether the graph has exactly one node selected — deleting
    /// needs one, and the menu opens with or without. `dep_selected` is the
    /// same question for a line: clicking one selects it and draws it in the
    /// accent, so what would be removed is visible before the row is chosen.
    FlowGraph {
        selected: bool,
        dep_selected: bool,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PaneMenuContext {
    pub(super) pane_id: PaneId,
    pub(super) role: PaneRole,
    pub(super) lane: LaneAccess,
    /// Captured *before* focus moves to the menu target, because clicking a
    /// menu item is a left-click outside the text block and clears the live
    /// selection first.
    pub(super) selection: Option<SharedString>,
    /// `None` for a pane-header right-click — there is no cell under it, so
    /// click-derived entries drop out without a separate code path.
    pub(super) click: Option<ClickInfo>,
    pub(super) send_targets: Vec<SendTarget>,
    pub(super) kind: PaneMenuKind,
}
