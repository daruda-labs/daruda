use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionUpdate, TextContent};
use gpui::{TestAppContext, VisualTestContext, px};

use crate::view::tests::make_test_view;
use crate::view::{HostCard, HostCardAction};
use daruda_ui::theme;

#[gpui::test]
fn host_cards_reserve_space_below_the_body_and_cap_the_scroll_strip(cx: &mut TestAppContext) {
    let window = make_test_view(cx);
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("agent-chat-host-cards").is_none());
    let view = window.root(&mut visual).unwrap();
    view.update(&mut visual, |view, cx| {
        view.apply_event(
            daruda_acp::AcpEvent::Update(Box::new(SessionUpdate::AgentMessageChunk(
                ContentChunk::new(ContentBlock::Text(TextContent::new("Body"))),
            ))),
            "",
            false,
            cx,
        );
        for id in 0..10 {
            view.push_host_card(
                HostCard {
                    id,
                    summary: format!("Request {id}"),
                    actions: vec![HostCardAction {
                        id: "choose".into(),
                        label: "Choose".into(),
                    }],
                },
                cx,
            );
        }
    });
    visual.run_until_parked();
    let strip = visual
        .debug_bounds("agent-chat-host-cards")
        .expect("the strip paints");
    view.read_with(&visual, |view, cx| {
        let body = view.list_state.viewport_bounds();
        assert!(body.size.height > px(0.0));
        assert!(body.bottom() <= strip.top());
        let max_height = theme::agent_chat_font_size(cx)
            * theme::agent_chat_line_height(cx)
            * super::MAX_VISIBLE_TEXT_ROWS;
        assert!(
            strip.size.height <= px(max_height.ceil()),
            "strip {:?}, requested maximum {:?}",
            strip.size,
            px(max_height)
        );
        assert!(view.host_cards_scroll.max_offset().y > px(0.0));
    });

    view.update(&mut visual, |view, cx| {
        view.settle_host_card(0, "Handled on phone".into(), cx)
    });
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("agent-chat-host-card-action-0-0")
            .is_none()
    );
    assert!(
        visual
            .debug_bounds("agent-chat-host-card-result-0")
            .is_some()
    );
}
