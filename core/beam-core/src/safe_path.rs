//! Safe relative paths for files received from a peer.
//!
//! A sender controls the names in a manifest, so every path is treated as hostile. A
//! [`SafePath`] can only be built through [`SafePath::parse`], which:
//!
//! - **rejects** anything that could escape the destination folder: absolute paths, drive
//!   letters, `..`, `.`, empty segments and NUL bytes;
//! - **sanitizes** names that are merely awkward on some file systems (Windows, FAT/exFAT SD
//!   cards): forbidden characters become `_`, trailing dots/spaces are trimmed and reserved
//!   device names such as `CON` or `NUL` get a `_` prefix.
//!
//! The result can only be joined onto a destination with [`SafePath::resolve_in`], so it always
//! stays inside that folder.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::{BeamError, Result};

/// Longest accepted single path segment, in bytes (common file system limit).
pub const MAX_SEGMENT_LEN: usize = 255;

/// Longest accepted relative path, in bytes.
pub const MAX_PATH_LEN: usize = 4096;

/// Deepest accepted folder nesting.
pub const MAX_DEPTH: usize = 64;

/// Characters that are invalid in file names on Windows or FAT/exFAT storage.
const FORBIDDEN_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*', '\\'];

/// Device names reserved by Windows, with or without an extension.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A validated, sanitized path relative to a destination folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SafePath {
    segments: Vec<String>,
}

impl SafePath {
    /// Parses a `/`-separated relative path received from a peer.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::UnsafePath`] if the path is empty, too long, too deep, absolute,
    /// contains traversal segments or NUL bytes, or sanitizes down to nothing.
    pub fn parse(raw: &str) -> Result<Self> {
        let reject = |reason| {
            Err(BeamError::UnsafePath {
                path: raw.to_owned(),
                reason,
            })
        };

        if raw.is_empty() {
            return reject("path is empty");
        }
        if raw.len() > MAX_PATH_LEN {
            return reject("path is too long");
        }
        if raw.contains('\0') {
            return reject("path contains a NUL byte");
        }
        if raw.starts_with('/') || raw.starts_with('\\') || has_drive_prefix(raw) {
            return reject("path is absolute");
        }

        let mut segments = Vec::new();
        for segment in raw.split('/') {
            match segment {
                "" => return reject("path has an empty segment"),
                "." | ".." => return reject("path contains a traversal segment"),
                _ if segment.len() > MAX_SEGMENT_LEN => return reject("path segment is too long"),
                _ => {}
            }
            let clean = sanitize_segment(segment);
            if clean.is_empty() || clean == "." || clean == ".." {
                return reject("path segment is empty after sanitizing");
            }
            segments.push(clean);
        }
        if segments.len() > MAX_DEPTH {
            return reject("path is nested too deeply");
        }
        Ok(Self { segments })
    }

    /// The sanitized segments, outermost folder first.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// The final segment (the file name).
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.segments.last().map_or("", String::as_str)
    }

    /// A case-folded key used to detect paths that collide on case-insensitive file systems.
    #[must_use]
    pub fn collision_key(&self) -> String {
        self.segments.join("/").to_lowercase()
    }

    /// Joins this path onto `destination`. The result is always inside `destination`.
    #[must_use]
    pub fn resolve_in(&self, destination: &Path) -> PathBuf {
        let mut path = destination.to_path_buf();
        path.extend(&self.segments);
        path
    }
}

impl fmt::Display for SafePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.segments.join("/"))
    }
}

/// True for paths like `C:` or `c:foo` that Windows treats as drive-relative.
fn has_drive_prefix(raw: &str) -> bool {
    let mut chars = raw.chars();
    matches!((chars.next(), chars.next()), (Some(letter), Some(':')) if letter.is_ascii_alphabetic())
}

/// Replaces characters that are invalid on common file systems and neutralizes reserved names.
fn sanitize_segment(segment: &str) -> String {
    let replaced: String = segment
        .chars()
        .map(|c| {
            if c.is_control() || FORBIDDEN_CHARS.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced
        .trim_end_matches(['.', ' '])
        .trim_start_matches(' ');

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if RESERVED_NAMES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(stem))
    {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
#[path = "safe_path_tests.rs"]
mod tests;
