//! MIME type mapping based on file extensions.
//!
//! Reference: https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/MIME_types/Common_types

use std::collections::HashMap;
use std::sync::LazyLock;

pub(crate) static MIME_TYPES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        // Audio
        ("aac", "audio/aac"),
        ("midi", "audio/midi"),
        ("mid", "audio/midi"),
        ("mp3", "audio/mpeg"),
        ("oga", "audio/ogg"),
        ("opus", "audio/ogg"),
        ("wav", "audio/wav"),
        ("weba", "audio/webm"),
        // Video
        ("avi", "video/x-msvideo"),
        ("mp4", "video/mp4"),
        ("mpeg", "video/mpeg"),
        ("ogv", "video/ogg"),
        ("webm", "video/webm"),
        ("3gp", "video/3gpp"),
        ("3g2", "video/3gpp2"),
        ("ts", "video/mp2t"),
        // Images
        ("apng", "image/apng"),
        ("avif", "image/avif"),
        ("bmp", "image/bmp"),
        ("gif", "image/gif"),
        ("ico", "image/vnd.microsoft.icon"),
        ("jpeg", "image/jpeg"),
        ("jpg", "image/jpeg"),
        ("png", "image/png"),
        ("svg", "image/svg+xml"),
        ("tif", "image/tiff"),
        ("tiff", "image/tiff"),
        ("webp", "image/webp"),
        // Fonts
        ("eot", "application/vnd.ms-fontobject"),
        ("otf", "font/otf"),
        ("ttf", "font/ttf"),
        ("woff", "font/woff"),
        ("woff2", "font/woff2"),
        // Text/Code
        ("css", "text/css"),
        ("csv", "text/csv"),
        ("html", "text/html"),
        ("htm", "text/html"),
        ("ics", "text/calendar"),
        ("js", "text/javascript"),
        ("cjs", "text/javascript"),
        ("mjs", "text/javascript"),
        ("txt", "text/plain"),
        ("xml", "application/xml"),
        // Application
        ("json", "application/json"),
        ("jsonld", "application/ld+json"),
        ("pdf", "application/pdf"),
        ("wasm", "application/wasm"),
        ("xhtml", "application/xhtml+xml"),
        ("zip", "application/zip"),
        ("7z", "application/x-7z-compressed"),
        ("gz", "application/gzip"),
        ("tar", "application/x-tar"),
        ("rar", "application/vnd.rar"),
        ("bz", "application/x-bzip"),
        ("bz2", "application/x-bzip2"),
        ("bin", "application/octet-stream"),
        ("sh", "application/x-sh"),
        ("csh", "application/x-csh"),
        ("jar", "application/java-archive"),
        ("php", "application/x-httpd-php"),
        ("rtf", "application/rtf"),
        // Documents
        ("doc", "application/msword"),
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        ("xls", "application/vnd.ms-excel"),
        (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        ("ppt", "application/vnd.ms-powerpoint"),
        (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        ("odt", "application/vnd.oasis.opendocument.text"),
        ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
        ("odp", "application/vnd.oasis.opendocument.presentation"),
        // Other
        ("abw", "application/x-abiword"),
        ("arc", "application/x-freearc"),
        ("azw", "application/vnd.amazon.ebook"),
        ("cda", "application/x-cdf"),
        ("epub", "application/epub+zip"),
        ("mpkg", "application/vnd.apple.installer+xml"),
        ("ogx", "application/ogg"),
        ("vsd", "application/vnd.visio"),
        ("xul", "application/vnd.mozilla.xul+xml"),
    ])
});

pub(crate) fn get_mime_type(extension: &str) -> &'static str {
    MIME_TYPES
        .get(extension.to_lowercase().as_str())
        .unwrap_or(&"application/octet-stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_mime_type() {
        assert_eq!(get_mime_type("html"), "text/html");
        assert_eq!(get_mime_type("HTML"), "text/html");
        assert_eq!(get_mime_type("css"), "text/css");
        assert_eq!(get_mime_type("js"), "text/javascript");
        assert_eq!(get_mime_type("json"), "application/json");
        assert_eq!(get_mime_type("png"), "image/png");
        assert_eq!(get_mime_type("unknown"), "application/octet-stream");
    }
}
