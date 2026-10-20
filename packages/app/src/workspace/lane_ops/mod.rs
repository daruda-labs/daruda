//! Lane lifecycle operations on `Workspace`: create / remove validation
//! and execution, modal openers, slot-id allocation, and the lane
//! activation path (re-points `active` into the single runtimes map).
//! Git CLI runs on `cx.background_executor`; post-git state mutations
//! return via `cx.update`.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::ProjectId;
use gpui::Context;

use super::Workspace;

mod activation;
mod lifecycle;
mod metadata;
mod switcher;

/// Immutable plan from `CreateWorktreeModal::validate` — sanitized
/// branch, derived new-path, and repo_root, so the modal can ship clones
/// to a background executor without borrowing `Workspace`.
#[derive(Debug, Clone)]
pub(in crate::workspace) struct CreateWorktreePlan {
    pub branch: String,
    pub new_path: std::path::PathBuf,
    pub repo_root: std::path::PathBuf,
    /// Ref the new branch is based on. `None` = git's default (current
    /// HEAD when paired with `-b`).
    pub base_ref: Option<String>,
    /// Free-form description; surfaced as the lane row sublabel.
    pub description: Option<String>,
    /// The host picked in the create form's registry dropdown. `None` keeps
    /// the freshly-created `Lane` at its `Lane::git` default (`session_host:
    /// None`, i.e. unanswered/Local) — only an explicit non-Local pick
    /// carries a value here, so the modal's Local default never writes a
    /// redundant `Some(LaneSessionHost::Local)`.
    pub session_host: Option<daruda_store::project::LaneSessionHost>,
}

/// Whether work opened in a lane is brought in front of the person at the
/// window. Work someone asked for elsewhere — the phone, the orchestrator —
/// must not move the view they are typing into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum LaneReveal {
    /// Activate the lane and focus the new pane.
    Foreground,
    /// Leave every lane's active tab and the window's focus as they are.
    /// The new pane still runs: an Agent chat connects without a focus.
    Background,
}

/// Counterpart to `CreateWorktreePlan` for the remove path.
#[derive(Debug)]
pub(in crate::workspace) struct RemoveWorktreePlan {
    pub path: std::path::PathBuf,
    pub repo_root: std::path::PathBuf,
    /// Resolved here because `validate_remove_lane` runs while the
    /// checkout still exists; see [`super::flow_paths::forget_lane_lock`].
    pub lock_dir: Option<std::path::PathBuf>,
}

/// `"<project> / <lane>"`. One derivation because three surfaces show it —
/// the lane switcher, a flow run row, the Ports segment's owner column —
/// and a lane read off two of them at once has to be spelled the same.
pub(in crate::workspace) fn lane_label(project_name: &str, lane: &crate::lane::Lane) -> String {
    format!("{project_name} / {}", lane.display_name())
}

impl Workspace {
    /// Resolve the base ref for a new lane against the active project
    /// (lanes are created into `active_project()`, matching
    /// `finalize_create_lane`). Single resolution point — both
    /// `add_lane` consumers route through this so the recorded base
    /// always matches what git actually branched from.
    pub(in crate::workspace) fn resolve_lane_base_ref(
        &self,
        requested: Option<String>,
    ) -> Option<String> {
        resolved_lane_base_ref(
            requested,
            self.active_project()
                .and_then(crate::project::Project::effective_base_branch),
        )
    }

    /// Same, for a *named* project.
    ///
    /// The active-project form above is right for the create form, which only
    /// ever makes a lane in the project on screen. A control command names its
    /// project, and resolving that one against the active project's default
    /// branch would branch from the wrong repository.
    pub(in crate::workspace) fn resolve_lane_base_ref_for(
        &self,
        project: ProjectId,
        requested: Option<String>,
    ) -> Option<String> {
        resolved_lane_base_ref(
            requested,
            self.project_for(project)
                .and_then(crate::project::Project::effective_base_branch),
        )
    }
}

fn git_repo_root_of(lanes: &[crate::lane::Lane]) -> Option<std::path::PathBuf> {
    lanes.iter().find_map(|w| match &w.kind {
        daruda_store::project::LaneKind::Git { repo_root, .. } => Some(repo_root.clone()),
        _ => None,
    })
}

/// Repo-basename fallback when a path has no final component. Never displayed
/// — it only keeps the derived directory name from starting with a dash.
const UNNAMED_REPO: &str = "project";

/// Where a new lane's checkout goes: a sibling of the repository named
/// `<repo>-<branch>`, with `/` folded to `-`.
///
/// One function because three paths derive it — the create form, the task
/// workflow, and the control surface — and a lane an agent made has to sit
/// where a lane the user made would. Folding the slash matters: `feat/x` would
/// otherwise nest the checkout inside a stray `repo-feat` directory.
pub(in crate::workspace) fn lane_checkout_path(
    repo_root: &std::path::Path,
    branch: &str,
) -> std::path::PathBuf {
    let repo_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(UNNAMED_REPO);
    // A branch name may hold what a directory name cannot (`a|b`, `x:y`).
    let suffix = crate::file_name::sanitized(branch);
    repo_root
        .parent()
        .unwrap_or(repo_root)
        .join(format!("{repo_name}-{suffix}"))
}

impl Workspace {
    /// Select `path` in the platform's file manager — Finder, File Explorer,
    /// or the desktop's own. gpui's `reveal_path` reports no failure, so the
    /// one a lane realistically hits, its folder gone, is checked first.
    pub(in crate::workspace) fn reveal_in_file_manager(
        &mut self,
        path: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        if !path.exists() {
            let report = ErrorReport::new(crate::surface::strings::error::reveal_path_missing())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context(
                    "path",
                    daruda_store::observability::system_info::redact_home(path),
                )
                .dedup("files.reveal")
                .build();
            self.report_error(report, cx);
            return;
        }
        cx.reveal_path(path);
    }
}

/// Effective base ref for a new lane: an explicit user choice wins, else
/// the project's `effective_base_branch`; `None` lets git use HEAD.
fn resolved_lane_base_ref(requested: Option<String>, project_base: Option<&str>) -> Option<String> {
    requested.or_else(|| project_base.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::{lane_checkout_path, resolved_lane_base_ref};

    /// git takes `a|b` as a branch name; Windows will not take it as a
    /// directory name, so `git worktree add` failed after the form accepted it.
    #[test]
    fn a_branch_becomes_a_directory_name_every_platform_takes() {
        let repo = std::path::Path::new("/work/repo");
        assert_eq!(
            lane_checkout_path(repo, "feat/login"),
            std::path::Path::new("/work/repo-feat-login")
        );
        assert_eq!(
            lane_checkout_path(repo, "fix/a|b\"<c>"),
            std::path::Path::new("/work/repo-fix-a-b--c-")
        );
    }

    #[test]
    fn explicit_request_overrides_project_branches() {
        let out = resolved_lane_base_ref(Some("origin/feat".to_string()), Some("develop"));
        assert_eq!(out.as_deref(), Some("origin/feat"));
    }

    #[test]
    fn falls_back_to_the_project_base_when_unspecified() {
        let out = resolved_lane_base_ref(None, Some("develop"));
        assert_eq!(out.as_deref(), Some("develop"));
    }

    #[test]
    fn returns_none_when_nothing_known() {
        let out = resolved_lane_base_ref(None, None);
        assert_eq!(out, None);
    }
}
