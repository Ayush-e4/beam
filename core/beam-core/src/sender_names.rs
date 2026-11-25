//! Picking unique, receiver-safe names for the files and folders in one folder of a send.
//!
//! Names are sanitized exactly as the receiver would sanitize them ([`SafePath::parse`]), so two
//! names that end up the same on the receiver (`a?` and `a_`, or `Photo.jpg` and `photo.JPG` on a
//! case-insensitive file system) are caught here. Later duplicates get a ` (2)`, ` (3)`, …
//! suffix before the extension, keeping the manifest valid.

use std::collections::HashSet;

use crate::safe_path::SafePath;

/// Name used when a file name sanitizes down to nothing (e.g. `...`) or is too long.
const UNNAMED: &str = "unnamed";

/// Hands out unique names within one folder.
#[derive(Debug, Default)]
pub struct NameAllocator {
    used: HashSet<String>,
}

impl NameAllocator {
    /// Returns a sanitized name for `raw` that no earlier call in this folder returned,
    /// ignoring case.
    pub fn allocate(&mut self, raw: &str) -> String {
        let clean = sanitize(raw);
        if self.used.insert(clean.to_lowercase()) {
            return clean;
        }
        let (stem, extension) = split_extension(&clean);
        // At most `used.len()` candidates can already be taken, so this range always has a free one.
        let last = self.used.len() + 2;
        (2..=last)
            .map(|n| format!("{stem} ({n}){extension}"))
            .find(|candidate| self.used.insert(candidate.to_lowercase()))
            .unwrap_or(clean)
    }
}

fn sanitize(raw: &str) -> String {
    match SafePath::parse(raw) {
        Ok(path) if path.segments().len() == 1 => path.file_name().to_owned(),
        _ => UNNAMED.to_owned(),
    }
}

/// Splits `name` into its stem and extension (with the dot). Dotfiles have no extension.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unique_names() {
        let mut names = NameAllocator::default();
        assert_eq!(names.allocate("a.txt"), "a.txt");
        assert_eq!(names.allocate("b.txt"), "b.txt");
    }

    #[test]
    fn suffixes_duplicates_ignoring_case() {
        let mut names = NameAllocator::default();
        assert_eq!(names.allocate("Photo.jpg"), "Photo.jpg");
        assert_eq!(names.allocate("photo.JPG"), "photo (2).JPG");
        assert_eq!(names.allocate("photo.jpg"), "photo (3).jpg");
        assert_eq!(names.allocate("photo (2).jpg"), "photo (2) (2).jpg");
    }

    #[test]
    fn catches_names_that_collide_after_sanitizing() {
        let mut names = NameAllocator::default();
        assert_eq!(names.allocate("a?"), "a_");
        assert_eq!(names.allocate("a_"), "a_ (2)");
        assert_eq!(names.allocate("CON.txt"), "_CON.txt");
    }

    #[test]
    fn handles_dotfiles_and_unusable_names() {
        let mut names = NameAllocator::default();
        assert_eq!(names.allocate(".env"), ".env");
        assert_eq!(names.allocate(".env"), ".env (2)");
        assert_eq!(names.allocate("..."), "unnamed");
        assert_eq!(names.allocate(&"x".repeat(300)), "unnamed (2)");
    }
}
