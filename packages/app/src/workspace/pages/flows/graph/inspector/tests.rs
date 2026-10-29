//! Transition contracts run without constructing an app, window or entity.
use super::model::SaveBlock;
use super::*;
use daruda_flow::{
    NodeId,
    error::{ValidationIssue, ValidationKind},
};

const FLOW: &str = "version: 1\nnodes:\n  - id: design\n    kind: agent\n    prompt: write\n    output: design.md\n  - id: build\n    deps: [design]\n    kind: command\n    run: echo ok\n";

fn model() -> InspectorModel {
    let mut model = InspectorModel::new(Some(FLOW.into()), Vec::new());
    model.select(Some("design".into()));
    model.assert_invariants();
    model
}

fn edit(model: &mut InspectorModel, change: impl FnOnce(&mut RawInputs)) {
    let draft = model.draft().unwrap();
    let id = draft.node().clone();
    let mut raw = draft.raw().clone();
    let ui = draft.ui().clone();
    change(&mut raw);
    model.edit(&id, raw, ui);
    model.assert_invariants();
}

#[test]
fn selecting_a_node_creates_one_owned_draft_and_keeps_input_across_switches() {
    let mut model = model();
    assert!(!model.status().unsaved);
    assert!(!model.select(Some("design".into())).rebind);
    edit(&mut model, |raw| raw.output = "draft.md".into());
    assert!(model.select(Some("build".into())).rebind);
    assert!(model.select(None).rebind);
    model.select(Some("design".into()));
    assert_eq!(model.draft().unwrap().raw().output, "draft.md");
    assert_eq!(model.pending_fields().len(), 1);
    model.assert_invariants();
}

#[test]
fn unrelated_reload_moves_the_baseline_and_keeps_the_binding() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    let effects = model.reload(
        Some(FLOW.replace("echo ok", "echo updated").into()),
        Vec::new(),
    );
    assert!(!effects.rebind);
    assert!(effects.refresh_links);
    assert!(!model.status().conflict);
    assert_eq!(model.draft().unwrap().raw().output, "draft.md");
    assert!(model.draft().unwrap().source().contains("echo updated"));
    model.assert_invariants();
}

#[test]
fn a_changed_node_conflicts_until_its_original_revision_returns() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    assert!(
        !model
            .reload(
                Some(FLOW.replace("prompt: write", "prompt: theirs").into()),
                Vec::new()
            )
            .rebind
    );
    assert!(matches!(model.notice(), Some(Notice::Conflict)));
    assert!(matches!(model.save_plan(), Err(SaveBlock::Conflict)));
    edit(&mut model, |raw| raw.output = "design.md".into());
    assert!(model.status().node_revertable);
    model.reload(Some(FLOW.into()), Vec::new());
    assert!(!model.status().conflict);
    assert!(model.notice().is_none());
    model.assert_invariants();
}

#[test]
fn untouched_changed_nodes_are_rebuilt_without_conflict() {
    let mut model = model();
    assert!(
        model
            .reload(
                Some(FLOW.replace("prompt: write", "prompt: theirs").into()),
                Vec::new()
            )
            .rebind
    );
    assert_eq!(model.draft().unwrap().raw().prompt_inline, "theirs");
    assert!(!model.status().unsaved);
    assert!(!model.status().conflict);
    model.assert_invariants();
}

#[test]
fn deleted_drafts_are_named_and_never_block_save() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    let only_build = "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n";
    assert!(model.reload(Some(only_build.into()), Vec::new()).rebind);
    assert!(model.selected().is_none());
    assert!(
        matches!(model.notice(), Some(Notice::Discarded(ids)) if ids == &[NodeId::from("design")])
    );
    assert!(model.save_plan().unwrap().nodes.is_empty());
    model.assert_invariants();
}

#[test]
fn file_loss_and_parse_failure_remember_input_and_restore_selection() {
    for unreadable in [None, Some("nodes: [".into())] {
        let mut model = model();
        edit(&mut model, |raw| raw.output = "draft.md".into());
        assert!(model.reload(unreadable, Vec::new()).rebind);
        assert!(model.selected().is_none());
        assert_eq!(model.remembered(), Some(&"design".into()));
        let effects = model.reload(Some(FLOW.into()), Vec::new());
        assert!(effects.rebind);
        assert_eq!(effects.select_on_canvas, Some("design".into()));
        assert_eq!(model.draft().unwrap().raw().output, "draft.md");
        model.assert_invariants();
    }
}

#[test]
fn canvas_edits_merge_edge_deltas_and_clear_deleted_references_in_hidden_drafts() {
    let mut model = model();
    model.select(Some("build".into()));
    edit(&mut model, |raw| {
        raw.run = "echo draft".into();
        raw.deps = "design, typed".into();
        raw.rerun = "design, typed".into();
    });
    model.select(Some("design".into()));
    let next = FLOW.replace("deps: [design]", "deps: []");
    model.reload(Some(next.into()), Vec::new());
    model.canvas_edit(&[], FLOW);
    model.select(Some("build".into()));
    assert_eq!(model.draft().unwrap().raw().deps, "typed");
    assert_eq!(model.draft().unwrap().raw().run, "echo draft");
    assert!(!model.status().conflict);
    let before = model.draft().unwrap().source().to_string();
    let only_build = "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n";
    model.reload(Some(only_build.into()), Vec::new());
    model.canvas_edit(&["design".into()], &before);
    assert_eq!(model.draft().unwrap().raw().rerun, "typed");
    assert!(model.notice().is_none());
    model.assert_invariants();
}

#[test]
fn an_external_conflict_is_not_rebased_by_a_later_canvas_edit() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    let external = FLOW.replace("prompt: write", "prompt: theirs");
    model.reload(Some(external.clone().into()), Vec::new());
    model.reload(
        Some(external.replace("deps: [design]", "deps: []").into()),
        Vec::new(),
    );
    model.canvas_edit(&[], &external);
    assert!(model.status().conflict);
    assert_eq!(model.draft().unwrap().raw().output, "draft.md");
    model.assert_invariants();
}

#[test]
fn node_revert_keeps_other_drafts_and_all_revert_clears_them() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    model.select(Some("build".into()));
    edit(&mut model, |raw| raw.run = "echo draft".into());
    assert!(model.revert_node().rebind);
    assert_eq!(model.draft().unwrap().raw().run, "echo ok");
    assert_eq!(model.pending_fields().len(), 1);
    model.revert_all();
    assert!(model.pending_fields().is_empty());
    assert!(!model.status().unsaved);
    model.assert_invariants();
}

#[test]
fn reverting_the_conflicting_node_retracts_its_notice() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    model.reload(
        Some(FLOW.replace("prompt: write", "prompt: theirs").into()),
        Vec::new(),
    );
    model.revert_node();
    assert!(model.notice().is_none());
    assert!(!model.status().conflict);
    assert_eq!(model.draft().unwrap().raw().prompt_inline, "theirs");
    model.assert_invariants();
}

#[test]
fn an_existing_refusal_and_inactive_input_do_not_block_another_nodes_save() {
    let empty = FLOW.replace("run: echo ok", "run: ''");
    let mut model = InspectorModel::new(Some(empty.into()), Vec::new());
    model.select(Some("build".into()));
    edit(&mut model, |raw| {
        raw.prompt_inline = "inactive".into();
        raw.max_turns = "oops".into();
    });
    assert!(model.save_plan().unwrap().nodes.is_empty());
    model.select(Some("design".into()));
    edit(&mut model, |raw| raw.output = "draft.md".into());
    assert_eq!(model.save_plan().unwrap().nodes.len(), 1);
    model.assert_invariants();
}

#[test]
fn newly_invalid_input_is_the_only_local_refusal_and_clears_when_fixed() {
    let mut model = model();
    edit(&mut model, |raw| raw.timeout = "soon".into());
    assert!(
        matches!(model.save_plan(), Err(SaveBlock::Invalid(id, fields::Refusal::Timeout(_))) if id == "design")
    );
    model.set_notice(Some(Notice::InvalidDraft("design".into())));
    edit(&mut model, |raw| raw.timeout.clear());
    assert!(model.notice().is_none());
    assert!(model.save_plan().is_ok());
    model.assert_invariants();
}

#[test]
fn a_refused_rename_uses_both_file_and_edited_identity() {
    for issue_id in ["design", "renamed"] {
        let mut model = model();
        edit(&mut model, |raw| raw.id = "renamed".into());
        model.select(Some("build".into()));
        let issues = [ValidationIssue {
            node: Some(issue_id.into()),
            kind: ValidationKind::DuplicateOutput,
            message: "duplicate".into(),
        }];
        let effects = model.save_refused("refused".into(), &issues);
        assert!(effects.rebind);
        assert_eq!(effects.select_on_canvas, Some("design".into()));
        assert_eq!(model.selected(), Some(&"design".into()));
        assert_eq!(model.draft().unwrap().raw().id, "renamed");
        model.assert_invariants();
    }
}

#[test]
fn successful_save_resets_drafts_and_reselects_the_written_id() {
    let mut model = model();
    edit(&mut model, |raw| raw.id = "renamed".into());
    model.reload(Some(FLOW.replace("design", "renamed").into()), Vec::new());
    let effects = model.save_succeeded(Some("renamed".into()));
    assert!(effects.rebind);
    assert_eq!(effects.select_on_canvas, Some("renamed".into()));
    assert!(!model.status().unsaved);
    assert!(model.notice().is_none());
    assert_eq!(model.draft().unwrap().raw().id, "renamed");
    model.assert_invariants();
}

#[test]
fn delayed_prompts_update_hidden_drafts_and_reject_second_or_conflicting_writes() {
    for field in [PromptField::Inline, PromptField::File] {
        let mut model = model();
        let token = model.begin_prompt(field).unwrap();
        model.reload(Some(format!("{FLOW}# comment\n").into()), Vec::new());
        model.select(Some("build".into()));
        assert!(model.apply_prompt(&token, "draft".into()));
        assert!(!model.apply_prompt(&token, "second".into()));
        model.select(Some("design".into()));
        let raw = model.draft().unwrap().raw();
        assert_eq!(
            match field {
                PromptField::Inline => &raw.prompt_inline,
                PromptField::File => &raw.prompt_file,
            },
            "draft"
        );
        let token = model.begin_prompt(field).unwrap();
        model.reload(
            Some(FLOW.replace("prompt: write", "prompt: theirs").into()),
            Vec::new(),
        );
        assert!(!model.apply_prompt(&token, "stale".into()));
        model.assert_invariants();
    }
}

#[test]
fn custom_modes_and_inactive_raw_values_survive_selection_and_canvas_rebase() {
    let mut model = model();
    let draft = model.draft().unwrap();
    let mut raw = draft.raw().clone();
    raw.prompt_file = "inactive.md".into();
    let mut ui = draft.ui().clone();
    ui.timeout_custom = true;
    ui.agent_model_custom = true;
    model.edit(&"design".into(), raw, ui);
    model.select(Some("build".into()));
    model.reload(
        Some(FLOW.replace("deps: [design]", "deps: []").into()),
        Vec::new(),
    );
    model.canvas_edit(&[], FLOW);
    model.select(Some("design".into()));
    let draft = model.draft().unwrap();
    assert_eq!(draft.raw().prompt_file, "inactive.md");
    assert!(draft.ui().timeout_custom && draft.ui().agent_model_custom);
    assert!(model.pending_fields().is_empty());
    model.assert_invariants();
}

#[test]
fn independent_failures_are_not_cleared_by_an_unrelated_reload() {
    let mut model = model();
    model.set_notice(Some(Notice::Io("disk full".into())));
    model.reload(Some(format!("{FLOW}# comment\n").into()), Vec::new());
    assert!(matches!(model.notice(), Some(Notice::Io(message)) if message == "disk full"));
    model.title_edited();
    assert!(model.notice().is_none());
    model.assert_invariants();
}

#[test]
fn event_sequences_preserve_invariants_after_each_transition() {
    enum Event {
        Edit,
        Switch,
        Reload,
        Canvas,
        Lose,
        Restore,
        RevertNode,
        RevertAll,
    }
    use Event::*;
    let sequences = [
        vec![Edit, Reload, Canvas, Switch, RevertNode, RevertAll],
        vec![Edit, Lose, Restore, Switch, Edit, Reload, RevertAll],
        vec![Switch, Edit, Reload, Canvas, Lose, Restore, RevertNode],
    ];
    for sequence in sequences {
        let mut model = model();
        let mut current = FLOW.to_owned();
        for event in sequence {
            match event {
                Edit => edit(&mut model, |raw| raw.cwd = "subdir".into()),
                Switch => {
                    let id = if model.selected() == Some(&"design".into()) {
                        "build"
                    } else {
                        "design"
                    };
                    model.select(Some(id.into()));
                }
                Reload => {
                    current.push_str("# reload\n");
                    model.reload(Some(current.clone().into()), Vec::new());
                }
                Canvas => {
                    let before = current.clone();
                    current = current.replace("deps: [design]", "deps: []");
                    model.reload(Some(current.clone().into()), Vec::new());
                    model.canvas_edit(&[], &before);
                }
                Lose => {
                    model.file_lost();
                }
                Restore => {
                    model.reload(Some(current.clone().into()), Vec::new());
                }
                RevertNode => {
                    model.revert_node();
                }
                RevertAll => {
                    model.revert_all();
                }
            }
            model.assert_invariants();
        }
    }
}

#[test]
fn a_lost_clean_file_has_no_draft_conflict_and_revert_keeps_remembered_selection() {
    let mut model = model();
    model.select(Some("build".into()));
    model.file_lost();
    assert!(!model.status().conflict);
    model.revert_all();
    assert_eq!(model.remembered(), Some(&"build".into()));
    model.reload(Some(FLOW.into()), Vec::new());
    assert_eq!(model.selected(), Some(&"build".into()));
    model.assert_invariants();
}

#[test]
fn an_unrelated_canvas_edit_preserves_raw_dependency_spacing() {
    let mut model = model();
    model.select(Some("build".into()));
    edit(&mut model, |raw| raw.deps = " design, ".into());
    model.reload(Some(format!("{FLOW}# edit\n").into()), Vec::new());
    model.canvas_edit(&["other".into()], FLOW);
    assert_eq!(model.draft().unwrap().raw().deps, " design, ");
    model.assert_invariants();
}

#[test]
fn inactive_input_can_be_reverted_without_becoming_a_save_mutation() {
    let mut model = model();
    edit(&mut model, |raw| raw.prompt_file = "draft.md".into());
    assert!(model.status().unsaved);
    assert!(model.status().node_revertable);
    assert!(model.save_plan().unwrap().nodes.is_empty());
    model.revert_node();
    assert!(!model.status().unsaved);
    assert!(!model.status().node_revertable);
    assert!(model.draft().unwrap().raw().prompt_file.is_empty());
    model.assert_invariants();
}

#[test]
fn discarded_history_returns_after_conflict_and_survives_node_revert_and_io_failure() {
    let mut model = model();
    edit(&mut model, |raw| raw.output = "draft.md".into());
    model.select(Some("build".into()));
    edit(&mut model, |raw| raw.run = "echo draft".into());
    let next = "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo theirs\n";
    model.reload(Some(next.into()), Vec::new());
    assert!(matches!(model.notice(), Some(Notice::Conflict)));
    model.revert_node();
    assert!(
        matches!(model.notice(), Some(Notice::Discarded(ids)) if ids == &[NodeId::from("design")])
    );
    model.set_notice(Some(Notice::Io("disk full".into())));
    model.set_notice(None);
    assert!(
        matches!(model.notice(), Some(Notice::Discarded(ids)) if ids == &[NodeId::from("design")])
    );
    model.reload(Some(format!("{next}# reload\n").into()), Vec::new());
    model.revert_node();
    assert!(matches!(model.notice(), Some(Notice::Discarded(_))));
    model.dismiss_discarded();
    assert!(model.notice().is_none());
    model.reload(Some(next.into()), Vec::new());
    assert!(model.notice().is_none());
    model.assert_invariants();
}

#[test]
fn successful_save_and_full_revert_clear_discarded_history() {
    for save in [false, true] {
        let mut model = model();
        edit(&mut model, |raw| raw.output = "draft.md".into());
        model.select(Some("build".into()));
        model.reload(
            Some("version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n".into()),
            Vec::new(),
        );
        assert!(matches!(model.notice(), Some(Notice::Discarded(_))));
        if save {
            model.save_succeeded(Some("build".into()));
        } else {
            model.revert_all();
        }
        model.set_notice(None);
        assert!(model.notice().is_none());
        model.assert_invariants();
    }
}

#[test]
fn delayed_prompts_restore_clean_hidden_drafts_after_file_recovery() {
    for field in [PromptField::Inline, PromptField::File] {
        let mut model = model();
        let token = model.begin_prompt(field).unwrap();
        model.select(Some("build".into()));
        model.file_lost();
        assert!(!model.apply_prompt(&token, "unavailable".into()));
        model.reload(Some(FLOW.into()), Vec::new());
        assert!(model.apply_prompt(&token, "recovered".into()));
        assert_eq!(model.selected(), Some(&NodeId::from("build")));
        assert!(!model.apply_prompt(&token, "stale".into()));
        model.select(Some("design".into()));
        let raw = model.draft().unwrap().raw();
        assert_eq!(
            match field {
                PromptField::Inline => &raw.prompt_inline,
                PromptField::File => &raw.prompt_file,
            },
            "recovered"
        );
        model.assert_invariants();
    }
}

#[test]
fn restoring_a_missing_prompt_draft_still_checks_the_expected_field_and_revision() {
    for changed in [
        FLOW.replace("prompt: write", "prompt: theirs"),
        FLOW.replace("design.md", "theirs.md"),
        FLOW.replace("id: design", "id: other"),
    ] {
        let mut model = model();
        let token = model.begin_prompt(PromptField::Inline).unwrap();
        model.select(Some("build".into()));
        model.file_lost();
        model.reload(Some(changed.into()), Vec::new());
        assert!(!model.apply_prompt(&token, "stale".into()));
        model.assert_invariants();
    }
}

#[test]
fn a_refused_rename_pins_notes_and_banner_by_either_identity() {
    for issue_id in ["design", "renamed"] {
        let mut model = model();
        edit(&mut model, |raw| raw.id = "renamed".into());
        let issues = [ValidationIssue {
            node: Some(issue_id.into()),
            kind: ValidationKind::DuplicateOutput,
            message: "duplicate".into(),
        }];
        model.save_refused("refused".into(), &issues);
        let chrome = model.chrome();
        assert_eq!(chrome.banner.as_deref(), Some("refused"));
        assert!(!chrome.notes.is_empty(), "issue named {issue_id}");
        model.assert_invariants();
    }
}

#[test]
fn standing_issues_open_a_fresh_draft_but_not_one_the_person_closed() {
    let issues = vec![ValidationIssue {
        node: Some("design".into()),
        kind: ValidationKind::AgentIdWithoutMode,
        message: "mode".into(),
    }];
    let mut model = InspectorModel::new(Some(FLOW.into()), issues);
    model.select(Some("design".into()));
    assert!(model.chrome().agent_open);
    model.toggle_agent_section();
    model.select(Some("build".into()));
    model.select(Some("design".into()));
    assert!(!model.chrome().agent_open);
    model.assert_invariants();
}

#[test]
fn an_unreadable_draft_value_selects_its_node_and_opens_settings() {
    let mut model = model();
    edit(&mut model, |raw| raw.timeout = "soon".into());
    model.select(Some("build".into()));
    let block = model.save_plan().err().expect("an invalid draft blocks");
    let effects = model.save_blocked(block);
    assert_eq!(effects.select_on_canvas, Some("design".into()));
    assert!(model.chrome().settings_open);
    assert!(matches!(model.notice(), Some(Notice::InvalidDraft(_))));
    model.assert_invariants();
}

#[test]
fn deleting_the_selected_node_on_the_canvas_unbinds_it() {
    let mut model = model();
    model.select(Some("build".into()));
    let effects = model.canvas_edit(&["build".into()], FLOW);
    assert!(effects.rebind);
    assert_eq!(model.selected(), None);
    model.assert_invariants();
}

#[test]
fn the_model_owns_the_pane_text() {
    let mut model = InspectorModel::new(Some(FLOW.into()), Vec::new());
    assert_eq!(model.text().map(AsRef::as_ref), Some(FLOW));
    model.file_lost();
    assert!(model.text().is_none());
}

fn refuse(model: &mut InspectorModel, node: &str) {
    let issues = [ValidationIssue {
        node: Some(node.into()),
        kind: ValidationKind::DuplicateOutput,
        message: "duplicate".into(),
    }];
    model.save_refused("refused".into(), &issues);
}

#[test]
fn a_refusal_stays_on_its_node_and_others_show_their_standing_notes() {
    let issues = vec![ValidationIssue {
        node: Some("build".into()),
        kind: ValidationKind::UnknownDep {
            dep: "ghost".into(),
        },
        message: "dep".into(),
    }];
    let mut model = InspectorModel::new(Some(FLOW.into()), issues);
    model.select(Some("design".into()));
    edit(&mut model, |raw| raw.output = "mine.md".into());
    refuse(&mut model, "design");
    assert!(model.chrome().banner.is_some());
    model.select(Some("build".into()));
    let chrome = model.chrome();
    assert_eq!(chrome.banner, None);
    assert!(!chrome.notes.is_empty(), "build keeps its standing note");
    model.select(Some("design".into()));
    assert!(model.chrome().banner.is_some());
    model.assert_invariants();
}

#[test]
fn reverting_a_refused_node_does_not_reopen_sections_for_the_refusal() {
    let mut model = model();
    edit(&mut model, |raw| raw.agent_id = "custom".into());
    assert!(!model.chrome().agent_open);
    let issues = [ValidationIssue {
        node: Some("design".into()),
        kind: ValidationKind::AgentIdWithoutMode,
        message: "mode".into(),
    }];
    model.save_refused("refused".into(), &issues);
    assert!(model.chrome().agent_open);
    model.revert_node();
    assert!(!model.chrome().agent_open);
    assert_eq!(model.notice(), None);
    model.assert_invariants();
}

#[test]
fn an_edit_flags_chrome_only_when_the_notice_changes() {
    let mut model = model();
    let draft = model.draft().unwrap();
    let (id, mut raw, ui) = (
        draft.node().clone(),
        draft.raw().clone(),
        draft.ui().clone(),
    );
    raw.timeout = "soon".into();
    assert!(!model.edit(&id, raw.clone(), ui.clone()).chrome);
    model.set_notice(Some(Notice::InvalidDraft(id.clone())));
    raw.timeout = String::new();
    assert!(
        model.edit(&id, raw, ui).chrome,
        "fixing the value retracts the notice"
    );
    model.assert_invariants();
}
