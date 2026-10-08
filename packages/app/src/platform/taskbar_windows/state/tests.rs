#[test]
fn unchanged_count_retries_after_failure_and_taskbar_recreation() {
    let mut state = super::Overlay::new();
    state.request(3);
    assert!(
        state
            .synchronize(false, true, |_| anyhow::bail!("temporary failure"))
            .is_err()
    );
    assert_eq!(state.applied, None);
    let mut attempts = 0;
    for invalidated in [false, false, true] {
        state
            .synchronize(invalidated, true, |count| {
                assert_eq!(count, 3);
                attempts += 1;
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(attempts, 2);
    state.request(0);
    state
        .synchronize(false, false, |_| panic!("no window"))
        .unwrap();
    assert_eq!(state.applied, Some(3));
    state
        .synchronize(false, true, |count| {
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
    assert_eq!(state.applied, Some(0));
}

#[test]
fn latest_request_wins_when_the_window_becomes_available() {
    let mut state = super::Overlay::new();
    for count in [3, 8, 0] {
        state.request(count);
        state
            .synchronize(false, false, |_| panic!("no window to update"))
            .unwrap();
        assert_eq!(state.applied, None);
    }
    state
        .synchronize(false, true, |count| {
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
    state
        .synchronize(false, true, |_| panic!("already applied"))
        .unwrap();
}
