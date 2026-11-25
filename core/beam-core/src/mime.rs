//! A best-effort MIME type guess from a file name's extension.
//!
//! The guess only labels files for the receiver and helps skip compression for formats that are
//! already compressed. It is never trusted for security, so a small table of common types is
//! enough.

/// MIME type used when the extension is unknown.
pub const FALLBACK: &str = "application/octet-stream";

/// Guesses the MIME type of `file_name` from its extension.
pub fn guess(file_name: &str) -> &'static str {
    let Some((stem, extension)) = file_name.rsplit_once('.') else {
        return FALLBACK;
    };
    if stem.is_empty() {
        return FALLBACK; // A dotfile such as `.bashrc` has no extension.
    }
    by_extension(&extension.to_ascii_lowercase()).unwrap_or(FALLBACK)
}

fn by_extension(extension: &str) -> Option<&'static str> {
    let mime = match extension {
        // Text and documents.
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "rtf" => "application/rtf",
        "epub" => "application/epub+zip",
        "doc" => "application/msword",
        "xls" => "application/vnd.ms-excel",
        "ppt" => "application/vnd.ms-powerpoint",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        // Images.
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "dng" => "image/x-adobe-dng",
        // Audio.
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        // Video.
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "3gp" => "video/3gpp",
        // Archives and packages.
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        "zst" => "application/zstd",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "tar" => "application/x-tar",
        "apk" => "application/vnd.android.package-archive",
        "jar" => "application/java-archive",
        // Fonts.
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => return None,
    };
    Some(mime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_common_types_ignoring_case() {
        assert_eq!(guess("photo.JPG"), "image/jpeg");
        assert_eq!(guess("clip.mp4"), "video/mp4");
        assert_eq!(guess("notes.tar.gz"), "application/gzip");
        assert_eq!(guess("app.apk"), "application/vnd.android.package-archive");
    }

    #[test]
    fn falls_back_for_unknown_or_missing_extensions() {
        assert_eq!(guess("README"), FALLBACK);
        assert_eq!(guess(".bashrc"), FALLBACK);
        assert_eq!(guess("data.unknownext"), FALLBACK);
        assert_eq!(guess("trailing."), FALLBACK);
    }
}
