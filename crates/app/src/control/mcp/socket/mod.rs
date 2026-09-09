//! The socket the `daruda --mcp` shim connects back through.
//!
//! `smol::net::UnixListener` rather than a blocking std listener plus a
//! thread: `smol` is already a direct dependency and re-exports `async-net`,
//! and flow execution already bridges background work into GPUI with
//! `smol::channel` + `cx.spawn` — this reuses that shape rather than inventing
//! a second one.
//!
//! Four parts: [`files`] owns what sits on disk, [`auth`] decides who may
//! speak, [`server`] binds and serves the one connection allowed at a time,
//! and [`frame`] reads a single message off the wire.

mod auth;
mod files;
mod frame;
mod server;
#[cfg(test)]
mod tests;

pub(crate) use auth::TokenGate;
pub(crate) use files::{Runtime, runtime_path};
pub(crate) use frame::MAX_FRAME_BYTES;
pub(crate) use server::{Inbound, Server};

// Reached from outside this module only by tests: a fixture standing in for a
// live surface, and the shim's round-trip against a real socket.
#[cfg(test)]
pub(crate) use auth::new_runtime_id;
#[cfg(test)]
pub(crate) use files::{Ownership, socket_path};

#[derive(Debug)]
pub(crate) enum SocketError {
    /// Another process in this profile already owns the socket.
    AlreadyRunning,
    PathTooLong {
        len: usize,
        max: usize,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "another daruda already owns this profile's socket"),
            Self::PathTooLong { len, max } => {
                write!(f, "socket path is {len} bytes, over the {max}-byte limit")
            }
            Self::Io(e) => write!(f, "socket io: {e}"),
        }
    }
}
