//! CEF resource and scheme handler implementations.
//!
//! This module provides the CEF callbacks that serve resources from
//! Godot's filesystem in response to `res://` and `user://` URL requests.

use cef::{
    CefStringUtf16, ImplRequest, ImplResourceHandler, ImplResponse, ImplSchemeHandlerFactory,
    ResourceHandler, SchemeHandlerFactory, WrapResourceHandler, WrapSchemeHandlerFactory, rc::Rc,
    wrap_resource_handler, wrap_scheme_handler_factory,
};
use godot::classes::FileAccess;
use godot::classes::file_access::ModeFlags;
use godot::prelude::*;
use percent_encoding::percent_decode_str;
use std::cell::RefCell;
use std::path::PathBuf;
use url::Url;

use super::GodotScheme;
use super::mime::get_mime_type;
use super::multipart::{
    MULTIPART_BOUNDARY, MultipartStreamState, read_multipart_streaming, skip_multipart_streaming,
};
use super::range::{ParsedRanges, parse_range_header};

/// Validate that a string contains only valid percent-encoded sequences.
///
/// Returns `false` if there are malformed percent sequences like `%`, `%G`, `%2`.
/// This is stricter than the percent-encoding crate which leaves invalid sequences as-is.
fn has_valid_percent_encoding(input: &str) -> bool {
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match (chars.next(), chars.next()) {
                (Some(h1), Some(h2)) if h1.is_ascii_hexdigit() && h2.is_ascii_hexdigit() => {}
                _ => return false,
            }
        }
    }

    true
}

/// Check if a URL string contains path traversal patterns in encoded form.
///
/// This checks for traversal patterns BEFORE URL normalization to prevent
/// the URL parser from resolving away the `..` segments. Handles:
/// - Plain `..`
/// - Percent-encoded dots: `%2e%2e`, `%2E%2E`
/// - Backslash variants: `..\\`, `%5c`
fn contains_path_traversal_encoded(url: &str) -> bool {
    if let Ok(decoded) = percent_decode_str(url).decode_utf8()
        && contains_path_traversal(&decoded)
    {
        return true;
    }
    contains_path_traversal(url)
}

/// Check if a path contains path traversal patterns.
///
/// Returns `true` if the path is suspicious and should be rejected.
/// Handles both forward slashes and backslashes (for Windows compatibility).
fn contains_path_traversal(decoded_path: &str) -> bool {
    // Normalize backslashes to forward slashes to catch Windows-style traversal
    let normalized = decoded_path.replace('\\', "/");

    // Check each component for ".." traversal
    for component in normalized.split('/') {
        if component == ".." {
            return true;
        }
    }

    false
}

/// Parse a URL into a Godot filesystem path.
///
/// Uses the `percent-encoding` crate for robust percent-decoding. Returns `None`
/// if the URL contains path traversal patterns, invalid encoding, or other
/// security concerns.
///
/// Note: `res://` and `user://` are custom Godot schemes that don't follow
/// standard URL authority rules - everything after `://` is treated as the path,
/// not as host + path. We intentionally do NOT use URL normalization to avoid
/// silently resolving `..` traversal patterns.
pub(crate) fn parse_godot_url(url_str: &str, scheme: GodotScheme) -> Option<String> {
    if contains_path_traversal_encoded(url_str) {
        return None;
    }

    // For custom schemes like res://, the url crate treats the first path
    // component as a host, so we reconstruct the full path ourselves.
    let path_encoded = if let Ok(parsed_url) = Url::parse(url_str) {
        if parsed_url.scheme() == scheme.name() {
            let host = parsed_url.host_str().unwrap_or("");
            let url_path = parsed_url.path();

            if host.is_empty() {
                url_path.strip_prefix('/').unwrap_or(url_path).to_string()
            } else {
                // When the URL is of the form `res://file.html?v=1` or `user://data.json#frag`,
                // `Url::parse` will treat `file.html`/`data.json` as the host and `/` as the path.
                // In those cases, we must *not* append a trailing slash, otherwise we end up with
                // `file.html/` which `finalize_godot_path` then treats as a directory.
                if url_path == "/" {
                    // Reconstruct what was after `scheme://` up to any `?` or `#` to see if there
                    // was an explicit trailing slash in the original URL.
                    let scheme_prefix = format!("{}://", scheme.name());
                    let had_trailing_slash = url_str
                        .strip_prefix(&scheme_prefix)
                        .map(|rest| {
                            let end = rest.find(['?', '#']).unwrap_or(rest.len());
                            rest[..end].ends_with('/')
                        })
                        // If we can't determine it reliably, preserve the previous behavior.
                        .unwrap_or(true);

                    if !had_trailing_slash {
                        host.to_string()
                    } else {
                        format!("{}{}", host, url_path)
                    }
                } else {
                    format!("{}{}", host, url_path)
                }
            }
        } else {
            return parse_godot_url_manual(url_str, scheme);
        }
    } else {
        return parse_godot_url_manual(url_str, scheme);
    };

    if !has_valid_percent_encoding(&path_encoded) {
        return None;
    }

    let path = percent_decode_str(&path_encoded)
        .decode_utf8()
        .ok()?
        .into_owned();

    if path.contains('\0') {
        return None;
    }

    if contains_path_traversal(&path) {
        return None;
    }

    finalize_godot_path(&path, scheme)
}

/// Manual URL parsing fallback for relative paths or when URL crate fails.
fn parse_godot_url_manual(url_str: &str, scheme: GodotScheme) -> Option<String> {
    if contains_path_traversal_encoded(url_str) {
        return None;
    }

    // Strip query parameters and URL fragments
    let url_without_query = url_str
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(url_str);
    let url_clean = url_without_query
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(url_without_query);

    let path_encoded = url_clean
        .strip_prefix(scheme.prefix())
        .or_else(|| url_clean.strip_prefix(scheme.short_prefix()))
        .unwrap_or(url_clean);

    if !has_valid_percent_encoding(path_encoded) {
        return None;
    }

    let path = percent_decode_str(path_encoded)
        .decode_utf8()
        .ok()?
        .into_owned();

    if path.contains('\0') {
        return None;
    }

    if contains_path_traversal(&path) {
        return None;
    }

    finalize_godot_path(&path, scheme)
}

/// Finalize a decoded path into a full Godot path, adding index.html if needed.
fn finalize_godot_path(path: &str, scheme: GodotScheme) -> Option<String> {
    let path = path.strip_prefix('/').unwrap_or(path);
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

    Some(full_path)
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
    multipart_stream: Option<MultipartStreamState>,
    file_path: Option<String>,
    open_file: Option<Gd<FileAccess>>,
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

            let mut state = self.handler.state.borrow_mut();

            // Reject paths with traversal patterns (returns 403 Forbidden)
            let godot_path = match parse_godot_url(&url, self.handler.scheme) {
                Some(path) => path,
                None => {
                    state.status_code = 403;
                    state.mime_type = "text/plain".to_string();
                    state.response_content_type = "text/plain".to_string();
                    state.error_message = Some("Forbidden: Invalid path".to_string());
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
            };
            let gstring_path = GString::from(&godot_path);

            if !FileAccess::file_exists(&gstring_path) {
                state.status_code = 404;
                state.mime_type = "text/plain".to_string();
                state.response_content_type = "text/plain".to_string();
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
                                let content_size_u64 = range.end.saturating_sub(range.start).saturating_add(1);
                                let content_size = i64::try_from(content_size_u64).unwrap_or(i64::MAX);
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
                            // Set up streaming multipart response (data loaded on-demand during read)
                            let stream_state = MultipartStreamState::new(
                                ranges,
                                &state.mime_type,
                                file_size,
                            );
                            state.status_code = 206;
                            state.response_content_type = format!(
                                "multipart/byteranges; boundary={}",
                                MULTIPART_BOUNDARY
                            );
                            state.range_start = None;
                            state.range_end = None;
                            state.is_multipart = true;
                            state.file_path = Some(godot_path.clone());
                            state.multipart_stream = Some(stream_state);
                            state.data = Vec::new(); // Data will be streamed, not buffered
                            state.offset = 0;
                        }
                        None => {
                            let buffer_size = i64::try_from(file_size).unwrap_or(i64::MAX);
                            let buffer = file.get_buffer(buffer_size);
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
                    state.response_content_type = "text/plain".to_string();
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
                    403 => "Forbidden",
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
                // For streaming multipart responses, use pre-calculated total size
                if let Some(ref stream) = state.multipart_stream {
                    *response_length = stream.total_size as i64;
                } else {
                    *response_length = state.data.len() as i64;
                }
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

            if data_out.is_null() {
                return false as _;
            }

            let bytes_to_read = bytes_to_read as usize;

            // Handle streaming multipart responses
            if state.multipart_stream.is_some() && state.file_path.is_some() {
                let file_path = state.file_path.clone().unwrap();
                let mime_type = state.mime_type.clone();
                let file_size = state.total_file_size;

                let ResourceState {
                    multipart_stream,
                    open_file,
                    ..
                } = &mut *state;

                let written = read_multipart_streaming(
                    multipart_stream.as_mut().unwrap(),
                    &file_path,
                    &mime_type,
                    file_size,
                    open_file,
                    data_out,
                    bytes_to_read,
                );

                if let Some(bytes_read) = bytes_read {
                    *bytes_read = written as _;
                }

                return (written > 0) as _;
            }

            // Handle buffered (non-streaming) responses
            let remaining = state.data.len().saturating_sub(state.offset);

            if remaining == 0 {
                if let Some(bytes_read) = bytes_read {
                    *bytes_read = 0;
                }
                return false as _;
            }

            let to_copy = remaining.min(bytes_to_read);

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

            let bytes_to_skip = bytes_to_skip.max(0) as usize;

            // Handle streaming multipart responses
            if state.multipart_stream.is_some() {
                let mime_type = state.mime_type.clone();
                let file_size = state.total_file_size;
                let stream = state.multipart_stream.as_mut().unwrap();

                let skipped = skip_multipart_streaming(
                    stream,
                    &mime_type,
                    file_size,
                    bytes_to_skip,
                );

                if let Some(bytes_skipped) = bytes_skipped {
                    *bytes_skipped = skipped as i64;
                }

                return true as _;
            }

            // Handle buffered (non-streaming) responses
            let remaining = state.data.len().saturating_sub(state.offset);
            let to_skip = remaining.min(bytes_to_skip);

            state.offset += to_skip;

            if let Some(bytes_skipped) = bytes_skipped {
                *bytes_skipped = to_skip as i64;
            }

            true as _
        }

        fn cancel(&self) {
            let mut state = self.handler.state.borrow_mut();

            // If a multipart stream is active, explicitly release its resources
            if state.multipart_stream.is_some() {
                state.multipart_stream = None;
                state.open_file = None;
            }
        }
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
            Some("res://ui/index.html".to_string())
        );
        assert_eq!(
            parse_godot_url("res://folder/", GodotScheme::Res),
            Some("res://folder/index.html".to_string())
        );
        assert_eq!(
            parse_godot_url("res://folder", GodotScheme::Res),
            Some("res://folder/index.html".to_string())
        );
        assert_eq!(
            parse_godot_url("ui/style.css", GodotScheme::Res),
            Some("res://ui/style.css".to_string())
        );
    }

    #[test]
    fn test_parse_url_strips_query_params() {
        assert_eq!(
            parse_godot_url("res://file.html?v=1", GodotScheme::Res),
            Some("res://file.html".to_string())
        );
        assert_eq!(
            parse_godot_url("res://ui/script.js?cache=false&v=2", GodotScheme::Res),
            Some("res://ui/script.js".to_string())
        );
        assert_eq!(
            parse_godot_url("user://data.json?timestamp=12345", GodotScheme::User),
            Some("user://data.json".to_string())
        );
    }

    #[test]
    fn test_parse_url_strips_fragments() {
        assert_eq!(
            parse_godot_url("res://file.html#section", GodotScheme::Res),
            Some("res://file.html".to_string())
        );
        assert_eq!(
            parse_godot_url("res://docs/page.html#heading-1", GodotScheme::Res),
            Some("res://docs/page.html".to_string())
        );
        assert_eq!(
            parse_godot_url("user://readme.html#intro", GodotScheme::User),
            Some("user://readme.html".to_string())
        );
    }

    #[test]
    fn test_parse_url_strips_query_and_fragment() {
        assert_eq!(
            parse_godot_url("res://file.html?v=1#section", GodotScheme::Res),
            Some("res://file.html".to_string())
        );
        assert_eq!(
            parse_godot_url("res://app.js?debug=true#line-50", GodotScheme::Res),
            Some("res://app.js".to_string())
        );
    }

    #[test]
    fn test_parse_user_url() {
        assert_eq!(
            parse_godot_url("user://data/index.html", GodotScheme::User),
            Some("user://data/index.html".to_string())
        );
        assert_eq!(
            parse_godot_url("user://folder/", GodotScheme::User),
            Some("user://folder/index.html".to_string())
        );
        assert_eq!(
            parse_godot_url("user://folder", GodotScheme::User),
            Some("user://folder/index.html".to_string())
        );
        assert_eq!(
            parse_godot_url("data/style.css", GodotScheme::User),
            Some("user://data/style.css".to_string())
        );
    }

    #[test]
    fn test_rejects_path_traversal() {
        // Basic traversal attempts
        assert_eq!(
            parse_godot_url("res://../etc/passwd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://../../etc/passwd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://folder/../../../etc/passwd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("user://../sensitive", GodotScheme::User),
            None
        );

        // Traversal in middle of path
        assert_eq!(
            parse_godot_url("res://a/b/../../../c", GodotScheme::Res),
            None
        );

        // Traversal at end
        assert_eq!(parse_godot_url("res://folder/..", GodotScheme::Res), None);

        // URL-encoded traversal attempts
        assert_eq!(
            parse_godot_url("res://%2e%2e/etc/passwd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://%2E%2E/etc/passwd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://folder%2f..%2f../etc", GodotScheme::Res),
            None
        );

        // Backslash-based traversal (Windows-style)
        assert_eq!(
            parse_godot_url("res://..\\etc\\passwd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://folder\\..\\..\\etc", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://a\\b\\..\\..\\..\\c", GodotScheme::Res),
            None
        );

        // URL-encoded backslash traversal (%5c = backslash)
        assert_eq!(
            parse_godot_url("res://..%5cetc%5cpasswd", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://..%5Cetc%5Cpasswd", GodotScheme::Res),
            None
        );
    }

    #[test]
    fn test_rejects_null_bytes() {
        // URL-encoded null byte (%00)
        assert_eq!(
            parse_godot_url("res://file%00.html", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://folder%00/file.txt", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("res://%00file.html", GodotScheme::Res),
            None
        );
        assert_eq!(
            parse_godot_url("user://data%00.json", GodotScheme::User),
            None
        );
    }

    #[test]
    fn test_allows_dots_in_filenames() {
        // Single dots and dots in filenames should be allowed
        assert_eq!(
            parse_godot_url("res://file.name.html", GodotScheme::Res),
            Some("res://file.name.html".to_string())
        );
        assert_eq!(
            parse_godot_url("res://.hidden/file.txt", GodotScheme::Res),
            Some("res://.hidden/file.txt".to_string())
        );
        // URL normalization removes meaningless "." path segments (per RFC 3986)
        // This is correct behavior: "./file.txt" and "file.txt" refer to the same resource
        assert_eq!(
            parse_godot_url("res://folder/./file.txt", GodotScheme::Res),
            Some("res://folder/file.txt".to_string())
        );
    }

    #[test]
    fn test_has_valid_percent_encoding() {
        // Valid encodings
        assert!(has_valid_percent_encoding("hello%20world"));
        assert!(has_valid_percent_encoding("a%20b%20c"));
        assert!(has_valid_percent_encoding("file%20name.txt"));
        assert!(has_valid_percent_encoding("%21%40%23"));
        assert!(has_valid_percent_encoding("%2F%2f")); // Mixed case hex
        assert!(has_valid_percent_encoding("plain.txt")); // No encoding
        assert!(has_valid_percent_encoding("")); // Empty string

        // Invalid: incomplete sequence
        assert!(!has_valid_percent_encoding("test%2"));
        assert!(!has_valid_percent_encoding("test%"));
        assert!(!has_valid_percent_encoding("%"));
        assert!(!has_valid_percent_encoding("abc%"));
        assert!(!has_valid_percent_encoding("abc%1"));

        // Invalid: non-hex characters
        assert!(!has_valid_percent_encoding("test%GG"));
        assert!(!has_valid_percent_encoding("test%ZZ"));
        assert!(!has_valid_percent_encoding("%XX"));
    }

    #[test]
    fn test_parse_url_decodes_percent_encoding() {
        // Space in filename
        assert_eq!(
            parse_godot_url("res://my%20file.html", GodotScheme::Res),
            Some("res://my file.html".to_string())
        );

        // Space in directory name
        assert_eq!(
            parse_godot_url("res://my%20folder/index.html", GodotScheme::Res),
            Some("res://my folder/index.html".to_string())
        );

        // Multiple spaces
        assert_eq!(
            parse_godot_url(
                "res://path%20with%20spaces/file%20name.txt",
                GodotScheme::Res
            ),
            Some("res://path with spaces/file name.txt".to_string())
        );

        // Special characters
        assert_eq!(
            parse_godot_url("res://file%5B1%5D.txt", GodotScheme::Res),
            Some("res://file[1].txt".to_string())
        );

        // User scheme with encoding
        assert_eq!(
            parse_godot_url("user://my%20data.json", GodotScheme::User),
            Some("user://my data.json".to_string())
        );

        // Combined with query params (query stripped, path decoded)
        assert_eq!(
            parse_godot_url("res://my%20file.html?v=1", GodotScheme::Res),
            Some("res://my file.html".to_string())
        );
    }

    #[test]
    fn test_rejects_invalid_percent_encoding() {
        // Incomplete encoding
        assert_eq!(parse_godot_url("res://file%2", GodotScheme::Res), None);
        assert_eq!(parse_godot_url("res://file%", GodotScheme::Res), None);

        // Invalid hex characters
        assert_eq!(parse_godot_url("res://file%GG.txt", GodotScheme::Res), None);
    }
}
