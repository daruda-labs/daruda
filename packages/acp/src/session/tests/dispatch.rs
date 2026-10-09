use super::*;

/// A resume replays the whole conversation before the session is
/// `Connected`, so the router has to hold the parent/child relation across
/// that boundary. Nothing else covers it: the capture-replay guard
/// (`wire_log::replay`) reconstructs from a file rather than a live load,
/// and the live subagent test above runs entirely after `Connected`.
#[test]
fn a_resume_replays_a_subagent_before_the_session_is_connected() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            native_subagent_resume_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            Some(SessionId::from("sess-root")),
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        // The loop exits at `Connected`, so everything counted here
        // necessarily arrived while the session was still loading.
        let mut items: Vec<ChatItem> = Vec::new();
        let mut updates_before_connected = 0usize;
        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Update(update)) => {
                    updates_before_connected += 1;
                    crate::mapping::apply_update(&mut items, &update);
                }
                Some(AcpEvent::Connected { .. }) => break,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        }

        assert!(
            updates_before_connected > 0,
            "the replay is supposed to land during the load, not after it"
        );
        let [ChatItem::ToolCall(parent), ChatItem::ToolCall(child)] = &items[..] else {
            panic!("a launch and its one child, got {items:?}");
        };
        assert!(parent.is_subagent_launch());
        assert_eq!(parent.subagent_type(), Some("Lorentz"));
        assert_eq!(parent.status, crate::model::ToolStatusView::Completed);
        assert_eq!(
            child.parent_tool_id.as_deref(),
            Some(parent.id.as_str()),
            "a resumed child renders inside its launch, not as a top-level row"
        );

        drop(command_tx);
        let _ = connection.await;
    });
}

/// The whole native subagent path over a real SDK connection: the compat
/// notification really does bind to `session/update`, the raw payload
/// survives, and the router turns a child session's work into the flat
/// parent/child tool calls the render model already draws.
#[test]
fn a_native_subagent_arrives_as_a_launch_card_with_its_child() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            native_subagent_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { .. }) => break,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        }
        command_tx
            .unbounded_send(Command::Prompt("go".into()))
            .unwrap();

        let mut items: Vec<ChatItem> = Vec::new();
        let mut notices = 0usize;
        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Update(update)) => {
                    crate::mapping::apply_update(&mut items, &update);
                }
                Some(AcpEvent::Notice(_)) => notices += 1,
                Some(AcpEvent::TurnEnded { .. }) => break,
                Some(_) => {}
                None => panic!("turn never ended"),
            }
        }

        let [ChatItem::ToolCall(parent), ChatItem::ToolCall(child)] = &items[..] else {
            panic!("a launch and its one child, got {items:?}");
        };
        assert!(parent.is_subagent_launch());
        assert_eq!(parent.subagent_type(), Some("Lorentz"));
        assert_eq!(parent.status, crate::model::ToolStatusView::Completed);
        assert_eq!(
            child.parent_tool_id.as_deref(),
            Some(parent.id.as_str()),
            "the child renders inside its launch, not as a top-level row"
        );
        assert_eq!(notices, 1, "the unknown kind is reported exactly once");

        drop(command_tx);
        let _ = connection.await;
    });
}

/// The legacy-delegation advisory has to survive an update the router
/// cannot type. The slot it spends is once-per-session, so asking for it
/// on an update that will not report it costs the notice permanently —
/// the user is left with "Ignoring an unrecognized session update" alone.
#[test]
fn a_legacy_delegation_is_reported_once_even_behind_an_untypable_update() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            legacy_delegation_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { .. }) => break,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        }
        command_tx
            .unbounded_send(Command::Prompt("go".into()))
            .unwrap();

        let mut legacy = 0usize;
        let mut notices = 0usize;
        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::LegacyDelegation) => legacy += 1,
                Some(AcpEvent::Notice(_)) => notices += 1,
                Some(AcpEvent::TurnEnded { .. }) => break,
                Some(_) => {}
                None => panic!("turn never ended"),
            }
        }

        assert_eq!(
            legacy, 1,
            "the spawn reports it once — not zero because an untypable \
             update went first, and not twice because `wait` followed"
        );
        assert_eq!(notices, 1, "the unknown kind is still reported");

        drop(command_tx);
        let _ = connection.await;
    });
}

/// Measures the SDK 2.0 dispatch semantics the permission park relies on:
/// the handler in `run_connection` awaits the host's decision *inside*
/// `on_receive_request`, and its comment claims the connection keeps
/// pumping concurrently. If instead the dispatch loop is held until the
/// handler returns (as `HandleDispatchFrom`'s "the server will not process
/// new messages until this handler returns" warns), the update queued
/// right behind the permission request can only surface *after*
/// `respond_permission` — streaming would freeze under every permission
/// prompt. The `#282` ordered-response barrier is not in play here: it
/// arms only via `on_receiving_result`, which this crate never calls.
#[test]
fn a_parked_permission_request_does_not_stall_update_dispatch() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: permission_parks.clone(),
    };

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            permission_then_update_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { .. }) => break,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        }

        handle.send_prompt("run a tool".to_string());

        let permission_id = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::PermissionRequested { id, .. }) => break id,
                Some(AcpEvent::Update(_)) => {
                    panic!("update outran the permission request sent before it")
                }
                Some(_) => {}
                None => panic!("permission request never surfaced"),
            }
        };

        // THE MEASUREMENT — the permission is still undecided, so this
        // update only arrives if the parked handler leaves the dispatch
        // loop free.
        match next_event_within(&mut event_rx).await {
            Some(AcpEvent::Update(_)) => {}
            Some(other) => panic!("expected the queued update, got {other:?}"),
            None => panic!(
                "dispatch loop is blocked by the parked permission handler: the \
                 session/update queued behind the permission request never surfaced"
            ),
        }

        handle.respond_permission(
            permission_id,
            PermissionDecision::Allow {
                option_id: "allow".to_string(),
            },
        );

        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::TurnEnded {
                    completed_normally, ..
                }) => {
                    assert!(completed_normally);
                    break;
                }
                Some(_) => {}
                None => panic!("turn never ended after the permission decision"),
            }
        }

        // Closing the command channel ends `prompt_loop`, and with it the
        // whole connection task.
        drop(handle);
        connection.await.expect("connection task ends cleanly");
    });
}

#[test]
fn connected_carries_the_program_the_agent_reported() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: permission_parks.clone(),
    };

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            self_identifying_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        let program = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { program, .. }) => break program,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        };

        assert_eq!(
            program.as_deref(),
            Some("@agentclientprotocol/claude-agent-acp"),
            "the reported program reaches the host verbatim"
        );

        drop(handle);
        connection.await.expect("connection task ends cleanly");
    });
}

/// The ordering this event exists for: a resume replays the conversation as
/// updates before `Connected` resolves, so the program has to arrive first
/// or the restored half of a transcript is mapped under the wrong dialect.
#[test]
fn the_program_arrives_before_any_update() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: permission_parks.clone(),
    };

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            self_identifying_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        let mut identified_at: Option<usize> = None;
        let connected_at: usize;
        let mut seen = 0usize;
        loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::AgentIdentified { program }) => {
                    assert_eq!(
                        program.as_deref(),
                        Some("@agentclientprotocol/claude-agent-acp")
                    );
                    identified_at.get_or_insert(seen);
                }
                Some(AcpEvent::Connected { .. }) => {
                    connected_at = seen;
                    break;
                }
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
            seen += 1;
        }

        assert!(
            identified_at.is_some_and(|at| at < connected_at),
            "the program must be known before Connected, since a resume's \
             replayed updates land in between"
        );

        drop(handle);
        connection.await.expect("connection task ends cleanly");
    });
}

/// The advertised logins have to reach the host, and reach it classified:
/// a host comparing id strings at the call site eventually offers the
/// metered Console login as if it were the free one.
#[test]
fn connected_carries_the_agents_advertised_login_methods() {
    let (command_tx, command_rx) = unbounded::<Command>();
    let (event_tx, mut event_rx) = unbounded::<AcpEvent>();
    let permission_parks: PermissionParks = Arc::new(Mutex::new(HashMap::new()));
    let handle = AcpSessionHandle {
        commands: command_tx,
        permission_parks: permission_parks.clone(),
    };

    smol::block_on(async move {
        let connection = smol::spawn(run_connection(
            login_advertising_agent(),
            PathBuf::from("."),
            None,
            Vec::new(),
            None,
            Vec::new(),
            command_rx,
            event_tx,
            permission_parks,
        ));

        let login_methods = loop {
            match next_event_within(&mut event_rx).await {
                Some(AcpEvent::Connected { login_methods, .. }) => break login_methods,
                Some(_) => {}
                None => panic!("connection never reached Connected"),
            }
        };

        assert_eq!(
            login_methods.len(),
            2,
            "both advertised logins reach the host"
        );
        assert_eq!(login_methods[0].kind, crate::LoginMethodKind::Subscription);
        assert_eq!(login_methods[1].kind, crate::LoginMethodKind::MeteredApi);
        // The agent resolved the interpreter for us; the host must not
        // re-derive it.
        assert_eq!(
            login_methods[0]
                .command
                .as_ref()
                .expect("the subscription login carries a terminal-auth block")
                .program,
            "/opt/node/bin/node"
        );
        // The second method omits `_meta` — normal, not an error.
        assert_eq!(login_methods[1].command, None);

        drop(handle);
        connection.await.expect("connection task ends cleanly");
    });
}
