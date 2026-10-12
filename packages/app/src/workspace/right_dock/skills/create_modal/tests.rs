use super::*;
use gpui::{TestAppContext, VisualTestContext};

#[gpui::test]
async fn modal_safety_escape_and_cancel_share_draft_protection(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
    let built = holder.clone();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, cx| {
        crate::workspace::dialog_helpers::open_form_modal(
            "New skill",
            None,
            move |window, cx| {
                *built.borrow_mut() = Some(cx.entity());
                CreateSkillModal::new(
                    workspace.downgrade(),
                    None,
                    None,
                    SkillsSnapshot::default(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    let modal: Entity<CreateSkillModal> = holder.borrow().clone().unwrap();
    vcx.update(|window, cx| {
        modal.update(cx, |modal, cx| {
            modal
                .name_input
                .update(cx, |input, cx| input.set_value("draft", window, cx));
        })
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(
        vcx.has_pending_prompt(),
        "Escape must reach the guarded close policy"
    );
    vcx.simulate_prompt_answer(&strings::common::btn_cancel());
    vcx.run_until_parked();
    assert!(!modal.read_with(&vcx, |modal, _| modal.form.asking));
    assert_eq!(
        modal.read_with(&vcx, |modal, cx| modal
            .name_input
            .read(cx)
            .value()
            .to_string()),
        "draft"
    );
    vcx.update(|window, cx| {
        modal.update(cx, |modal, cx| {
            modal.submitting = true;
            modal.leave(window, cx);
            modal.submitting = false;
        })
    });
    assert!(
        !vcx.has_pending_prompt(),
        "a submitted draft cannot be dismissed"
    );
    vcx.update(|window, cx| modal.update(cx, |modal, cx| modal.leave(window, cx)));
    assert!(
        vcx.has_pending_prompt(),
        "Cancel uses the same guard as Escape"
    );
    vcx.simulate_prompt_answer(&strings::common::btn_cancel());
    vcx.run_until_parked();
}

#[gpui::test]
async fn quick_entry_skill_import_keeps_source_and_separate_invocation(cx: &mut TestAppContext) {
    let (window, workspace) = crate::workspace::tests::build_workspace(cx);
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window,cx| {
        let modal=cx.new(|cx|CreateSkillModal::new(workspace.downgrade(),None,None,SkillsSnapshot::default(),window,cx));
        modal.update(cx,|modal,cx| {
            let source="---\n# Keep this comment\nname: original-call\ndescription: '  keep spacing  '\nhooks:\n  custom: true\n---\n\n# Instructions\n\n";
            let imported=crate::agent::skills::import::parse(source).unwrap();
            modal.apply_import(imported,window,cx);
            modal.name_input.update(cx,|input,cx|input.set_value("new-folder",window,cx));
            let draft=modal.build_draft(cx).unwrap();
            assert_eq!(draft.name,"new-folder");
            assert_eq!(draft.frontmatter.name.as_deref(),Some("original-call"));
            assert_eq!(draft.frontmatter.description.as_deref(),Some("  keep spacing  "));
            let imported=modal.imported.as_ref().unwrap();
            let rendered=crate::agent::skills::frontmatter::render_edited_skill_md(&imported.source,
                &imported.frontmatter,&draft.frontmatter,&draft.body).unwrap();
            assert_eq!(rendered,source);
            assert!(modal.form.is_dirty(&modal.values(cx)));
            modal.seed_duplicate(source.into(),"original".into(),window,cx);
            let copy=modal.build_draft(cx).unwrap();
            assert_eq!(copy.name,"original-copy");
            assert_eq!(copy.frontmatter.name,None);
            assert!(copy.frontmatter.extra.contains_key("hooks"));
        });
    });
}

#[test]
fn quick_entry_create_never_clobbers_an_existing_skill() {
    let temp = tempfile::tempdir().unwrap();
    let draft = SkillDraft {
        name: "sample".into(),
        scope: SkillScope::Project,
        frontmatter: SkillFrontmatter::empty(),
        body: String::new(),
    };
    let path = persist::create_skill_source(&draft, Some(temp.path()), "first\n").unwrap();
    assert!(matches!(
        persist::create_skill_source(&draft, Some(temp.path()), "second"),
        Err(persist::PersistError::AlreadyExists(_))
    ));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "first\n");
}
