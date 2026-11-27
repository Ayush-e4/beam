//! Joining a signaling room (see `protocol.md`, Signaling rooms): a WebSocket to
//! `{service}/rooms/{room}` that relays sealed messages ([`crate::SignalSeal`]) between the
//! two devices of a pairing, and tells each when the other comes and goes.
//!
//! `service` is `wss://…` for the deployed service (TLS on ring, trusting Mozilla's root
//! certificates) or `ws://…` for a local `wrangler dev`. Nothing here sets timeouts; callers
//! bound each step with their own.

use std::fmt::Write as _;
use std::io;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use http::Uri;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::signal_stream::RoomStream;
use crate::{BeamError, Result};

/// Largest message a room relays (16 KiB).
pub const MAX_MESSAGE_LEN: usize = 16 * 1024;

/// What happened in the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomEvent {
    /// This device is in; `peers` were there already.
    Joined {
        /// How many devices were already in the room.
        peers: u32,
    },
    /// The other device joined.
    PeerJoined,
    /// The other device left.
    PeerLeft,
    /// A message couldn't be kept for the other device (the room's buffer is full).
    BufferFull,
    /// A sealed message from the other device.
    Message(Vec<u8>),
}

/// This device's place in one signaling room.
pub struct SignalRoom {
    socket: WebSocketStream<RoomStream>,
}

impl SignalRoom {
    /// Joins `room` on the signaling `service` (`wss://host[:port]` or `ws://host[:port]`).
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] for a service address that isn't `ws://` or `wss://`, a
    /// network error if it can't be reached, a TLS error as [`BeamError::Handshake`], and the
    /// room's refusal as [`BeamError::Signaling`].
    pub async fn join(service: &str, room: &[u8; 16]) -> Result<Self> {
        let uri: Uri = service
            .trim_end_matches('/')
            .parse()
            .map_err(|_| BeamError::Malformed(format!("bad signaling address {service:?}")))?;
        let secure = match uri.scheme_str() {
            Some("wss") => true,
            Some("ws") => false,
            _ => {
                return Err(BeamError::Malformed(format!(
                    "signaling address {service:?} must start with ws:// or wss://"
                )));
            }
        };
        let host = uri
            .host()
            .ok_or_else(|| BeamError::Malformed(format!("no host in {service:?}")))?
            .trim_matches(['[', ']']);
        let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
        let tcp = TcpStream::connect((host, port))
            .await
            .map_err(BeamError::network)?;
        let _ = tcp.set_nodelay(true); // small messages; an error only costs latency
        let stream = if secure {
            RoomStream::Tls(Box::new(tls(host, tcp).await?))
        } else {
            RoomStream::Plain(tcp)
        };
        let url = format!("{}/rooms/{}", service.trim_end_matches('/'), hex(room));
        let (socket, _) = tokio_tungstenite::client_async(url, stream)
            .await
            .map_err(refused)?;
        Ok(Self { socket })
    }

    /// Posts a sealed message for the other device; the room keeps it if it isn't there yet.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] if it is over [`MAX_MESSAGE_LEN`], a network error if the
    /// connection fails.
    pub async fn post(&mut self, sealed: Vec<u8>) -> Result<()> {
        if sealed.len() > MAX_MESSAGE_LEN {
            return Err(BeamError::Malformed(format!(
                "signaling messages are at most {MAX_MESSAGE_LEN} bytes"
            )));
        }
        self.socket
            .send(Message::Binary(sealed.into()))
            .await
            .map_err(network)
    }

    /// The next event, or `None` once the room closed normally. Unknown notices are skipped.
    ///
    /// # Errors
    ///
    /// [`BeamError::Signaling`] if the room closed this device out (full, expired, a limit),
    /// a network error if the connection drops.
    pub async fn next(&mut self) -> Result<Option<RoomEvent>> {
        while let Some(message) = self.socket.next().await {
            match message.map_err(network)? {
                Message::Binary(bytes) => return Ok(Some(RoomEvent::Message(bytes.to_vec()))),
                Message::Text(text) => {
                    if let Some(event) = notice(&text) {
                        return Ok(Some(event));
                    }
                }
                Message::Close(frame) => return closed(frame.as_ref()).map(|()| None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok(None)
    }

    /// Leaves the room.
    ///
    /// # Errors
    ///
    /// A network error if the goodbye can't be sent; the device is out either way.
    pub async fn leave(mut self) -> Result<()> {
        self.socket.close(None).await.map_err(network)
    }

    /// Joins `room`, leaves `sealed` there for the other device and goes: the room passes it
    /// on now if the other device is in, or keeps it until it joins.
    ///
    /// # Errors
    ///
    /// As [`Self::join`] and [`Self::post`], and [`BeamError::Signaling`] if the room turned
    /// this device away.
    pub async fn deliver(service: &str, room: &[u8; 16], sealed: Vec<u8>) -> Result<()> {
        let mut joined = Self::join(service, room).await?;
        match joined.next().await? {
            Some(RoomEvent::Joined { .. }) => {}
            _ => {
                return Err(BeamError::Signaling(
                    "the room didn't let this device in".into(),
                ));
            }
        }
        joined.post(sealed).await?;
        joined.leave().await
    }
}

async fn tls(host: &str, tcp: TcpStream) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|err| BeamError::Handshake(format!("TLS setup failed: {err}")))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| BeamError::Malformed(format!("bad signaling host {host:?}")))?;
    TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|err| BeamError::Handshake(format!("TLS with the signaling service: {err}")))
}

/// A room notice (`{"type":"joined","peers":1}` and the like). Only the server sends text,
/// so a malformed or unknown one is skipped rather than ending the room.
fn notice(text: &str) -> Option<RoomEvent> {
    let kind = json_field(text, "type")?;
    match kind {
        "joined" => Some(RoomEvent::Joined {
            peers: json_field(text, "peers")?.parse().ok()?,
        }),
        "peer-joined" => Some(RoomEvent::PeerJoined),
        "peer-left" => Some(RoomEvent::PeerLeft),
        "buffer-full" => Some(RoomEvent::BufferFull),
        _ => None,
    }
}

/// The raw value of `"name":` in a flat JSON object (a quoted string or a number).
fn json_field<'a>(json: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("\"{name}\"");
    let rest = json[json.find(&key)? + key.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.split('"').next();
    }
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// How a room closing maps to a result: a normal close is fine, the room's own codes say why
/// it turned this device out.
fn closed(frame: Option<&CloseFrame>) -> Result<()> {
    let Some(frame) = frame else { return Ok(()) };
    let why = match u16::from(frame.code) {
        4001 => "the room is full (two devices already)",
        4002 => "the room expired",
        4003 => "too many messages",
        4004 => "a message was too big",
        4005 => "a text message was sent",
        _ if matches!(frame.code, CloseCode::Normal | CloseCode::Away) => return Ok(()),
        code => {
            return Err(BeamError::Signaling(format!(
                "closed with code {code}: {}",
                frame.reason
            )));
        }
    };
    Err(BeamError::Signaling(why.into()))
}

fn refused(err: tokio_tungstenite::tungstenite::Error) -> BeamError {
    use tokio_tungstenite::tungstenite::Error;
    match err {
        Error::Http(response) => BeamError::Signaling(format!(
            "the service refused the room (HTTP {})",
            response.status()
        )),
        other => network(other),
    }
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(network)`
fn network(err: tokio_tungstenite::tungstenite::Error) -> BeamError {
    match err {
        tokio_tungstenite::tungstenite::Error::Io(err) => BeamError::network(err),
        other => BeamError::network(io::Error::other(other.to_string())),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}"); // writing to a String can't fail
        out
    })
}

#[cfg(test)]
#[path = "signal_room_tests.rs"]
mod tests;
