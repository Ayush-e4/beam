//! Beam transfer engine.
//!
//! This crate holds the platform-independent core of Beam: the wire protocol, transports,
//! cryptography and the file pipeline. Platform apps (Android today, desktop later) call into it
//! through thin bindings.

pub mod chunk_set;
pub mod error;
pub mod frame;
pub mod integrity;
pub mod journal;
pub mod manifest;
pub mod safe_path;
pub mod version;

pub use chunk_set::ChunkSet;
pub use error::{BeamError, ErrorCategory, Result};
pub use frame::{Decoded, Frame, FrameType};
pub use integrity::{ChunkLayout, FileHashes, Hash, RootHasher};
pub use journal::Journal;
pub use manifest::{Manifest, ManifestItem, TransferId};
pub use safe_path::SafePath;
pub use version::ProtocolVersion;
