fn make_test_view(
    cx: &mut gpui::TestAppContext,
) -> gpui::WindowHandle<daruda_chat::view::AgentChatView> {
    daruda_ui::test_support::init_gpui_component(cx);
    cx.add_window(|window, cx| {
        daruda_chat::view::AgentChatView::new(
            0,
            window.window_handle(),
            None,
            daruda_chat::view::AgentSessionStatus::Idle,
            None,
            daruda_chat::view::AgentIdentity {
                id: "claude".to_string(),
                name: "Claude".to_string(),
                icon: None,
            },
            None,
            daruda_chat::transcript_defaults::TranscriptDefaults::resolve(
                None,
                daruda_chat::transcript_defaults::ReaderDefaults::from_config(
                    &daruda_config::AgentConfig::default(),
                ),
            ),
            daruda_config::file_viewer::DEFAULT_SYNTAX_THEME.to_owned(),
            std::rc::Rc::new(|_, _, _| Ok(())),
            cx,
        )
    })
}

fn connected_view(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::WindowHandle<daruda_chat::view::AgentChatView>,
    daruda_acp::HandleProbe,
) {
    let window = make_test_view(cx);
    let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
    window
        .update(cx, |view, _window, _cx| {
            view.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
            view.attach_handle(handle);
        })
        .unwrap();
    (window, probe)
}

#[gpui::test]
fn refused_solicited_internal_notice_preserves_the_previous_phone_turn(
    cx: &mut gpui::TestAppContext,
) {
    for refusal in [
        daruda_chat::view::InternalDispatch::NotNow,
        daruda_chat::view::InternalDispatch::ReadOnly,
        daruda_chat::view::InternalDispatch::Unavailable,
    ] {
        let (window, probe) = connected_view(cx);
        if refusal == daruda_chat::view::InternalDispatch::Unavailable {
            drop(probe);
        }
        window
            .update(cx, |view, _window, cx| {
                view.start_phone_turn_for_test(std::time::Instant::now());
                let previous = view.phone_turn().cloned();
                match refusal {
                    daruda_chat::view::InternalDispatch::NotNow => view.set_turn_in_flight(),
                    daruda_chat::view::InternalDispatch::ReadOnly => {
                        view.set_access(daruda_store::tasks::AgentChatAccess::CliSnapshot(
                            daruda_store::tasks::ExecutionRef {
                                task_id: "task".into(),
                                execution_id: "execution".into(),
                            },
                        ))
                    }
                    _ => {}
                }
                assert_eq!(
                    view.try_send_internal_prompt_text_with_phone_mode(
                        "notice".into(),
                        Some(daruda_chat::phone_turn::PhoneTurnMode::CompletionOnly),
                        cx
                    ),
                    refusal
                );
                assert_eq!(view.phone_turn(), previous.as_ref());
            })
            .unwrap();
    }
}

#[gpui::test]
fn internal_notice_refuses_while_a_turn_is_in_flight(cx: &mut gpui::TestAppContext) {
    let (window, mut probe) = connected_view(cx);
    window
        .update(cx, |view, _window, cx| {
            view.set_turn_in_flight();
            assert_eq!(
                view.try_send_internal_prompt_text("notice".into(), cx),
                daruda_chat::view::InternalDispatch::NotNow
            );
            assert_eq!(view.queued_prompt_count(), 0, "a refusal never queues");
            assert_eq!(probe.drain().0, 0);
        })
        .unwrap();
}
