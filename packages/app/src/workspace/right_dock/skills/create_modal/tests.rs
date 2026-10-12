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
