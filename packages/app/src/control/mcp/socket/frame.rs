//! Reading one newline-delimited frame off the wire.
//!
//! Both sides of this socket speak the same framing MCP's stdio transport
//! does, which is what lets the shim forward a frame without parsing it.

/// Bytes a single frame may carry. Generous for a tool call, far below what a
/// runaway peer could use to exhaust memory.
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Read one newline-delimited frame, refusing one that would exceed
/// [`MAX_FRAME_BYTES`].
///
/// Byte-by-byte against a cap rather than `AsyncBufReadExt::lines`, which
/// accumulates without bound: a peer sending an endless line with no newline
/// would otherwise drive the allocator, *before* presenting a token.
pub(super) async fn read_frame<R>(reader: &mut R) -> std::io::Result<Option<String>>
where
    R: futures::AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        let read = {
            use futures::AsyncReadExt as _;
            reader.read(&mut byte).await?
        };
        if read == 0 {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(decode(buf)?)
            });
        }
        if byte[0] == b'\n' {
            return Ok(Some(decode(buf)?));
        }
        if buf.len() >= MAX_FRAME_BYTES {
            return Err(std::io::Error::other("frame over the size limit"));
        }
        buf.push(byte[0]);
    }
}

fn decode(mut buf: Vec<u8>) -> std::io::Result<String> {
    // Tolerate CRLF: the frame is the line without its terminator.
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| std::io::Error::other("frame is not UTF-8"))
}

/// [`read_frame`] with a deadline. `Err` on timeout, so the caller drops the
/// connection rather than waiting on a peer that may never speak.
pub(super) async fn read_frame_deadlined<R>(
    reader: &mut R,
    within: std::time::Duration,
) -> std::io::Result<Option<String>>
where
    R: futures::AsyncBufRead + Unpin,
{
    use futures::FutureExt as _;

    // ALLOW: what this bounds is how long *another process* may stay silent,
    // so the clock has to be the wall one — a virtual clock cannot describe a
    // peer that is not ours to schedule. Threading a `BackgroundExecutor` in
    // would also couple `serve` to gpui, which this layer is deliberately free
    // of because gpui's test scheduler cannot drive OS socket I/O (see the
    // module docs). Tests pass a short explicit `within`.
    #[allow(clippy::disallowed_methods)]
    let deadline = smol::Timer::after(within);
    futures::select! {
        frame = read_frame(reader).fuse() => frame,
        _ = deadline.fuse() => {
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
        }
    }
}
