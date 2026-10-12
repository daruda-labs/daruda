//! Coalesce raw watcher events into one batch per quiet window.
//!
//! The thread blocks on `recv()` until the first event, so it parks at zero
//! CPU while nothing changes. What a batch holds and how it is emitted is the
//! caller's: the file tree keeps paths, the git-dir watcher a signal.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::Event;

/// Run until `raw_rx` disconnects. Each batch starts at the first event and
/// closes `window` later; a watcher error flushes what came before it.
pub(super) fn debounce<B: Default>(
    raw_rx: mpsc::Receiver<Result<Event, String>>,
    window: Duration,
    mut absorb: impl FnMut(&mut B, Event),
    mut flush: impl FnMut(B),
    mut fail: impl FnMut(String),
) {
    loop {
        let first = match raw_rx.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => {
                fail(error);
                continue;
            }
            Err(_) => return,
        };
        let mut batch = B::default();
        absorb(&mut batch, first);
        let deadline = Instant::now() + window;

        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match raw_rx.recv_timeout(deadline - now) {
                Ok(Ok(event)) => absorb(&mut batch, event),
                Ok(Err(error)) => {
                    flush(std::mem::take(&mut batch));
                    fail(error);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    flush(batch);
                    return;
                }
            }
        }
        flush(batch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Out {
        Batch(usize),
        Error,
    }

    fn run(inputs: Vec<Result<Event, String>>) -> Vec<Out> {
        let (tx, rx) = mpsc::channel();
        for input in inputs {
            tx.send(input).unwrap();
        }
        drop(tx);
        let out = std::cell::RefCell::new(Vec::new());
        debounce::<usize>(
            rx,
            Duration::from_secs(5),
            |count, _| *count += 1,
            |count| out.borrow_mut().push(Out::Batch(count)),
            |_| out.borrow_mut().push(Out::Error),
        );
        out.into_inner()
    }

    fn event() -> Result<Event, String> {
        Ok(Event::new(notify::EventKind::Any))
    }

    #[test]
    fn a_burst_flushes_once_when_the_source_closes() {
        assert_eq!(run(vec![event(), event(), event()]), [Out::Batch(3)]);
    }

    #[test]
    fn an_error_flushes_the_batch_before_it_and_starts_a_new_one() {
        assert_eq!(
            run(vec![event(), Err("lost".into()), event()]),
            [Out::Batch(1), Out::Error, Out::Batch(1)]
        );
    }

    #[test]
    fn an_error_with_nothing_pending_flushes_nothing() {
        assert_eq!(run(vec![Err("lost".into())]), [Out::Error]);
    }

    #[test]
    fn a_quiet_window_closes_the_batch_before_the_next_event() {
        let (tx, rx) = mpsc::channel();
        let sender = std::thread::spawn(move || {
            tx.send(event()).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            tx.send(event()).unwrap();
        });
        let out = std::cell::RefCell::new(Vec::new());
        debounce::<usize>(
            rx,
            Duration::from_millis(20),
            |count, _| *count += 1,
            |count| out.borrow_mut().push(Out::Batch(count)),
            |_| out.borrow_mut().push(Out::Error),
        );
        sender.join().unwrap();
        assert_eq!(out.into_inner(), [Out::Batch(1), Out::Batch(1)]);
    }

    #[test]
    fn a_closed_source_returns_without_a_batch() {
        assert!(run(Vec::new()).is_empty());
    }
}
