//! Beam transfer engine.
//!
//! This crate holds the platform-independent core of Beam: the wire protocol, transports,
//! cryptography and the file pipeline. Platform apps (Android today, desktop later) call into it
//! through thin bindings.

pub mod error;
pub mod frame;
pub mod version;

pub use error::{BeamError, ErrorCategory, Result};
pub use frame::{Decoded, Frame, FrameType};
pub use version::ProtocolVersion;
