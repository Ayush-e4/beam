//! Beam transfer engine.
//!
//! This crate holds the platform-independent core of Beam: the wire protocol, transports,
//! cryptography and the file pipeline. Platform apps (Android today, desktop later) call into it
//! through thin bindings.

pub mod chunk_set;
pub mod compress;
pub mod connection;
pub mod error;
pub mod frame;
pub mod handshake;
pub mod integrity;
pub mod journal;
pub mod keys;
pub mod manifest;
pub mod messages;
pub mod offer;
pub mod pack;
pub mod quic;
mod quic_tls;
pub mod receiver;
mod receiver_store;
pub mod safe_path;
pub mod session;
pub mod tcp;
pub mod verification;
pub mod version;
mod wire;

pub use chunk_set::ChunkSet;
pub use connection::Connection;
pub use error::{BeamError, ErrorCategory, Result};
pub use frame::{Decoded, Frame, FrameType};
pub use handshake::{Cipher, Established, Handshake, Role};
pub use integrity::{ChunkLayout, FileHashes, Hash, RootHasher};
pub use journal::Journal;
pub use keys::{PublicKey, StaticKeypair};
pub use manifest::{Manifest, ManifestItem, TransferId};
pub use receiver::Receiver;
pub use safe_path::SafePath;
pub use session::Session;
pub use verification::VerificationCode;
pub use version::ProtocolVersion;
