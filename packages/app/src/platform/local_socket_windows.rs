//! Windows AF_UNIX sockets registered with smol's readiness reactor.

use std::io;
use std::os::windows::io::{AsRawSocket, AsSocket, BorrowedSocket};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{AsyncRead, AsyncWrite};
use smol::Async;

struct ListenerSocket(uds_windows::UnixListener);

impl AsSocket for ListenerSocket {
    fn as_socket(&self) -> BorrowedSocket<'_> {
        // SAFETY: self owns the socket and the borrow cannot outlive self.
        unsafe { BorrowedSocket::borrow_raw(self.0.as_raw_socket()) }
    }
}

pub(crate) struct Listener(Async<ListenerSocket>);

impl Listener {
    pub(crate) fn bind(path: &Path) -> io::Result<Self> {
        Async::new(ListenerSocket(uds_windows::UnixListener::bind(path)?)).map(Self)
    }

    pub(crate) async fn accept(&self) -> io::Result<(Stream, uds_windows::SocketAddr)> {
        let (stream, addr) = self.0.read_with(|socket| socket.0.accept()).await?;
        Ok((Stream::try_from(stream)?, addr))
    }
}

#[derive(Clone)]
pub(crate) struct Stream(Arc<Async<uds_windows::UnixStream>>);

impl TryFrom<uds_windows::UnixStream> for Stream {
    type Error = io::Error;

    fn try_from(stream: uds_windows::UnixStream) -> io::Result<Self> {
        Async::new(stream).map(|stream| Self(Arc::new(stream)))
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut &*self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut &*self.0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_close(cx)
    }
}

pub(crate) fn restrict(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
    };

    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: the SDDL is static and descriptor is a valid output pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            windows_sys::core::w!("D:P(A;;GA;;;OW)"),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: path is terminated and descriptor was allocated by Win32.
    let result = unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    let error = (result == 0).then(io::Error::last_os_error);
    // SAFETY: this descriptor is released once with its matching allocator.
    unsafe { LocalFree(descriptor) };
    error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    #[test]
    fn restricting_a_missing_socket_reports_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(super::restrict(&dir.path().join("missing.sock")).is_err());
    }
}
