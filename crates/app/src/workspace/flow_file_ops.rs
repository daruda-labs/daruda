//! Flow *files* — making one, renaming it, throwing it away, and listing what
//! a lane can run.
//!
//! Split from `flow_ops.rs`: these touch the filesystem and never the engine.
//! The contents of a flow are S4's business (`flow_edit.rs`); this module only
//! ever writes a whole file or removes one.
//!
//! Every operation here takes a path and nothing else — deliberately. A flow's
//! [`FlowOrigin`](super::flow_paths::FlowOrigin) says which directory it came
//! from, and none of the three is read-only: the repository's copy is committed
//! *in order to* be authored, so gating edits on origin would lock the one place
//! a shared flow can live. The working-tree change that a repo flow's edit makes
//! is the point of it, and the git-changes view is where it shows.
//!
//! Origin does reach one decision, and it is the panel's: the sentence the
//! delete dialog says (`right_dock::flows::delete_confirm_body`), because three
//! directories can hold one file name.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Window};

use super::Workspace;
use crate::surface::strings as s;

/// What a new flow file contains. Indentation is load-bearing, which is why it
/// lives here rather than in a locale file; `{agent}` and `{prompt}` are the
/// only substitutions.
const STARTER_FLOW: &str = "\
version: 1
defaults:
  agent:
    id: {agent}
    mode: bypassPermissions
nodes:
  - id: first
    kind: agent
    output: first.md
    prompt: |
      {prompt}
";

/// Why an edit did not reach the file.
///
/// Typed rather than a message, because the callers do different things with
/// them: a form shows the first three beside its fields, `NothingToDo` is not
/// worth saying at all, and an I/O failure is not about the edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum EditRefusal {
    /// The file no longer holds the bytes the change was made against.
    Stale,
    /// The result would not load. Carries the engine's own words for the banner,
    /// and the issues themselves so a caller can point at the boxes they name.
    WouldNotLoad {
        detail: String,
        issues: Vec<daruda_flow::error::ValidationIssue>,
    },
    /// `flow_edit` cannot write this change — flow style, a folded scalar.
    Unsupported(String),
    /// The change amounts to nothing: no edit, and nothing to say.
    NothingToDo,
    Io(String),
}

impl EditRefusal {
    /// What to show a person. The wording lives in the locale files; this only
    /// chooses which line.
    pub(in crate::workspace) fn message(&self) -> String {
        match self {
            EditRefusal::Stale => s::flow_edit_stale(),
            EditRefusal::WouldNotLoad { detail, .. } => s::flow_edit_would_not_load(detail),
            EditRefusal::Unsupported(detail) => s::flow_edit_unsupported(detail),
            EditRefusal::NothingToDo => String::new(),
            // Built by `io` below, which is the only way this variant is
            // made — a whole sentence already, with the path in it.
            EditRefusal::Io(detail) => detail.clone(),
        }
    }

    /// An I/O failure, worded where the path is known. The file system's own
    /// message is not translatable; the sentence around it is.
    fn io(path: &Path, error: &std::io::Error) -> Self {
        EditRefusal::Io(s::flow_file_op_failed(
            &path.display().to_string(),
            &error.to_string(),
        ))
    }

    fn dedup(&self) -> &'static str {
        match self {
            EditRefusal::Stale => "flow.edit_stale",
            EditRefusal::WouldNotLoad { .. } => "flow.edit_would_not_load",
            EditRefusal::Unsupported(_) => "flow.edit_unsupported",
            EditRefusal::NothingToDo => "flow.edit_nothing",
            EditRefusal::Io(_) => "flow.edit_io",
        }
    }
}

impl Workspace {
    /// Create a flow under this project's own directory in the app home and
    /// open its graph.
    ///
    /// Not the repository's `.daruda/flows/`: a flow made here is this
    /// machine's answer for this project, and writing into the working tree
    /// would put it in front of a reviewer who never asked for it. Committing
    /// one is a deliberate move — copy it in — rather than the default.
    pub(in crate::workspace) fn create_flow(
        &mut self,
        typed_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.active_project().map(|p| p.root.clone()) else {
            return;
        };
        let dir = super::flow_paths::project_flows_dir(&self.data_dir, &root);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.report_flow_file_error(s::flow_create_failed_title(), &dir, &e, cx);
            return;
        }
        let path = match super::flow_paths::flow_file_name_in(&dir, typed_name) {
            Ok(path) => path,
            Err(reason) => {
                self.report_flow_name_refusal(reason, cx);
                return;
            }
        };
        let Some(starter) = self.starter_flow() else {
            self.report_flow_no_agent(cx);
            return;
        };
        if let Err(e) = write_new_file(&path, &starter) {
            self.report_flow_file_error(s::flow_create_failed_title(), &path, &e, cx);
            return;
        }
        self.invalidate_flow_list();
        // The write above may have created the directory itself, which the
        // watcher can only anchor on once it exists.
        self.respawn_flow_watcher(cx);
        self.open_flow_graph(&path, window, cx);
    }

    /// A flow that loads on the first open, so the new file draws a graph
    /// rather than an error.
    ///
    /// The agent is `agents[0]` — the same catalog entry a new chat pane runs
    /// under. Writing a literal `claude` here would put a hardcoded agent id in
    /// a file the person keeps, and this app's whole agent story is that the
    /// catalog decides. `None` when the catalog is somehow empty: a template
    /// with no agent id parses as YAML and then fails to load, which is a
    /// worse first impression than refusing.
    fn starter_flow(&self) -> Option<String> {
        let agent = self.agents.first().map(|a| a.id.as_str())?;
        Some(
            STARTER_FLOW
                .replace("{agent}", agent)
                .replace("{prompt}", &s::flow_starter_prompt()),
        )
    }

    /// Rename a flow file, keeping any open graph of it pointed at it.
    ///
    /// The run history is not touched and does not need to be: `run.yaml`
    /// records the resolved spec, not the file it came from, and the panel
    /// lists a lane's runs rather than a flow's.
    pub(in crate::workspace) fn rename_flow(
        &mut self,
        from: &Path,
        typed_name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = from.parent().map(Path::to_path_buf) else {
            return;
        };
        let to = match super::flow_paths::flow_file_name_in(&dir, typed_name) {
            Ok(path) => path,
            Err(reason) => {
                self.report_flow_name_refusal(reason, cx);
                return;
            }
        };
        // WORKAROUND: the name check above is not sealed by this write, unlike
        // `create_flow`'s — `rename(2)` replaces its destination by definition,
        // so a file arriving at the new name in between is overwritten. Closing
        // it needs `renamex_np` / `renameat2`, an unsafe FFI pair for two
        // platforms; deferred until something makes that worth carrying.
        if let Err(e) = std::fs::rename(from, &to) {
            self.report_flow_file_error(s::flow_rename_failed_title(), from, &e, cx);
            return;
        }
        self.repoint_flow_graph_panes(from, &to, cx);
        self.invalidate_flow_list();
        cx.notify();
    }

    /// Delete a flow file. The caller is responsible for having asked first.
    pub(in crate::workspace) fn delete_flow(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Err(e) = std::fs::remove_file(path) {
            self.report_flow_file_error(s::flow_delete_failed_title(), path, &e, cx);
            return;
        }
        // Tell the panes drawing it directly rather than leaving it to the
        // watcher: this is our own deletion, so the tab should say so now, and a
        // pane left alone would persist the path of a file that is gone.
        let views: Vec<_> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .filter_map(|pane| pane.flow_graph_content())
            .filter(|fg| fg.path == path)
            .map(|fg| fg.view.clone())
            .collect();
        for view in views {
            view.update(cx, |view, cx| view.report_file_gone(cx));
        }
        self.invalidate_flow_list();
        cx.notify();
    }

    /// Drop the cached listing so the panel reads the directory again. One
    /// assignment, like the run history's — the snapshot rebuilds it.
    pub(in crate::workspace) fn invalidate_flow_list(&mut self) {
        self.flow_list.invalidate();
    }

    fn report_flow_no_agent(&mut self, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(s::flow_create_failed_title())
                .severity(ErrorSeverity::Warning)
                .message(s::flow_no_agent())
                .dedup("flow.no_agent")
                .at(file!(), line!())
                .build(),
            cx,
        );
    }

    /// Change a flow file through its typed shape, or refuse and say why.
    ///
    /// `base` is the text the change was made against — the graph pane holds it
    /// ([`super::main_area::flow_graph_pane::FlowGraphView::text`]). Two gates
    /// stand between a change and the file, and the file is untouched unless
    /// both pass:
    ///
    /// 1. **The file still says what it said.** Re-read and compare bytes rather
    ///    than an mtime or a hash: we already hold the text the edit was made
    ///    against, so comparing it is both cheaper and exact. (Zed's
    ///    `Buffer::has_conflict` compares mtimes and names its own helper
    ///    `bad_is_greater_than`, with a comment on why that comparison is not
    ///    reliable. We have the better input, so we use it.) Merging is not
    ///    attempted — an editor open beside this app is the normal case, and
    ///    silently merging YAML is how a flow starts doing something nobody
    ///    wrote.
    /// 2. **The result still loads.** The whole file goes back through
    ///    `daruda_flow::load`, so an edit that would leave the flow unrunnable
    ///    is refused with the engine's own reason.
    ///
    /// `Ok(())` when the file was written; `Err` naming what stopped it, for the
    /// caller to put wherever the person is looking. Nothing is reported from
    /// here: a form shows this beside the field that caused it, and a caller with
    /// no such place uses [`Self::report_edit_refusal`].
    pub(in crate::workspace) fn edit_flow(
        &mut self,
        path: &Path,
        base: &str,
        update: impl FnOnce(&mut daruda_flow::parse::FlowFile),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), EditRefusal> {
        match std::fs::read_to_string(path) {
            Ok(on_disk) if on_disk == base => {}
            Ok(_) => return Err(EditRefusal::Stale),
            Err(e) => return Err(EditRefusal::io(path, &e)),
        }

        let edits = super::flow_edit::edits_for_update(base, update)
            .map_err(|err| EditRefusal::Unsupported(err.to_string()))?;
        if edits.is_empty() {
            return Err(EditRefusal::NothingToDo);
        }
        let candidate = super::flow_edit::apply(base, &edits);
        if let Err(e) = daruda_flow::load(&candidate, None) {
            return Err(EditRefusal::WouldNotLoad {
                detail: load_failure_detail(&e),
                issues: match e {
                    daruda_flow::FlowError::Validate(issues) => issues,
                    _ => Vec::new(),
                },
            });
        }
        std::fs::write(path, &candidate).map_err(|e| EditRefusal::io(path, &e))?;
        // Reading back what we just wrote is a no-op for a pane already holding
        // those bytes, so this is for the panes that are not: another lane's
        // graph of the same file, and this one before its watcher event lands.
        self.reload_flow_graphs(Some(path), window, cx);
        self.invalidate_flow_list();
        Ok(())
    }

    /// Report a refusal the way a caller with no form does — a toast, the same
    /// one every other flow-file failure gets.
    pub(in crate::workspace) fn report_edit_refusal(
        &mut self,
        refusal: &EditRefusal,
        cx: &mut Context<Self>,
    ) {
        if matches!(refusal, EditRefusal::NothingToDo) {
            return;
        }
        self.report_flow_edit_refusal(refusal.message(), refusal.dedup(), cx);
    }

    /// A refusal this app made itself — not the engine's, and not an edit that
    /// reached `edit_flow`. The one caller is deleting the last node.
    pub(in crate::workspace) fn report_own_flow_refusal(
        &mut self,
        message: String,
        dedup: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.report_flow_edit_refusal(message, dedup, cx);
    }

    fn report_flow_edit_refusal(
        &mut self,
        message: String,
        dedup: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.report_error(
            ErrorReport::new(s::flow_edit_refused_title())
                .severity(ErrorSeverity::Warning)
                .message(message)
                .dedup(dedup)
                .at(file!(), line!())
                .build(),
            cx,
        );
    }

    fn report_flow_name_refusal(
        &mut self,
        reason: super::flow_paths::FlowNameError,
        cx: &mut Context<Self>,
    ) {
        use super::flow_paths::FlowNameError;
        let message = match reason {
            FlowNameError::Empty => s::flow_name_empty(),
            FlowNameError::HasSeparator => s::flow_name_has_separator(),
            FlowNameError::Taken => s::flow_name_taken(),
        };
        self.report_error(
            ErrorReport::new(s::flow_name_refused_title())
                .severity(ErrorSeverity::Warning)
                .message(message)
                .dedup("flow.name_refused")
                .at(file!(), line!())
                .build(),
            cx,
        );
    }

    fn report_flow_file_error(
        &mut self,
        title: String,
        path: &Path,
        error: &std::io::Error,
        cx: &mut Context<Self>,
    ) {
        self.report_error(
            ErrorReport::new(title)
                .severity(ErrorSeverity::Error)
                .message(s::flow_file_op_failed(
                    &path.display().to_string(),
                    &error.to_string(),
                ))
                .dedup("flow.file_op")
                .at(file!(), line!())
                .build(),
            cx,
        );
    }

    /// The three directories a flow can be listed from, for the active lane.
    /// Resolved in one place so the picker, the panel and the shot scenarios
    /// cannot disagree about what this lane can run.
    pub(in crate::workspace) fn flow_sources(&self) -> Option<(PathBuf, PathBuf, PathBuf)> {
        let cwd = self.active_lane_root()?;
        let project = self
            .active_project()
            .map(|p| super::flow_paths::project_flows_dir(&self.data_dir, &p.root))
            .unwrap_or_default();
        Some((
            cwd,
            project,
            super::flow_paths::global_flows_dir(&self.data_dir),
        ))
    }

    /// The active lane's flow files, for the panel's list.
    ///
    /// Same shape as [`Self::flow_history_for_panel`] and for the same
    /// reason: read disk only while the Flows tab is showing, and only when
    /// the cache belongs to another lane. A file added from outside the app
    /// shows up on the next lane switch — the panel has no way to create one
    /// yet, so there is nothing here that could go stale by our own hand.
    pub(in crate::workspace) fn flow_list_for_panel(
        &mut self,
    ) -> Vec<super::flow_paths::FoundFlow> {
        if self.right_dock_view != daruda_store::project::RightDockView::Flows {
            return Vec::new();
        }
        let lane = self.active;
        if self.flow_list.get(lane).is_none() {
            let Some((cwd, project, global)) = self.flow_sources() else {
                return Vec::new();
            };
            let found = super::flow_paths::list_flows(&cwd, &project, &global);
            self.flow_list.put(lane, found);
        }
        self.flow_list.get(lane).cloned().unwrap_or_default()
    }

    /// The flows whose graph pane, in this lane, holds unsaved inspector edits.
    ///
    /// The panel's ▶ reads the file like the toolbar's does, so it has to be off
    /// for the same reason — but the panel cannot see a pane's form, and a view
    /// must not reach across entities to ask. This is that question answered
    /// once, on the way into the snapshot.
    ///
    /// Gated on the tab like the list above: a panel nobody is looking at must
    /// not cost a walk of the panes, and `is_dirty` reads several inputs per
    /// form. Only the active lane's panes — a pane in another lane is not the
    /// one on screen, and is not where this ▶ would run.
    pub(in crate::workspace) fn flows_with_unsaved_edits(
        &self,
        cx: &gpui::App,
    ) -> Vec<std::path::PathBuf> {
        if self.right_dock_view != daruda_store::project::RightDockView::Flows {
            return Vec::new();
        }
        self.active_runtime()
            .panes
            .iter()
            .filter_map(|pane| pane.flow_graph_content())
            .filter(|fg| fg.view.read(cx).has_unsaved_form(cx))
            .map(|fg| fg.path.clone())
            .collect()
    }
}

/// Why the engine refused the candidate text, in words a person can act on.
///
/// `FlowError::Validate`'s `Display` is a count, so the issues are spelled out
/// here through the same helper the graph pane uses — otherwise a refused save
/// says "1 validation problem(s)" and nothing about which one.
fn load_failure_detail(error: &daruda_flow::FlowError) -> String {
    match error {
        daruda_flow::FlowError::Validate(issues) => s::flow_issue_lines(issues).join(" · "),
        other => other.to_string(),
    }
}

/// Write a file that must not exist yet.
///
/// `create_new` rather than a `path.exists()` check and a plain write: the
/// check is for telling the person the name is taken while they are typing it,
/// and by the time a write runs it is a claim about the past. Here the kernel
/// checks and writes as one, so nothing can arrive in between and be
/// overwritten — and a flow outside a repository has no copy anywhere else.
fn write_new_file(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(contents.as_bytes()).inspect_err(|_| {
        // The name is taken now, by a file with nothing in it. Leaving it would
        // answer the next attempt with "that name is taken" instead of the disk
        // failure that actually happened.
        // SILENT-OK: the write already failed and is what gets reported; a
        // failure to undo it has nothing better to say.
        let _ = std::fs::remove_file(path);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_is_written_and_an_existing_one_is_not_touched() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ship.yaml");

        write_new_file(&path, "first").expect("the name was free");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        let again = write_new_file(&path, "second").expect_err("the name is taken now");
        assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "first",
            "and what was there is still there"
        );
    }
}
