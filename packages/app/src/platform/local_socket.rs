//! Local IPC transports selected once for the app and its MCP shim.

#[cfg(unix)]
pub(crate) use smol::net::unix::{UnixListener as Listener, UnixStream as Stream};
#[cfg(unix)]
pub(crate) use std::os::unix::net::UnixStream as BlockingStream;

#[cfg(windows)]
#[path = "local_socket_windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use uds_windows::UnixStream as BlockingStream;
#[cfg(windows)]
pub(crate) use windows::{Listener, Stream, restrict};

#[cfg(unix)]
pub(crate) fn restrict(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_ONLY_FILE))
}

#[cfg(unix)]
pub(crate) const OWNER_ONLY_FILE: u32 = 0o600;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{AsyncReadExt as _, AsyncWriteExt as _};

    #[test]
    fn bound_socket_accepts_and_exchanges_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipc.sock");
        let listener = Listener::bind(&path).unwrap();
        restrict(&path).unwrap();
        let client = std::thread::spawn(move || {
            use std::io::{Read as _, Write as _};
            let mut stream = BlockingStream::connect(path).unwrap();
            stream.write_all(b"ping").unwrap();
            let mut reply = [0; 4];
            stream.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"pong");
        });
        smol::block_on(async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });
        client.join().unwrap();
    }
}
