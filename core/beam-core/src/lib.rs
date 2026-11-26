//! Beam transfer engine.
//!
//! This crate holds the platform-independent core of Beam: the wire protocol, transports,
//! cryptography and the file pipeline. Platform apps (Android today, desktop later) call into it
//! through thin bindings.

mod base32;
pub mod chunk_set;
pub mod compress;
pub mod connection;
pub mod discovery;
pub mod error;
pub mod frame;
pub mod handshake;
pub mod hello;
mod hints;
pub mod integrity;
pub mod invite;
pub mod journal;
pub mod keys;
pub mod manifest;
pub mod messages;
mod mime;
pub mod offer;
pub mod pack;
pub mod progress;
#[cfg(feature = "native")]
pub mod quic;
#[cfg(feature = "native")]
mod quic_tls;
pub mod read_at;
pub mod receiver;
mod receiver_store;
#[cfg(any(feature = "native", target_family = "wasm"))]
mod runtime;
pub mod safe_path;
pub mod sender;
pub mod sender_files;
mod sender_names;
pub mod sender_opened;
mod sender_pack;
mod sender_source;
mod sender_stream;
pub mod session;
pub mod signaling;
pub mod staging;
pub mod storage_file;
#[cfg(feature = "native")]
pub mod tcp;
#[cfg(any(feature = "native", target_family = "wasm"))]
pub mod transfer;
pub mod transfer_staging;
#[cfg(feature = "native")]
mod udp_socket;
pub mod verification;
pub mod version;
#[cfg(feature = "native")]
pub mod webtransport;
#[cfg(feature = "native")]
mod webtransport_cert;
#[cfg(feature = "native")]
mod webtransport_setup;
mod wire;

pub use chunk_set::ChunkSet;
pub use connection::Connection;
pub use discovery::{Beacon, DiscoveryKey};
pub use error::{BeamError, ErrorCategory, Result};
pub use frame::{Decoded, Frame, FrameType};
pub use handshake::{Cipher, Established, Handshake, Role};
pub use hello::{Hello, exchange_hello};
pub use integrity::{ChunkLayout, FileHashes, Hash, RootHasher};
pub use invite::PairingInvite;
pub use journal::Journal;
pub use keys::{DeviceId, PublicKey, StaticKeypair};
pub use manifest::{Manifest, ManifestItem, TransferId};
pub use progress::Progress;
pub use read_at::ReadAt;
pub use receiver::Receiver;
pub use safe_path::SafePath;
pub use sender::Sender;
pub use sender_files::{PreparedTransfer, SourceFile, prepare_transfer, prepare_transfer_with_id};
pub use sender_opened::{
    OpenedFile, ReadableFile, prepare_opened, prepare_opened_with_id, prepare_readable,
    prepare_readable_with_id,
};
pub use session::Session;
pub use signaling::{SignalFrom, SignalMessage, SignalSeal};
pub use storage_file::StorageFile;
#[cfg(any(feature = "native", target_family = "wasm"))]
pub use transfer::{
    SendOutcome, confirm_received, read_offer, receive_transfer, receive_transfer_noting,
    reject_offer, send_transfer,
};
pub use transfer_staging::TransferStaging;
pub use verification::VerificationCode;
pub use version::ProtocolVersion;
