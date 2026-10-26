use daruda_acp::ChatItem;
use daruda_chat::agent_chat_helpers::{activity_bar_title, has_conversation};

fn asst(text: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: text.to_owned(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

#[test]
fn has_conversation_distinguishes_empty_error_only_and_real_transcript() {
    assert!(!has_conversation(&[]));

    // The pane that hit a usage limit holds nothing but the notice — the
    // switch that follows has no transcript to protect.
    let items = [ChatItem::Failure(daruda_acp::AcpFailure::unclassified(
        "session limit reached",
    ))];
    assert!(!has_conversation(&items));

    for item in [
        ChatItem::UserText("hi".to_owned()),
        asst("hello"),
        ChatItem::Failure(daruda_acp::AcpFailure::unclassified("boom")),
    ]
    .windows(2)
    .map(|w| w.to_vec())
    {
        assert!(has_conversation(&item), "{item:?} holds a transcript");
    }
}

/// A *classified* failure must be excluded exactly like an unclassified one.
/// The classes are what an auth failure now arrives as, and they are the whole
/// reason a user reaches for another account — so a pane holding only these is
/// still empty and must reconnect in place rather than stranding the user with
/// a fresh pane every time a login expires.
#[test]
fn has_conversation_excludes_every_failure_class() {
    let classes = [
        daruda_acp::AcpFailure::classify(&agent_client_protocol::Error::new(
            -32000,
            "Authentication required",
        )),
        daruda_acp::AcpFailure::Categorized {
            kind: daruda_acp::FailureKind::OauthOrgNotAllowed,
            message: "org disabled".to_owned(),
        },
        daruda_acp::AcpFailure::Runtime {
            kind: daruda_acp::RuntimeKind::Download,
            message: "no network".to_owned(),
        },
        daruda_acp::AcpFailure::unclassified("stream ended"),
    ];
    for failure in classes {
        let items = [ChatItem::Failure(failure.clone())];
        assert!(
            !has_conversation(&items),
            "{failure:?} is a notice, not a transcript"
        );
    }
}

#[test]
fn activity_bar_title_prefers_session_title_then_prompt_preview() {
    let items = [ChatItem::UserText("run the tests".to_owned())];
    assert_eq!(
        activity_bar_title(Some("Refactor fold state"), &items).as_deref(),
        Some("Refactor fold state")
    );

    // No session title yet (pre first turn-end): the first prompt stands in.
    let items = [
        ChatItem::UserText("  fix the   parser  ".to_owned()),
        asst("sure"),
        ChatItem::UserText("second".to_owned()),
    ];
    assert_eq!(
        activity_bar_title(None, &items).as_deref(),
        Some("fix the parser")
    );

    let items = [ChatItem::UserText("hello".to_owned())];
    assert_eq!(
        activity_bar_title(Some("   "), &items).as_deref(),
        Some("hello")
    );

    // Neither a session title nor a user prompt → blank bar (no placeholder).
    assert_eq!(activity_bar_title(None, &[]), None);
    // Non-user leading items don't seed a title.
    assert_eq!(activity_bar_title(None, &[asst("greeting")]), None);
    // A whitespace-only prompt yields nothing.
    assert_eq!(
        activity_bar_title(None, &[ChatItem::UserText("   ".to_owned())]),
        None
    );
}
