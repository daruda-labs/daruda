use super::*;

#[test]
fn streaming_polls_at_cap() {
    assert_eq!(
        stdout_poll_interval(CAP_30FPS, STREAM_ENTER_TICKS, 0),
        CAP_30FPS
    );
    // Streaming wins regardless of a stale idle counter.
    assert_eq!(
        stdout_poll_interval(CAP_30FPS, STREAM_ENTER_TICKS + 5, 99),
        CAP_30FPS
    );
}

#[test]
fn active_and_grace_window_poll_fast() {
    assert_eq!(stdout_poll_interval(CAP_30FPS, 0, 0), IDLE_POLL);
    assert_eq!(
        stdout_poll_interval(CAP_30FPS, STREAM_ENTER_TICKS - 1, 0),
        IDLE_POLL
    );
    assert_eq!(
        stdout_poll_interval(CAP_30FPS, 0, IDLE_GRACE_TICKS),
        IDLE_POLL
    );
}

#[test]
fn idle_backoff_doubles_up_to_max() {
    let at = |idle| stdout_poll_interval(CAP_30FPS, 0, idle);
    assert_eq!(at(IDLE_GRACE_TICKS + 1), Duration::from_millis(32));
    assert_eq!(at(IDLE_GRACE_TICKS + 2), Duration::from_millis(64));
    assert_eq!(at(IDLE_GRACE_TICKS + 3), Duration::from_millis(128));
    assert_eq!(at(IDLE_GRACE_TICKS + 4), IDLE_BACKOFF_MAX);
    assert_eq!(at(u32::MAX), IDLE_BACKOFF_MAX);
}

#[test]
fn high_fps_cap_bounds_the_fast_interval() {
    let cap = Duration::from_millis(8); // 120 fps
    assert_eq!(stdout_poll_interval(cap, 0, 0), cap);
    assert_eq!(stdout_poll_interval(cap, STREAM_ENTER_TICKS, 0), cap);
    // Backoff doubles from the bounded fast interval.
    assert_eq!(
        stdout_poll_interval(cap, 0, IDLE_GRACE_TICKS + 1),
        Duration::from_millis(16)
    );
}
