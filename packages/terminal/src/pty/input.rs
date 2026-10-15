use std::io::{self, Write};
use std::sync::mpsc;

use super::PtyError;

#[derive(Clone)]
/// A PTY input queue with optional acknowledgements from the actual writer.
pub struct PtyInput {
    sender: Option<mpsc::Sender<InputWrite>>,
}

pub(super) struct InputWrite {
    bytes: Vec<u8>,
    written: Option<mpsc::Sender<Result<(), PtyError>>>,
}

impl PtyInput {
    pub(super) fn channel() -> (Self, mpsc::Receiver<InputWrite>) {
        let (sender, receiver) = mpsc::channel();
        (
            Self {
                sender: Some(sender),
            },
            receiver,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn stub() -> Self {
        Self { sender: None }
    }

    /// Enqueue keyboard input without waiting for its write.
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), mpsc::SendError<Vec<u8>>> {
        match &self.sender {
            Some(sender) => sender
                .send(InputWrite {
                    bytes,
                    written: None,
                })
                .map_err(|error| mpsc::SendError(error.0.bytes)),
            None => Ok(()),
        }
    }

    /// Resolve after write_all and flush, or fail if the writer is gone.
    pub fn write_confirmed(&self, bytes: Vec<u8>) -> mpsc::Receiver<Result<(), PtyError>> {
        let (written, receiver) = mpsc::channel();
        match &self.sender {
            Some(sender) => {
                if let Err(error) = sender.send(InputWrite {
                    bytes,
                    written: Some(written.clone()),
                }) {
                    let _ = written.send(Err(PtyError::Io(error.to_string())));
                }
            }
            None => {
                let _ = written.send(Ok(()));
            }
        }
        receiver
    }
}

impl InputWrite {
    pub(super) fn write_to(self, writer: &mut impl Write) -> io::Result<()> {
        let result = writer.write_all(&self.bytes).and_then(|()| writer.flush());
        if let Some(written) = self.written {
            let _ = written.send(
                result
                    .as_ref()
                    .copied()
                    .map_err(|error| PtyError::Io(error.to_string())),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_is_acknowledged_only_after_the_writer_accepts_it() {
        let (input, pending) = PtyInput::channel();
        let written = input.write_confirmed(b"task".to_vec());
        assert!(matches!(written.try_recv(), Err(mpsc::TryRecvError::Empty)));
        let mut output = Vec::new();
        pending.recv().unwrap().write_to(&mut output).unwrap();
        assert!(written.recv().unwrap().is_ok());
        assert_eq!(output, b"task");
    }

    #[test]
    fn a_closed_writer_reports_failure_instead_of_success() {
        let (input, pending) = PtyInput::channel();
        drop(pending);
        assert!(input.write_confirmed(Vec::new()).recv().unwrap().is_err());
        assert!(
            PtyInput::stub()
                .write_confirmed(Vec::new())
                .recv()
                .unwrap()
                .is_ok()
        );
    }

    struct RefusingWriter;

    impl Write for RefusingWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_kernel_write_failure_is_returned_to_the_sender() {
        let (input, pending) = PtyInput::channel();
        let written = input.write_confirmed(b"task".to_vec());
        assert!(
            pending
                .recv()
                .unwrap()
                .write_to(&mut RefusingWriter)
                .is_err()
        );
        assert!(written.recv().unwrap().is_err());
    }
}
