use super::super::{AgentSessionStatus, PromptDispatch};
use super::make_test_view;
use daruda_acp::{AttachmentContent, PromptAttachment};

fn attachment() -> PromptAttachment {
    PromptAttachment {
        name: "note.txt".into(),
        content: AttachmentContent::TextFile {
            text: "immutable queued contents".into(),
            uri: "urn:test:note".into(),
        },
    }
}

#[gpui::test]
fn attachments_survive_queue_edit_stop_and_resume(cx: &mut gpui::TestAppContext) {
    make_test_view(cx)
        .update(cx, |view, _window, cx| {
            view.status = AgentSessionStatus::Connected;
            view.session_capabilities.embedded_context = true;
            view.add_attachments(vec![attachment()], cx).unwrap();
            assert_eq!(
                view.send_composer_prompt("inspect".into(), cx).unwrap(),
                PromptDispatch::Queued
            );
            assert!(view.draft_attachments().is_empty());
            let id = view.queue.pending_prompts[0].id;
            view.begin_edit(id, cx);
            view.send_composer_prompt("inspect carefully".into(), cx)
                .unwrap();
            assert_eq!(
                view.queue.pending_prompts[0].attachments,
                vec![attachment()]
            );
            view.set_turn_in_flight();
            view.handle_escape(cx);
            assert_eq!(view.queue.paused_prompts[0].attachments, vec![attachment()]);
            view.resume_queue(cx);
            assert_eq!(
                view.queue.pending_prompts[0].attachments,
                vec![attachment()]
            );
            view.set_turn_idle();
            view.session_capabilities.embedded_context = false;
            assert!(view.drain_next_queued_prompt_for_test(cx).is_none());
            assert_eq!(
                view.queue.pending_prompts[0].attachments,
                vec![attachment()]
            );
        })
        .unwrap();
}

#[gpui::test]
fn attachment_refusal_preserves_the_draft_and_existing_queue(cx: &mut gpui::TestAppContext) {
    make_test_view(cx)
        .update(cx, |view, _window, cx| {
            view.status = AgentSessionStatus::Connected;
            assert!(view.add_attachments(vec![attachment()], cx).is_err());
            assert!(view.draft_attachments().is_empty());
            view.session_capabilities.embedded_context = true;
            view.add_attachments(vec![attachment()], cx).unwrap();
            view.fill_queue_for_test(crate::control::guards::QUEUE_DEPTH_MAX);
            assert_eq!(
                view.send_composer_prompt("inspect".into(), cx).unwrap(),
                PromptDispatch::QueueFull
            );
            assert_eq!(view.draft_attachments(), &[attachment()]);
            assert_eq!(
                view.queued_prompt_count(),
                crate::control::guards::QUEUE_DEPTH_MAX
            );
        })
        .unwrap();
}

#[gpui::test]
fn editing_a_full_attachment_prompt_cannot_bypass_the_limit(cx: &mut gpui::TestAppContext) {
    make_test_view(cx)
        .update(cx, |view, _window, cx| {
            view.status = AgentSessionStatus::Connected;
            view.session_capabilities.embedded_context = true;
            view.add_attachments(vec![attachment(); 8], cx).unwrap();
            view.send_composer_prompt("inspect".into(), cx).unwrap();
            view.begin_edit(view.queue.pending_prompts[0].id, cx);
            assert!(view.add_attachments(vec![attachment()], cx).is_err());
            assert!(view.draft_attachments().is_empty());
            assert_eq!(view.queue.pending_prompts[0].attachments.len(), 8);
        })
        .unwrap();
}

#[gpui::test]
fn editing_after_building_a_draft_cannot_exceed_the_prompt_limit(cx: &mut gpui::TestAppContext) {
    make_test_view(cx)
        .update(cx, |view, _window, cx| {
            view.status = AgentSessionStatus::Connected;
            view.session_capabilities.embedded_context = true;
            view.add_attachments(vec![attachment(); 8], cx).unwrap();
            view.send_composer_prompt("queued".into(), cx).unwrap();
            view.add_attachments(vec![attachment()], cx).unwrap();
            let id = view.queue.pending_prompts[0].id;
            view.begin_edit(id, cx);
            assert!(view.send_composer_prompt("edited".into(), cx).is_err());
            assert_eq!(view.draft_attachments(), &[attachment()]);
            assert_eq!(view.queue.pending_prompts[0].text, "queued");
            assert_eq!(view.queue.pending_prompts[0].attachments.len(), 8);
            assert_eq!(view.queue.editing_prompt, Some(id));
        })
        .unwrap();
}
