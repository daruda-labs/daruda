use super::{Result, TransportError};
use serde_json::Value;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

pub type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_millis(250);

pub fn connect(url: &str) -> Result<Socket> {
    let parsed =
        url::Url::parse(url).map_err(|_| TransportError::Message("Invalid gateway URL".into()))?;
    if parsed.scheme() != "wss" {
        return Err(TransportError::Message("Gateway requires TLS".into()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| TransportError::Message("Gateway URL has no host".into()))?;
    let addresses = (host, parsed.port().unwrap_or(443))
        .to_socket_addrs()
        .map_err(|_| TransportError::Message("Gateway DNS lookup failed".into()))?;
    let stream = addresses
        .filter_map(|addr| TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok())
        .next()
        .ok_or_else(|| TransportError::Message("Gateway connection failed".into()))?;
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(io_error)?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .map_err(io_error)?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(super::http::MAX_BODY_BYTES))
        .max_frame_size(Some(super::http::MAX_BODY_BYTES));
    let (mut socket, _) = tungstenite::client_tls_with_config(url, stream, Some(config), None)
        .map_err(|_| TransportError::Message("Gateway TLS or WebSocket handshake failed".into()))?;
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(Some(READ_TIMEOUT)),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(Some(READ_TIMEOUT)),
        _ => {
            return Err(TransportError::Message(
                "Unsupported gateway TLS transport".into(),
            ));
        }
    }
    .map_err(io_error)?;
    Ok(socket)
}

fn io_error(_: std::io::Error) -> TransportError {
    TransportError::Message("Gateway socket configuration failed".into())
}

pub fn send(socket: &mut Socket, payload: &Value) -> Result<()> {
    socket
        .send(Message::Text(payload.to_string().into()))
        .map_err(|_| TransportError::Message("Gateway write failed".into()))
}

pub fn read(socket: &mut Socket) -> Result<Option<Value>> {
    match socket.read() {
        Ok(Message::Text(text)) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| TransportError::Message("Gateway sent invalid JSON".into())),
        Ok(Message::Close(frame)) => Err(TransportError::Closed(
            frame.map_or(1000, |frame| frame.code.into()),
        )),
        Ok(_) => {
            socket
                .flush()
                .map_err(|_| TransportError::Message("Gateway pong failed".into()))?;
            Ok(None)
        }
        Err(tungstenite::Error::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(_) => Err(TransportError::Message("Gateway read failed".into())),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn plaintext_gateway_urls_are_rejected_before_connecting() {
        assert!(super::connect("ws://localhost").is_err());
    }
}
