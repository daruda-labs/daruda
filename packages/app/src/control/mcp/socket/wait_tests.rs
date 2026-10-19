//! Cross-layer delivery tests use the real JSON-line writer and foreground receipt handling.
use super::*;
use crate::control::mcp::{
    dispatch::{self, ConnectionSession},
    run_wait,
};
use crate::runs::{
    RunsHost,
    result::RunKey,
    seed::{self, SeedEnd, SeedRun},
};
use gpui::{App, TestAppContext};

fn request(id: u64, timeout: u64) -> String {
    serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"daruda_run_wait","arguments":{"timeout":timeout}}}).to_string()
}
fn deliver(
    frame: String,
    connection: u64,
    session: &mut ConnectionSession,
    cx: &mut TestAppContext,
) -> smol::channel::Receiver<Outgoing> {
    let (reply, receiver) = smol::channel::unbounded();
    cx.update(|cx| {
        dispatch::answer(
            Inbound::Frame {
                frame,
                reply,
                connection,
            },
            session,
            None,
            cx,
        )
    });
    receiver
}
fn handshake(connection: u64, session: &mut ConnectionSession, cx: &mut TestAppContext) {
    let init = serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}).to_string();
    assert!(deliver(init, connection, session, cx).try_recv().is_ok());
    deliver(
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        connection,
        session,
        cx,
    );
}
fn completed(cx: &mut App) -> RunKey {
    RunsHost::ensure_installed(cx);
    let run = SeedRun::chat("/work");
    let task = seed::task(
        seed::spec(Default::default(), "done"),
        Some((&run, SeedEnd::Done)),
        cx,
    );
    let key = RunKey::Task(run.exec(&task));
    RunsHost::register_run(key.clone(), cx);
    key
}
async fn write(outgoing: Outgoing) -> serde_json::Value {
    let mut bytes = futures::io::Cursor::new(Vec::new());
    server::write_response(&mut bytes, outgoing).await.unwrap();
    let bytes = bytes.into_inner();
    assert_eq!(bytes.last(), Some(&b'\n'));
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["result"]["structuredContent"]
        .clone()
}

#[gpui::test]
async fn only_the_full_socket_write_reports_a_wait_and_next_session_recovers_it(
    cx: &mut TestAppContext,
) {
    let key = cx.update(completed);
    let mut session = ConnectionSession::default();
    handshake(1, &mut session, cx);
    let rx = deliver(request(1, 0), 1, &mut session, cx);
    let outgoing = rx.recv().await.unwrap();
    cx.update(|cx| {
        assert!(RunsHost::run_wait_covers(&key, cx));
        assert!(RunsHost::unreported_run_events(cx).is_empty());
    });
    let answer = write(outgoing).await;
    assert_eq!(answer["ended"].as_array().unwrap().len(), 1);
    cx.run_until_parked();
    cx.update(|cx| assert!(!RunsHost::run_wait_covers(&key, cx)));
    handshake(2, &mut session, cx);
    let rx = deliver(request(2, 0), 2, &mut session, cx);
    assert_eq!(write(rx.recv().await.unwrap()).await["outcome"], "nothing");
    cx.run_until_parked();
    cx.update(|cx| {
        let wait = run_wait::prepare(
            Some(vec![
                super::super::runs::convert(RunsHost::run_query(key, cx).read())
                    .handle
                    .to_string(),
            ]),
            0,
            cx,
        )
        .unwrap_or_else(|_| panic!("prepared"));
        assert_eq!(
            RunsHost::poll_run_wait(wait.pending.0, false, cx)
                .unwrap()
                .already_reported
                .len(),
            1
        );
        wait.pending.withdraw(cx);
    });
}

#[gpui::test]
async fn cancellation_and_disconnect_restore_in_flight_and_late_write_ack_is_ignored(
    cx: &mut TestAppContext,
) {
    cx.update(completed);
    let mut session = ConnectionSession::default();
    handshake(1, &mut session, cx);
    let rx = deliver(request(1, 0), 1, &mut session, cx);
    let old = rx.recv().await.unwrap();
    deliver(serde_json::json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}).to_string(), 1, &mut session, cx);
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
    let rx = deliver(request(2, 0), 1, &mut session, cx);
    let current = rx.recv().await.unwrap();
    write(old).await;
    cx.run_until_parked();
    cx.update(|cx| assert!(RunsHost::unreported_run_events(cx).is_empty()));
    cx.update(|cx| {
        dispatch::answer(
            Inbound::Disconnected { connection: 1 },
            &mut session,
            None,
            cx,
        )
    });
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
    write(current).await;
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
    handshake(2, &mut session, cx);
    let rx = deliver(request(3, 0), 2, &mut session, cx);
    assert_eq!(
        write(rx.recv().await.unwrap()).await["ended"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    cx.run_until_parked();
}

#[gpui::test]
async fn replacement_withdraws_pending_wait_and_failed_write_makes_it_available(
    cx: &mut TestAppContext,
) {
    cx.update(completed);
    let mut session = ConnectionSession::default();
    handshake(1, &mut session, cx);
    let rx = deliver(request(1, 0), 1, &mut session, cx);
    let old = rx.recv().await.unwrap();
    handshake(2, &mut session, cx);
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
    drop(old);
    cx.run_until_parked();
    let rx = deliver(request(2, 0), 2, &mut session, cx);
    let outgoing = rx.recv().await.unwrap();
    let mut buffer = [0u8; 0];
    let mut writer = futures::io::AllowStdIo::new(std::io::Cursor::new(&mut buffer[..]));
    assert!(server::write_response(&mut writer, outgoing).await.is_err());
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(RunsHost::unreported_run_events(cx).len(), 1));
}

#[gpui::test]
async fn old_disconnect_retries_a_notice_after_the_new_gate_flushed(cx: &mut TestAppContext) {
    use crate::window_registry::WindowRegistry;
    crate::test_support::register_test_orchestrator(cx);
    let host = cx.update(|cx| {
        WindowRegistry::orchestrator(cx)
            .unwrap()
            .1
            .upgrade()
            .unwrap()
    });
    // A live idle session, so a notice is sent the moment it is eligible.
    let _probe = host.update(cx, |ws, cx| ws.connect_orchestrator_chat_for_test(cx));
    cx.update(crate::workspace::Workspace::install_task_effects);
    cx.update(completed);
    let mut session = ConnectionSession::default();
    handshake(1, &mut session, cx);
    let rx = deliver(request(1, 0), 1, &mut session, cx);
    let old = rx.recv().await.unwrap();
    // Notices the orchestrator received, as prompts sent to its session.
    let notices = |cx: &mut TestAppContext| {
        host.read_with(cx, |ws, cx| ws.orchestrator_sent_prompts_for_test(cx))
    };
    cx.update(|cx| {
        crate::orchestrator::seed_control_surface_for_test(cx);
        assert!(crate::orchestrator::session_tools(cx).is_some());
    });
    cx.run_until_parked();
    assert_eq!(
        notices(cx),
        0,
        "the old waiter still covers its in-flight event"
    );
    cx.update(|cx| {
        dispatch::answer(
            Inbound::Disconnected { connection: 1 },
            &mut session,
            None,
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(notices(cx), 1, "withdrawal alone must retry the idle pane");
    drop(old);
    cx.run_until_parked();
    assert_eq!(notices(cx), 1, "a late receipt cannot duplicate acceptance");
}
