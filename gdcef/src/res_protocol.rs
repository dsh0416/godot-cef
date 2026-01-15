//! Custom `res://` scheme handler for CEF.
//!
//! This module implements a custom scheme handler that allows CEF to load
//! resources from Godot's packed resource system using the `res://` protocol.
//! This enables exported Godot projects to serve local web content (HTML, CSS,
//! JS, images, etc.) directly to the embedded browser without requiring an
//! external web server.

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

fn parse_res_url(url: &str) -> String {
    let path = url
        .strip_prefix("res://")
        .or_else(|| url.strip_prefix("res:"))
        .unwrap_or(url);

    let mut full_path = format!("res://{}", path);

    // Determine whether the last path component (ignoring trailing '/')
    // has an extension (i.e., contains a dot). This avoids treating dots
    // in parent directory names as file extensions.
    let trimmed = full_path.trim_end_matches('/');
    let last_segment = trimmed.rsplit('/').next().unwrap_or("");
    let has_extension = last_segment.contains('.');

    if full_path.ends_with('/') || !has_extension || full_path.ends_with("res://") {
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
    error_message: Option<String>,
    total_file_size: u64,
    range_start: Option<u64>,
    range_end: Option<u64>,
}

#[derive(Clone)]
pub struct ResResourceHandler {
    state: RefCell<ResourceState>,
}

impl Default for ResResourceHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ResResourceHandler {
    pub fn new() -> Self {
        Self {
            state: RefCell::new(ResourceState::default()),
        }
    }
}

wrap_resource_handler! {
    pub struct ResResourceHandlerImpl {
        handler: ResResourceHandler,
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
            let res_path = parse_res_url(&url);
            let gstring_path = GString::from(&res_path);

            let mut state = self.handler.state.borrow_mut();

            if !FileAccess::file_exists(&gstring_path) {
                state.status_code = 404;
                state.mime_type = "text/plain".to_string();
                state.error_message = Some(format!("File not found: {}", res_path));
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

                    // Parse `Range` header. Supports "bytes=start-end", "bytes=start-",
                    // and "bytes=-suffix_length". Multi-range requests (with commas)
                    // are not supported and are treated as if no Range header was set.
                    let content_range: Option<(u64, Option<u64>)> =
                        if !range_str.is_empty() && range_str.starts_with("bytes=") {
                            let range_part = &range_str[6..];

                            // Reject multi-range specifications like "bytes=0-100,200-300".
                            if range_part.contains(',') {
                                None
                            } else {
                                let parts: Vec<&str> = range_part.split('-').collect();
                                if parts.len() != 2 {
                                    None
                                } else {
                                    let start_str = parts[0].trim();
                                    let end_str = parts[1].trim();

                                    if !start_str.is_empty() {
                                        // "bytes=start-" or "bytes=start-end"
                                        match start_str.parse::<u64>() {
                                            Ok(start) => {
                                                let end_opt = if end_str.is_empty() {
                                                    None
                                                } else {
                                                    end_str.parse::<u64>().ok()
                                                };
                                                end_opt.map(|e| (start, Some(e))).or(Some((start, None)))
                                            }
                                            Err(_) => None,
                                        }
                                    } else if !end_str.is_empty() {
                                        // "bytes=-suffix_length"
                                        match end_str.parse::<u64>() {
                                            Ok(suffix_len) if suffix_len > 0 => {
                                                if suffix_len >= file_size {
                                                    Some((0, None))
                                                } else {
                                                    Some((file_size - suffix_len, None))
                                                }
                                            }
                                            _ => None,
                                        }
                                    } else {
                                        None
                                    }
                                }
                            }
                        } else {
                            None
                        };
                    let path = PathBuf::from(&res_path);
                    let extension = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    state.mime_type = get_mime_type(extension).to_string();

                    if let Some((start, end_opt)) = content_range {
                        if start >= file_size {
                            state.status_code = 416;
                            state.data = Vec::new();
                            state.range_start = None;
                            state.range_end = None;
                        } else {
                            let end = match end_opt {
                                Some(e) if e < file_size => e,
                                _ => file_size - 1,
                            };

                            let content_size = (end - start + 1) as i64;
                            file.seek(start);
                            let buffer = file.get_buffer(content_size);
                            state.data = buffer.as_slice().to_vec();
                            state.status_code = 206;
                            state.range_start = Some(start);
                            state.range_end = Some(end);
                            state.offset = 0;
                        }
                    } else {
                        let buffer = file.get_buffer(file_size as i64);
                        state.data = buffer.as_slice().to_vec();
                        state.status_code = 200;
                        state.range_start = None;
                        state.range_end = None;
                        state.offset = 0;
                    }
                }
                None => {
                    state.status_code = 500;
                    state.mime_type = "text/plain".to_string();
                    state.error_message = Some(format!("Failed to open file: {}", res_path));
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

                response.set_mime_type(Some(&state.mime_type.as_str().into()));

                response.set_header_by_name(Some(&"Content-Type".into()), Some(&state.mime_type.as_str().into()), true as _);
                response.set_header_by_name(Some(&"Access-Control-Allow-Origin".into()), Some(&"*".into()), true as _);
                response.set_header_by_name(Some(&"Accept-Ranges".into()), Some(&"bytes".into()), true as _);

                if state.status_code == 206 {
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

impl ResResourceHandlerImpl {
    pub fn build(handler: ResResourceHandler) -> ResourceHandler {
        Self::new(handler)
    }
}

#[derive(Clone)]
pub struct ResSchemeHandler {}

impl Default for ResSchemeHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ResSchemeHandler {
    pub fn new() -> Self {
        Self {}
    }
}

wrap_scheme_handler_factory! {
    pub struct ResSchemeHandlerFactory {
        handler: ResSchemeHandler,
    }

    impl SchemeHandlerFactory {
        fn create(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _scheme_name: Option<&cef::CefString>,
            _request: Option<&mut cef::Request>,
        ) -> Option<ResourceHandler> {
            Some(ResResourceHandlerImpl::build(ResResourceHandler::new()))
        }
    }
}

impl ResSchemeHandlerFactory {
    pub fn build(handler: ResSchemeHandler) -> SchemeHandlerFactory {
        Self::new(handler)
    }
}

pub fn register_res_scheme_handler_on_context(context: &mut cef::RequestContext) {
    use cef::ImplRequestContext;
    let mut factory = ResSchemeHandlerFactory::build(ResSchemeHandler::new());
    context.register_scheme_handler_factory(
        Some(&"res".into()),
        Some(&"".into()),
        Some(&mut factory),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_res_url() {
        assert_eq!(parse_res_url("res://ui/index.html"), "res://ui/index.html");
        assert_eq!(parse_res_url("res://folder/"), "res://folder/index.html");
        assert_eq!(parse_res_url("res://folder"), "res://folder/index.html");
        assert_eq!(parse_res_url("ui/style.css"), "res://ui/style.css");
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
}
