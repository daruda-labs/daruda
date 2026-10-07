//! Flow *files* — making one, renaming it, throwing it away, and listing what
//! a lane can run.
//!
//! Split from `flow_ops.rs`: these touch the filesystem and never the engine.
//! The contents of a flow are S4's business (`daruda_flow_edit`); this module only
//! ever writes a whole file or removes one.
//!
//! Every operation here takes a path and nothing else — deliberately. A flow's
//! [`FlowOrigin`](super::flow_paths::FlowOrigin) says which directory it came
//! from, and none of the three is read-only: the repository's copy is committed
//! *in order to* be authored, so gating edits on origin would lock the one place
//! a shared flow can live. The working-tree change that a repo flow's edit makes
//! is the point of it, and the git-changes view is where it shows.
//!
//! Origin does reach one decision: the sentence the delete dialog says
//! ([`flow_paths::delete_confirm_body`](super::flow_paths::delete_confirm_body)),
//! because three directories can hold one file name. The dialog itself is
//! [`ask_before_deleting`] here, beside the [`Workspace::delete_flow`] its yes
//! calls — a panel row is one of the two places it opens from, not its owner.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{App, Context, WeakEntity, Window};

use super::Workspace;
use crate::surface::strings as s;

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
            EditRefusal::Stale => s::flow::edit_stale(),
            EditRefusal::WouldNotLoad { detail, .. } => s::flow::edit_would_not_load(detail),
            EditRefusal::Unsupported(detail) => s::flow::edit_unsupported(detail),
            EditRefusal::NothingToDo => String::new(),
            // Built by `io` below, which is the only way this variant is
            // made — a whole sentence already, with the path in it.
            EditRefusal::Io(detail) => detail.clone(),
        }
    }

    /// An I/O failure, worded where the path is known. The file system's own
    /// message is not translatable; the sentence around it is.
    fn io(path: &Path, error: &std::io::Error) -> Self {
        EditRefusal::Io(s::flow::file_op_failed(
            path.display().to_string(),
            error.to_string(),
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
    pub(in crate::workspace) fn create_flow_in(
        &mut self,
        lane: daruda_store::project::LaneRef,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(lane).is_none() {
            return;
        }
        let Some(root) = self.project_for(lane.project).map(|p| p.root.clone()) else {
            return;
        };
        let dir = super::flow_paths::project_flows_dir(&self.data_dir, &root);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.report_flow_file_error(s::flow::create_failed_title(), &dir, &e, cx);
            return;
        }
        let path = dir.join(format!("flow-{}.yaml", uuid::Uuid::new_v4()));
        let Some(agent) = self.mirrors.agents.first() else {
            self.report_flow_no_agent(cx);
            return;
        };
        let starter = match starter_flow(&agent.id, name, &s::flow::starter_prompt()) {
            Ok(text) => text,
            Err(error) => {
                self.report_own_flow_refusal(error.to_string(), "flow.create_serialize", cx);
                return;
            }
        };
        if let Err(e) = write_new_file(&path, &starter) {
            self.report_flow_file_error(s::flow::create_failed_title(), &path, &e, cx);
            return;
        }
        self.invalidate_flow_list();
        // The write above may have created the directory itself, which the
        // watcher can only anchor on once it exists.
        self.respawn_flow_watcher(cx);
        self.open_browsed_flow(lane, &path, window, cx);
        if let Some(pane) = self.find_flow_graph_pane(&path)
            && let Some((_, view)) = self.flow_graph_of_pane(pane)
        {
            view.update(cx, |view, cx| {
                view.select_node_after_add(&daruda_flow::NodeId::from("first"), window, cx);
                view.focus_name(window, cx);
            });
        }
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
        window: &mut Window,
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
        // `create_flow_in`'s — `rename(2)` replaces its destination by definition,
        // so a file arriving at the new name in between is overwritten. Closing
        // it needs `renamex_np` / `renameat2`, an unsafe FFI pair for two
        // platforms; deferred until something makes that worth carrying.
        if let Err(e) = std::fs::rename(from, &to) {
            self.report_flow_file_error(s::flow::rename_failed_title(), from, &e, cx);
            return;
        }
        self.repoint_flow_graph_panes(from, &to, window, cx);
        self.invalidate_flow_list();
        cx.notify();
    }

    /// Delete a flow file. The caller is responsible for having asked first.
    pub(in crate::workspace) fn delete_flow(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Err(e) = std::fs::remove_file(path) {
            self.report_flow_file_error(s::flow::delete_failed_title(), path, &e, cx);
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
        self.flows.list.invalidate();
    }

    fn report_flow_no_agent(&mut self, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(s::flow::create_failed_title())
                .severity(ErrorSeverity::Warning)
                .message(s::flow::no_agent())
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

        let edits = daruda_flow_edit::edits_for_update(base, update)
            .map_err(|err| EditRefusal::Unsupported(err.to_string()))?;
        if edits.is_empty() {
            return Err(EditRefusal::NothingToDo);
        }
        let candidate = daruda_flow_edit::apply(base, &edits);
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
            ErrorReport::new(s::flow::edit_refused_title())
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
            FlowNameError::Empty => s::flow::name_empty(),
            FlowNameError::HasSeparator => s::flow::name_has_separator(),
            FlowNameError::Unportable => s::flow::name_unportable(),
            FlowNameError::Taken => s::flow::name_taken(),
        };
        self.report_error(
            ErrorReport::new(s::flow::name_refused_title())
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
                .message(s::flow::file_op_failed(
                    path.display().to_string(),
                    error.to_string(),
                ))
                .dedup("flow.file_op")
                .at(file!(), line!())
                .build(),
            cx,
        );
    }

    /// Where the active lane's flows come from. Resolved in one place so the
    /// picker, the panel and the shot scenarios cannot disagree about what
    /// this lane can run.
    pub(in crate::workspace) fn flow_sources(&self) -> Option<super::flow_paths::FlowSources> {
        self.flow_sources_for(self.active)
    }

    /// The same, for a worktree the caller named rather than the one on
    /// screen.
    ///
    /// The project scope follows `target`'s own owner, not the active
    /// project: a run in another worktree must not pick up this one's
    /// project-scoped flows.
    pub(in crate::workspace) fn flow_sources_for(
        &self,
        target: daruda_store::project::LaneRef,
    ) -> Option<super::flow_paths::FlowSources> {
        Some(super::flow_paths::FlowSources {
            lane: self.lane_for(target)?.path.clone(),
            project: self
                .project_for(target.project)
                .map(|p| super::flow_paths::project_flows_dir(&self.data_dir, &p.root)),
            global: super::flow_paths::global_flows_dir(&self.data_dir),
        })
    }

    /// The browsed worktree's files, read only while Flows is visible.
    /// Scope changes, file operations, and watcher events invalidate the cache.
    pub(in crate::workspace) fn flow_list_for_panel(
        &mut self,
    ) -> Vec<super::flow_paths::FoundFlow> {
        if self.active_page() != Some(super::pages::Page::Flows) {
            return Vec::new();
        }
        let lane = self.flow_browser_lane();
        if self.lane_for(lane).is_none() {
            return Vec::new();
        }
        if self.flows.list.get(lane).is_none() {
            let Some(sources) = self.flow_sources_for(lane) else {
                return Vec::new();
            };
            self.flows.list.put(
                lane,
                super::flow_browser::listing::FlowListing::read(&sources),
            );
        }
        self.flows
            .list
            .get(lane)
            .map(|listing| listing.files.clone())
            .unwrap_or_default()
    }

    /// Disable Run for files with unsaved inspector edits in any worktree.
    /// Project and global files can be open in multiple lanes. This scan runs
    /// only while Flows is visible, before the view receives its snapshot.
    pub(in crate::workspace) fn flows_with_unsaved_edits(
        &self,
        cx: &gpui::App,
    ) -> Vec<std::path::PathBuf> {
        if self.active_page() != Some(super::pages::Page::Flows) {
            return Vec::new();
        }
        self.main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .filter_map(|pane| pane.flow_graph_content())
            .filter(|fg| fg.view.read(cx).has_unsaved_form(cx))
            .map(|fg| fg.path.clone())
            .collect()
    }
}

/// Ask, then delete on yes. One funnel so the screenshot scenario opens the
/// dialog a person actually gets rather than a second copy of it.
pub(in crate::workspace) fn ask_before_deleting(
    path: PathBuf,
    name: &str,
    origin: super::flow_paths::FlowOrigin,
    ws: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    crate::workspace::dialog_helpers::open_confirm_dialog(
        s::flow::delete_confirm_title(),
        super::flow_paths::delete_confirm_body(name, origin),
        s::flow::delete_confirm_ok(),
        crate::ui::dialog::ButtonVariant::Danger,
        move |_, _window, app| {
            let path = path.clone();
            if let Some(ws) = ws.upgrade() {
                ws.update(app, |ws, cx| ws.delete_flow(&path, cx));
            }
        },
        window,
        cx,
    );
}

/// Why the engine refused the candidate text, in words a person can act on.
///
/// `FlowError::Validate`'s `Display` is a count, so the issues are spelled out
/// here through the same helper the graph pane uses — otherwise a refused save
/// says "1 validation problem(s)" and nothing about which one.
fn load_failure_detail(error: &daruda_flow::FlowError) -> String {
    match error {
        daruda_flow::FlowError::Validate(issues) => s::flow::issue_lines(issues).join(" · "),
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

fn starter_flow(agent: &str, name: &str, prompt: &str) -> Result<String, yaml_serde::Error> {
    use daruda_flow::parse::{
        AgentOverride, Defaults, FlowFile, NodeFile, NodeKindFile, PromptSource,
    };
    yaml_serde::to_string(&FlowFile {
        version: 1,
        name: Some(name.to_owned()),
        defaults: Defaults {
            agent: Some(AgentOverride {
                id: Some(agent.to_owned()),
                mode: Some("bypassPermissions".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
        profiles: Default::default(),
        nodes: vec![NodeFile {
            id: "first".into(),
            deps: Vec::new(),
            timeout: None,
            cwd: None,
            kind: NodeKindFile::Agent {
                agent: None,
                prompt: PromptSource::Prompt(prompt.to_owned()),
                output: "first.md".into(),
                output_schema: None,
                continue_until: None,
                max_turns: None,
                on_fail: Default::default(),
            },
        }],
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

    #[test]
    fn starter_preserves_names_and_multiline_prompts_as_data() {
        let name = "Review / build: \"release\"";
        let text = starter_flow("agent: custom", name, "first\nsecond\n").unwrap();
        let file = daruda_flow::parse::parse_flow_file(&text).unwrap();
        assert_eq!(file.name.as_deref(), Some(name));
        assert!(text.contains("second"));
        daruda_flow::load(&text, None).expect("the starter is runnable");
    }
}
