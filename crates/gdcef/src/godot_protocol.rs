//! Custom Godot scheme handlers for CEF.
//!
//! This module implements custom scheme handlers that allow CEF to load
//! resources from Godot's filesystem using `res://` and `user://` protocols.
//! This enables exported Godot projects to serve local web content (HTML, CSS,
//! JS, images, etc.) directly to the embedded browser without requiring an
//! external web server.
//!
//! - `res://` - Access resources from Godot's packed resource system
//! - `user://` - Access files from Godot's user data directory

use cef::{
    CefStringUtf16, ImplRequest, ImplResourceHandler, ImplResponse, ImplSchemeHandlerFactory,
    ResourceHandler, SchemeHandlerFactory, WrapResourceHandler, WrapSchemeHandlerFactory, rc::Rc,
    wrap_resource_handler, wrap_scheme_handler_factory,
};
use godot::classes::FileAccess;
use godot::classes::file_access::ModeFlags;
use godot::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

/// Static MIME type mapping based on file extensions.
/// Reference: https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/MIME_types/Common_types
static MIME_TYPES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
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

fn get_mime_type(extension: &str) -> &'static str {
    MIME_TYPES
        .get(extension.to_lowercase().as_str())
        .unwrap_or(&"application/octet-stream")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GodotScheme {
    Res,
    User,
}

impl GodotScheme {
    fn prefix(&self) -> &'static str {
        match self {
            GodotScheme::Res => "res://",
            GodotScheme::User => "user://",
        }
    }

    fn short_prefix(&self) -> &'static str {
        match self {
            GodotScheme::Res => "res:",
            GodotScheme::User => "user:",
        }
    }

    fn name(&self) -> &'static str {
        match self {
            GodotScheme::Res => "res",
            GodotScheme::User => "user",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ParsedRanges {
    Single(ByteRange),
    Multi(Vec<ByteRange>),
}

const MULTIPART_BOUNDARY: &str = "godot_cef_multipart_boundary";

fn parse_single_range(range_spec: &str, file_size: u64) -> Option<ByteRange> {
    // Empty file has no valid byte ranges
    if file_size == 0 {
        return None;
    }

    let parts: Vec<&str> = range_spec.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start_str = parts[0].trim();
    let end_str = parts[1].trim();

    if !start_str.is_empty() {
        // "start-" or "start-end"
        match start_str.parse::<u64>() {
            Ok(start) => {
                if start >= file_size {
                    return None;
                }
                let end = if end_str.is_empty() {
                    file_size - 1
                } else {
                    end_str.parse::<u64>().ok()?.min(file_size - 1)
                };
                if start <= end {
                    Some(ByteRange { start, end })
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    } else if !end_str.is_empty() {
        // "-suffix_length"
        match end_str.parse::<u64>() {
            Ok(suffix_len) if suffix_len > 0 => {
                let start = file_size.saturating_sub(suffix_len);
                Some(ByteRange {
                    start,
                    end: file_size - 1,
                })
            }
            _ => None,
        }
    } else {
        None
    }
}

fn parse_range_header(range_str: &str, file_size: u64) -> Option<ParsedRanges> {
    if range_str.is_empty() || !range_str.starts_with("bytes=") {
        return None;
    }

    let range_part = &range_str[6..];

    if range_part.contains(',') {
        // Multi-range request
        let ranges: Vec<ByteRange> = range_part
            .split(',')
            .filter_map(|spec| parse_single_range(spec.trim(), file_size))
            .collect();

        if ranges.is_empty() {
            None
        } else if ranges.len() == 1 {
            Some(ParsedRanges::Single(ranges.into_iter().next().unwrap()))
        } else {
            Some(ParsedRanges::Multi(ranges))
        }
    } else {
        // Single range request
        parse_single_range(range_part, file_size).map(ParsedRanges::Single)
    }
}

fn build_multipart_body(
    ranges: &[ByteRange],
    file: &mut godot::classes::FileAccess,
    mime_type: &str,
    file_size: u64,
) -> Vec<u8> {
    let mut body = Vec::new();

    for range in ranges {
        // Add boundary
        body.extend_from_slice(format!("--{}\r\n", MULTIPART_BOUNDARY).as_bytes());

        // Add headers for this part
        body.extend_from_slice(format!("Content-Type: {}\r\n", mime_type).as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Range: bytes {}-{}/{}\r\n",
                range.start, range.end, file_size
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"\r\n");

        // Add the data for this range
        let content_size = (range.end - range.start + 1) as i64;
        file.seek(range.start);
        let buffer = file.get_buffer(content_size);
        body.extend_from_slice(buffer.as_slice());
        body.extend_from_slice(b"\r\n");
    }

    // Add closing boundary
    body.extend_from_slice(format!("--{}--\r\n", MULTIPART_BOUNDARY).as_bytes());

    body
}

fn parse_godot_url(url: &str, scheme: GodotScheme) -> String {
    let path = url
        .strip_prefix(scheme.prefix())
        .or_else(|| url.strip_prefix(scheme.short_prefix()))
        .unwrap_or(url);

    let mut full_path = format!("{}{}", scheme.prefix(), path);

    // Determine whether the last path component (ignoring trailing '/')
    // has an extension (i.e., contains a dot). This avoids treating dots
    // in parent directory names as file extensions.
    let trimmed = full_path.trim_end_matches('/');
    let last_segment = trimmed.rsplit('/').next().unwrap_or("");
    let has_extension = last_segment.contains('.');

    if full_path.ends_with('/') || !has_extension || full_path.ends_with(scheme.prefix()) {
        if !full_path.ends_with('/') {
            full_path.push('/');
        }
        full_path.push_str("index.html");
    }

    full_path
}

#[derive(Clone, Default)]
struct ResourceState {
    data: Vec<u8>,
    offset: usize,
    status_code: i32,
    mime_type: String,
    response_content_type: String,
    error_message: Option<String>,
    total_file_size: u64,
    range_start: Option<u64>,
    range_end: Option<u64>,
    is_multipart: bool,
}

#[derive(Clone)]
pub struct GodotResourceHandler {
    state: RefCell<ResourceState>,
    scheme: GodotScheme,
}

impl GodotResourceHandler {
    pub fn new(scheme: GodotScheme) -> Self {
        Self {
            state: RefCell::new(ResourceState::default()),
            scheme,
        }
    }
}

wrap_resource_handler! {
    pub struct GodotResourceHandlerImpl {
        handler: GodotResourceHandler,
    }

    impl ResourceHandler {
        fn open(
            &self,
            request: Option<&mut cef::Request>,
            handle_request: Option<&mut ::std::os::raw::c_int>,
            _callback: Option<&mut cef::Callback>,
        ) -> ::std::os::raw::c_int {
            let Some(request) = request else {
                return false as _;
            };

            let url_cef = request.url();
            let url = CefStringUtf16::from(&url_cef).to_string();
            let godot_path = parse_godot_url(&url, self.handler.scheme);
            let gstring_path = GString::from(&godot_path);

            let mut state = self.handler.state.borrow_mut();

            if !FileAccess::file_exists(&gstring_path) {
                state.status_code = 404;
                state.mime_type = "text/plain".to_string();
                state.error_message = Some(format!("File not found: {}", godot_path));
                state.data = state
                    .error_message
                    .as_ref()
                    .unwrap()
                    .as_bytes()
                    .to_vec();

                if let Some(handle_request) = handle_request {
                    *handle_request = true as _;
                }
                return true as _;
            }

            let range_header = request.header_by_name(Some(&"Range".into()));
            let range_str = CefStringUtf16::from(&range_header).to_string();

            match FileAccess::open(&gstring_path, ModeFlags::READ) {
                Some(mut file) => {
                    let file_size = file.get_length();
                    state.total_file_size = file_size;

                    let path = PathBuf::from(&godot_path);
                    let extension = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    state.mime_type = get_mime_type(extension).to_string();
                    state.response_content_type = state.mime_type.clone();

                    // Parse `Range` header. Supports single ranges ("bytes=start-end",
                    // "bytes=start-", "bytes=-suffix_length") and multi-range requests
                    // ("bytes=0-100,200-300").
                    match parse_range_header(&range_str, file_size) {
                        Some(ParsedRanges::Single(range)) => {
                            if range.start >= file_size {
                                state.status_code = 416;
                                state.data = Vec::new();
                                state.range_start = None;
                                state.range_end = None;
                                state.is_multipart = false;
                            } else {
                                let content_size = (range.end - range.start + 1) as i64;
                                file.seek(range.start);
                                let buffer = file.get_buffer(content_size);
                                state.data = buffer.as_slice().to_vec();
                                state.status_code = 206;
                                state.range_start = Some(range.start);
                                state.range_end = Some(range.end);
                                state.is_multipart = false;
                                state.offset = 0;
                            }
                        }
                        Some(ParsedRanges::Multi(ranges)) => {
                            // Build multipart response
                            state.data = build_multipart_body(&ranges, &mut file, &state.mime_type, file_size);
                            state.status_code = 206;
                            state.response_content_type = format!(
                                "multipart/byteranges; boundary={}",
                                MULTIPART_BOUNDARY
                            );
                            state.range_start = None;
                            state.range_end = None;
                            state.is_multipart = true;
                            state.offset = 0;
                        }
                        None => {
                            // No range or invalid range - return entire file
                            let buffer = file.get_buffer(file_size as i64);
                            state.data = buffer.as_slice().to_vec();
                            state.status_code = 200;
                            state.range_start = None;
                            state.range_end = None;
                            state.is_multipart = false;
                            state.offset = 0;
                        }
                    }
                }
                None => {
                    state.status_code = 500;
                    state.mime_type = "text/plain".to_string();
                    state.error_message = Some(format!("Failed to open file: {}", godot_path));
                    state.data = state
                        .error_message
                        .as_ref()
                        .unwrap()
                        .as_bytes()
                        .to_vec();
                }
            }

            if let Some(handle_request) = handle_request {
                *handle_request = true as _;
            }

            true as _
        }

        fn response_headers(
            &self,
            response: Option<&mut cef::Response>,
            response_length: Option<&mut i64>,
            _redirect_url: Option<&mut cef::CefStringUtf16>,
        ) {
            let state = self.handler.state.borrow();

            if let Some(response) = response {
                response.set_status(state.status_code);

                let status_text = match state.status_code {
                    200 => "OK",
                    206 => "Partial Content",
                    404 => "Not Found",
                    416 => "Range Not Satisfiable",
                    500 => "Internal Server Error",
                    _ => "Unknown",
                };
                response.set_status_text(Some(&status_text.into()));

                response.set_mime_type(Some(&state.response_content_type.as_str().into()));

                response.set_header_by_name(Some(&"Content-Type".into()), Some(&state.response_content_type.as_str().into()), true as _);
                response.set_header_by_name(Some(&"Access-Control-Allow-Origin".into()), Some(&"*".into()), true as _);
                response.set_header_by_name(Some(&"Accept-Ranges".into()), Some(&"bytes".into()), true as _);

                if state.status_code == 206 && !state.is_multipart {
                    if let (Some(start), Some(end)) = (state.range_start, state.range_end) {
                        let value: CefStringUtf16 = format!("bytes {}-{}/{}", start, end, state.total_file_size).as_str().into();
                        response.set_header_by_name(Some(&"Content-Range".into()), Some(&value), true as _);
                    }
                } else if state.status_code == 416 {
                    let value: CefStringUtf16 = format!("bytes */{}", state.total_file_size).as_str().into();
                    response.set_header_by_name(Some(&"Content-Range".into()), Some(&value), true as _);
                }
            }

            if let Some(response_length) = response_length {
                *response_length = state.data.len() as i64;
            }
        }

        fn read(
            &self,
            data_out: *mut u8,
            bytes_to_read: ::std::os::raw::c_int,
            bytes_read: Option<&mut ::std::os::raw::c_int>,
            _callback: Option<&mut cef::ResourceReadCallback>,
        ) -> ::std::os::raw::c_int {
            let mut state = self.handler.state.borrow_mut();

            let bytes_to_read = bytes_to_read as usize;
            let remaining = state.data.len().saturating_sub(state.offset);

            if remaining == 0 {
                if let Some(bytes_read) = bytes_read {
                    *bytes_read = 0;
                }
                return false as _;
            }

            let to_copy = remaining.min(bytes_to_read);
            if data_out.is_null() { return false as _; }

            unsafe {
                std::ptr::copy_nonoverlapping(
                    state.data.as_ptr().add(state.offset),
                    data_out,
                    to_copy,
                );
            }

            state.offset += to_copy;

            if let Some(bytes_read) = bytes_read {
                *bytes_read = to_copy as _;
            }

            true as _
        }

        fn skip(
            &self,
            bytes_to_skip: i64,
            bytes_skipped: Option<&mut i64>,
            _callback: Option<&mut cef::ResourceSkipCallback>,
        ) -> ::std::os::raw::c_int {
            let mut state = self.handler.state.borrow_mut();

            let bytes_to_skip = bytes_to_skip as usize;
            let remaining = state.data.len().saturating_sub(state.offset);
            let to_skip = remaining.min(bytes_to_skip);

            state.offset += to_skip;

            if let Some(bytes_skipped) = bytes_skipped {
                *bytes_skipped = to_skip as i64;
            }

            true as _
        }

        fn cancel(&self) {}
    }
}

impl GodotResourceHandlerImpl {
    pub fn build(handler: GodotResourceHandler) -> ResourceHandler {
        Self::new(handler)
    }
}

#[derive(Clone)]
pub struct GodotSchemeHandler {
    scheme: GodotScheme,
}

impl GodotSchemeHandler {
    pub fn new(scheme: GodotScheme) -> Self {
        Self { scheme }
    }
}

wrap_scheme_handler_factory! {
    pub struct GodotSchemeHandlerFactory {
        handler: GodotSchemeHandler,
    }

    impl SchemeHandlerFactory {
        fn create(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _scheme_name: Option<&cef::CefString>,
            _request: Option<&mut cef::Request>,
        ) -> Option<ResourceHandler> {
            Some(GodotResourceHandlerImpl::build(GodotResourceHandler::new(self.handler.scheme)))
        }
    }
}

impl GodotSchemeHandlerFactory {
    pub fn build(handler: GodotSchemeHandler) -> SchemeHandlerFactory {
        Self::new(handler)
    }
}

fn register_scheme_handler_on_context(context: &mut cef::RequestContext, scheme: GodotScheme) {
    use cef::ImplRequestContext;
    let mut factory = GodotSchemeHandlerFactory::build(GodotSchemeHandler::new(scheme));
    context.register_scheme_handler_factory(
        Some(&scheme.name().into()),
        Some(&"".into()),
        Some(&mut factory),
    );
}

pub fn register_res_scheme_handler_on_context(context: &mut cef::RequestContext) {
    register_scheme_handler_on_context(context, GodotScheme::Res);
}

pub fn register_user_scheme_handler_on_context(context: &mut cef::RequestContext) {
    register_scheme_handler_on_context(context, GodotScheme::User);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_res_url() {
        assert_eq!(
            parse_godot_url("res://ui/index.html", GodotScheme::Res),
            "res://ui/index.html"
        );
        assert_eq!(
            parse_godot_url("res://folder/", GodotScheme::Res),
            "res://folder/index.html"
        );
        assert_eq!(
            parse_godot_url("res://folder", GodotScheme::Res),
            "res://folder/index.html"
        );
        assert_eq!(
            parse_godot_url("ui/style.css", GodotScheme::Res),
            "res://ui/style.css"
        );
    }

    #[test]
    fn test_parse_user_url() {
        assert_eq!(
            parse_godot_url("user://data/index.html", GodotScheme::User),
            "user://data/index.html"
        );
        assert_eq!(
            parse_godot_url("user://folder/", GodotScheme::User),
            "user://folder/index.html"
        );
        assert_eq!(
            parse_godot_url("user://folder", GodotScheme::User),
            "user://folder/index.html"
        );
        assert_eq!(
            parse_godot_url("data/style.css", GodotScheme::User),
            "user://data/style.css"
        );
    }

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

    // Range header parsing tests
    const TEST_FILE_SIZE: u64 = 1000;

    // Helper to create a single range result
    fn single(start: u64, end: u64) -> Option<ParsedRanges> {
        Some(ParsedRanges::Single(ByteRange { start, end }))
    }

    // Helper to create a multi range result
    fn multi(ranges: Vec<(u64, u64)>) -> Option<ParsedRanges> {
        Some(ParsedRanges::Multi(
            ranges
                .into_iter()
                .map(|(start, end)| ByteRange { start, end })
                .collect(),
        ))
    }

    #[test]
    fn test_range_header_empty() {
        assert_eq!(parse_range_header("", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_no_bytes_prefix() {
        // Invalid: missing "bytes=" prefix
        assert_eq!(parse_range_header("0-100", TEST_FILE_SIZE), None);
        assert_eq!(parse_range_header("range=0-100", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_single_range_start_end() {
        // bytes=start-end
        assert_eq!(
            parse_range_header("bytes=0-100", TEST_FILE_SIZE),
            single(0, 100)
        );
        assert_eq!(
            parse_range_header("bytes=100-200", TEST_FILE_SIZE),
            single(100, 200)
        );
        assert_eq!(
            parse_range_header("bytes=0-999", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=500-999", TEST_FILE_SIZE),
            single(500, 999)
        );
    }

    #[test]
    fn test_range_header_open_ended() {
        // bytes=start- (from start to end of file)
        assert_eq!(
            parse_range_header("bytes=0-", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=100-", TEST_FILE_SIZE),
            single(100, 999)
        );
        assert_eq!(
            parse_range_header("bytes=500-", TEST_FILE_SIZE),
            single(500, 999)
        );
        assert_eq!(
            parse_range_header("bytes=999-", TEST_FILE_SIZE),
            single(999, 999)
        );
    }

    #[test]
    fn test_range_header_suffix_length() {
        // bytes=-suffix_length (last N bytes)
        assert_eq!(
            parse_range_header("bytes=-100", TEST_FILE_SIZE),
            single(900, 999)
        );
        assert_eq!(
            parse_range_header("bytes=-500", TEST_FILE_SIZE),
            single(500, 999)
        );
        assert_eq!(
            parse_range_header("bytes=-1", TEST_FILE_SIZE),
            single(999, 999)
        );

        // Suffix length >= file size should return entire file
        assert_eq!(
            parse_range_header("bytes=-1000", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=-2000", TEST_FILE_SIZE),
            single(0, 999)
        );
    }

    #[test]
    fn test_range_header_suffix_zero() {
        // bytes=-0 is invalid (suffix length must be > 0)
        assert_eq!(parse_range_header("bytes=-0", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_multi_range() {
        // Multi-range requests should now be properly parsed
        assert_eq!(
            parse_range_header("bytes=0-100,200-300", TEST_FILE_SIZE),
            multi(vec![(0, 100), (200, 300)])
        );
        assert_eq!(
            parse_range_header("bytes=0-100,200-300,400-500", TEST_FILE_SIZE),
            multi(vec![(0, 100), (200, 300), (400, 500)])
        );
        assert_eq!(
            parse_range_header("bytes=0-50,100-150,200-250", TEST_FILE_SIZE),
            multi(vec![(0, 50), (100, 150), (200, 250)])
        );
    }

    #[test]
    fn test_range_header_multi_range_with_open_ended() {
        // Multi-range with open-ended ranges
        assert_eq!(
            parse_range_header("bytes=0-100,500-", TEST_FILE_SIZE),
            multi(vec![(0, 100), (500, 999)])
        );
        assert_eq!(
            parse_range_header("bytes=-100,0-50", TEST_FILE_SIZE),
            multi(vec![(900, 999), (0, 50)])
        );
    }

    #[test]
    fn test_range_header_multi_range_with_invalid_parts() {
        // Multi-range with some invalid parts - invalid parts are skipped
        // Only "0-100" is valid, "abc-def" is skipped, result is single range
        assert_eq!(
            parse_range_header("bytes=0-100,abc-def", TEST_FILE_SIZE),
            single(0, 100)
        );

        // All parts invalid
        assert_eq!(
            parse_range_header("bytes=abc-def,xyz-123", TEST_FILE_SIZE),
            None
        );
    }

    #[test]
    fn test_range_header_multi_range_empty_parts() {
        // Edge case: empty parts after comma (invalid parts filtered out)
        // "0-100" valid, empty string invalid, result is single
        assert_eq!(
            parse_range_header("bytes=0-100,", TEST_FILE_SIZE),
            single(0, 100)
        );

        // Leading comma - empty first part filtered out
        assert_eq!(
            parse_range_header("bytes=,0-100", TEST_FILE_SIZE),
            single(0, 100)
        );
    }

    #[test]
    fn test_range_header_multi_range_whitespace() {
        // Whitespace around ranges in multi-range
        assert_eq!(
            parse_range_header("bytes= 0-100 , 200-300 ", TEST_FILE_SIZE),
            multi(vec![(0, 100), (200, 300)])
        );
    }

    #[test]
    fn test_range_header_whitespace() {
        // Whitespace around numbers should be trimmed
        assert_eq!(
            parse_range_header("bytes= 0 - 100 ", TEST_FILE_SIZE),
            single(0, 100)
        );
        assert_eq!(
            parse_range_header("bytes=  100  -  ", TEST_FILE_SIZE),
            single(100, 999)
        );
        assert_eq!(
            parse_range_header("bytes=  -  100  ", TEST_FILE_SIZE),
            single(900, 999)
        );
    }

    #[test]
    fn test_range_header_invalid_numbers() {
        // Invalid start number
        assert_eq!(parse_range_header("bytes=abc-100", TEST_FILE_SIZE), None);
        assert_eq!(parse_range_header("bytes=-1x-100", TEST_FILE_SIZE), None);

        // Invalid end number (but valid start - end clamped to file size - 1)
        assert_eq!(parse_range_header("bytes=0-abc", TEST_FILE_SIZE), None);

        // Invalid suffix
        assert_eq!(parse_range_header("bytes=-abc", TEST_FILE_SIZE), None);

        // Negative numbers (parsed as invalid)
        assert_eq!(parse_range_header("bytes=--100", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_malformed() {
        // Missing dash
        assert_eq!(parse_range_header("bytes=100", TEST_FILE_SIZE), None);

        // Multiple dashes
        assert_eq!(parse_range_header("bytes=0-100-200", TEST_FILE_SIZE), None);

        // Empty both sides
        assert_eq!(parse_range_header("bytes=-", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_range_clamping() {
        // End value beyond file size should be clamped
        assert_eq!(
            parse_range_header("bytes=0-5000", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=500-2000", TEST_FILE_SIZE),
            single(500, 999)
        );
    }

    #[test]
    fn test_range_header_start_beyond_file() {
        // Start beyond file size is invalid
        assert_eq!(parse_range_header("bytes=1000-2000", TEST_FILE_SIZE), None);
        assert_eq!(parse_range_header("bytes=5000-", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_edge_cases() {
        // Very small file (1 byte)
        assert_eq!(parse_range_header("bytes=0-0", 1), single(0, 0));
        assert_eq!(parse_range_header("bytes=0-", 1), single(0, 0));
        assert_eq!(parse_range_header("bytes=-1", 1), single(0, 0));
        assert_eq!(parse_range_header("bytes=1-", 1), None); // start >= file_size

        // Very large numbers
        let large_file: u64 = 10_000_000_000;
        assert_eq!(
            parse_range_header("bytes=0-9999999999", large_file),
            single(0, 9999999999)
        );
        assert_eq!(
            parse_range_header("bytes=5000000000-", large_file),
            single(5000000000, 9999999999)
        );
    }

    #[test]
    fn test_range_header_zero_file_size() {
        // Zero file size - all ranges are invalid since there are no bytes
        assert_eq!(parse_range_header("bytes=0-0", 0), None);
        assert_eq!(parse_range_header("bytes=0-", 0), None);
        assert_eq!(parse_range_header("bytes=-1", 0), None);
        assert_eq!(parse_range_header("bytes=-100", 0), None);

        // Multi-range on empty file
        assert_eq!(parse_range_header("bytes=0-0,1-1", 0), None);
    }

    #[test]
    fn test_range_header_multi_range_many_ranges() {
        // Test with many ranges
        assert_eq!(
            parse_range_header("bytes=0-10,100-110,200-210,300-310,400-410", TEST_FILE_SIZE),
            multi(vec![
                (0, 10),
                (100, 110),
                (200, 210),
                (300, 310),
                (400, 410)
            ])
        );
    }

    #[test]
    fn test_range_header_overlapping_ranges() {
        // Overlapping ranges are allowed per HTTP spec (server can coalesce or serve as-is)
        assert_eq!(
            parse_range_header("bytes=0-100,50-150", TEST_FILE_SIZE),
            multi(vec![(0, 100), (50, 150)])
        );
    }

    #[test]
    fn test_single_range_helper() {
        // Test parse_single_range directly
        assert_eq!(
            parse_single_range("0-100", TEST_FILE_SIZE),
            Some(ByteRange { start: 0, end: 100 })
        );
        assert_eq!(
            parse_single_range("100-", TEST_FILE_SIZE),
            Some(ByteRange {
                start: 100,
                end: 999
            })
        );
        assert_eq!(
            parse_single_range("-100", TEST_FILE_SIZE),
            Some(ByteRange {
                start: 900,
                end: 999
            })
        );
        assert_eq!(parse_single_range("invalid", TEST_FILE_SIZE), None);
        assert_eq!(parse_single_range("", TEST_FILE_SIZE), None);
    }
}
