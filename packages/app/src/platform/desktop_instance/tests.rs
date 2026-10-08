use super::*;

#[test]
fn second_launch_forwards_and_profile_lock_releases_on_exit() {
    let directory = tempfile::tempdir().unwrap();
    let Launch::Primary(first) = acquire(directory.path(), Request::default(), false).unwrap()
    else {
        panic!("first launch");
    };
    assert!(matches!(
        acquire(directory.path(), Request::default(), false).unwrap(),
        Launch::Forwarded
    ));
    assert!(
        first
            .receiver
            .recv_timeout(TIMEOUT)
            .unwrap()
            .paths
            .is_empty()
    );
    let other = tempfile::tempdir().unwrap();
    assert!(matches!(
        acquire(other.path(), Request::default(), false).unwrap(),
        Launch::Primary(_)
    ));
    drop(first);
    assert!(matches!(
        acquire(directory.path(), Request::default(), false).unwrap(),
        Launch::Primary(_)
    ));
}

#[test]
fn validation_launch_never_forwards_to_an_existing_gui() {
    let directory = tempfile::tempdir().unwrap();
    let Launch::Primary(first) = start(directory.path(), []).unwrap() else {
        panic!("first launch");
    };
    for flag in [
        "--smoke",
        "--screenshot",
        "--replay-acp-log",
        "--screenshot=shot.png",
        "--replay-acp-log=wire.log",
    ] {
        assert!(start(directory.path(), [flag.into()]).is_err());
        assert!(first.receiver.try_recv().is_err());
    }
}

#[test]
fn relative_open_paths_are_resolved_before_forwarding() {
    let request = request_from_args([std::ffi::OsString::from("--open"), ".".into()]).unwrap();
    assert!(request.paths[0].is_absolute());
    assert!(request_from_args([std::ffi::OsString::from("--open")]).is_err());
}

#[test]
fn unauthenticated_request_never_reaches_the_foreground_queue() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .write_all(br#"{"token":"wrong","request":{"paths":[]}}"#)
        .unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let (sender, receiver) = mpsc::channel();
    let (stream, _) = listener.accept().unwrap();
    assert!(receive(stream, "secret", &sender).is_err());
    assert!(receiver.try_recv().is_err());
}
