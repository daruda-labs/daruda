use super::*;
use gpui::TestAppContext;

#[gpui::test]
async fn modal_safety_skill_loading_blocks_save_and_preserves_source(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sample");
    std::fs::create_dir(&dir).unwrap();
    let source = "---\nname: sample\ncontext: fork\nagent: reviewer\neffort: high\nshell: bash\npaths: [src/**]\n---\n\nBody without final newline";
    std::fs::write(dir.join("SKILL.md"), source).unwrap();
    let skill = crate::agent::skills::scan::scan_scope(root.path(), SkillScope::Project).remove(0);
    let modal = cx
        .update_window(window.into(), |_, window, cx| {
            let modal = cx.new(|cx| {
                EditSkillModal::new(
                    workspace.downgrade(),
                    skill,
                    SkillsSnapshot::default(),
                    window,
                    cx,
                )
            });
            modal.update(cx, |modal, cx| {
                assert!(!modal.body_loaded);
                assert!(modal.body_editor.read(cx).is_disabled());
                modal.submit(window, cx);
                assert!(!modal.submitting);
            });
            modal
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, _, cx| {
        modal.update(cx, |modal, cx| {
            assert!(modal.body_loaded);
            assert!(!modal.form.is_dirty(&modal.values(cx)));
            let draft = modal.build_draft(cx);
            assert_eq!(draft.frontmatter.context.as_deref(), Some("fork"));
            assert_eq!(draft.frontmatter.agent.as_deref(), Some("reviewer"));
            let merged = crate::agent::skills::frontmatter::render_edited_skill_md(
                source,
                &modal.original_frontmatter,
                &draft.frontmatter,
                &draft.body,
            )
            .unwrap();
            assert_eq!(merged, source);
        });
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("SKILL.md")).unwrap(),
        source
    );
}

#[gpui::test]
async fn modal_safety_failed_skill_read_cannot_be_saved(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("missing");
    let skill = Skill {
        name: "missing".into(),
        dir: dir.clone(),
        scope: SkillScope::Project,
        frontmatter: SkillFrontmatter::empty(),
        body_preview: String::new(),
        aux_file_count: 0,
        modified_at: std::time::SystemTime::now(),
        plugin_id: None,
        plugin_availability: None,
    };
    let modal = cx
        .update_window(window.into(), |_, window, cx| {
            cx.new(|cx| {
                EditSkillModal::new(
                    workspace.downgrade(),
                    skill,
                    SkillsSnapshot::default(),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        modal.update(cx, |modal, cx| {
            assert!(!modal.body_loaded);
            assert!(modal.error.is_some());
            modal.submit(window, cx);
            assert!(!modal.submitting);
        })
    })
    .unwrap();
    assert!(!dir.join("SKILL.md").exists());
}
