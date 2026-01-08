//! Custom `res://` scheme handler for CEF.
//!
//! This module implements a custom scheme handler that allows CEF to load
//! resources from Godot's packed resource system using the `res://` protocol.
//! This enables exported Godot projects to serve local web content (HTML, CSS,
//! JS, images, etc.) directly to the embedded browser without requiring an
//! external web server.

use cef::{
    rc::Rc, wrap_resource_handler, wrap_scheme_handler_factory, CefStringUtf16, ImplRequest,
    ImplResourceHandler, ImplResponse, ImplSchemeHandlerFactory, ResourceHandler,
    SchemeHandlerFactory, WrapResourceHandler, WrapSchemeHandlerFactory,
};
use godot::classes::file_access::ModeFlags;
use godot::classes::FileAccess;
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

/// Get the MIME type for a file based on its extension.
fn get_mime_type(extension: &str) -> &'static str {
    MIME_TYPES
        .get(extension.to_lowercase().as_str())
        .unwrap_or(&"application/octet-stream")
}

/// Parse a `res://` URL and return the full Godot resource path.
///
/// URL format: `res://path/to/file.html` or `res://folder/`
/// Returns: `res://path/to/file.html` or `res://folder/index.html` for directories
fn parse_res_url(url: &str) -> String {
    // Remove the scheme prefix if present
    let path = url
        .strip_prefix("res://")
        .or_else(|| url.strip_prefix("res:"))
        .unwrap_or(url);

    // Build the full res:// path
    let mut full_path = format!("res://{}", path);

    // Check if path ends with / or has no extension (likely a directory)
    // In that case, append index.html
    if full_path.ends_with('/') || !full_path.contains('.') || full_path.ends_with("res://") {
        if !full_path.ends_with('/') {
            full_path.push('/');
        }
        full_path.push_str("index.html");
    }

    full_path
}

/// State for tracking the resource being served.
#[derive(Clone, Default)]
struct ResourceState {
    /// The file data loaded from Godot's FileAccess.
    data: Vec<u8>,
    /// Current read position in the data.
    offset: usize,
    /// HTTP status code for the response.
    status_code: i32,
    /// MIME type of the resource.
    mime_type: String,
    /// Error message if the resource could not be loaded.
    error_message: Option<String>,
}

/// Resource handler for serving files from Godot's res:// filesystem.
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

            // Get the URL from the request
            let url_cef = request.url();
            let url = CefStringUtf16::from(&url_cef).to_string();

            // Parse the res:// URL to get the full path
            let res_path = parse_res_url(&url);
            let gstring_path = GString::from(&res_path);

            let mut state = self.handler.state.borrow_mut();

            // Check if the file exists
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

            // Open the file using Godot's FileAccess
            match FileAccess::open(&gstring_path, ModeFlags::READ) {
                Some(file) => {
                    let file_length = file.get_length() as usize;

                    // Get file extension for MIME type
                    let path = PathBuf::from(&res_path);
                    let extension = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    state.mime_type = get_mime_type(extension).to_string();

                    // Read the entire file into memory
                    let buffer = file.get_buffer(file_length as i64);
                    state.data = buffer.as_slice().to_vec();
                    state.status_code = 200;
                    state.offset = 0;
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

            // Signal that we handled the request synchronously
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
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Unknown",
                };
                response.set_status_text(Some(&status_text.into()));

                response.set_mime_type(Some(&state.mime_type.as_str().into()));

                // Set Content-Type header
                let content_type_key: CefStringUtf16 = "Content-Type".into();
                let content_type_value: CefStringUtf16 = state.mime_type.as_str().into();
                response.set_header_by_name(Some(&content_type_key), Some(&content_type_value), true as _);

                // Set Access-Control-Allow-Origin for CORS
                let cors_key: CefStringUtf16 = "Access-Control-Allow-Origin".into();
                let cors_value: CefStringUtf16 = "*".into();
                response.set_header_by_name(Some(&cors_key), Some(&cors_value), true as _);
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

        fn cancel(&self) {
            // Nothing to cancel for synchronous file reading
        }
    }
}

impl ResResourceHandlerImpl {
    pub fn build(handler: ResResourceHandler) -> ResourceHandler {
        Self::new(handler)
    }
}

/// Factory for creating ResResourceHandler instances.
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

/// Register the `res://` scheme handler with CEF globally.
///
/// NOTE: This only works for browsers using the global request context.
/// For browsers with custom request contexts, use `register_res_scheme_handler_on_context`.
#[allow(dead_code)]
pub fn register_res_scheme_handler() {
    let mut factory = ResSchemeHandlerFactory::build(ResSchemeHandler::new());
    cef::register_scheme_handler_factory(
        Some(&"res".into()),
        Some(&"".into()),
        Some(&mut factory),
    );
}

/// Register the `res://` scheme handler on a specific request context.
///
/// This is needed when using a custom RequestContext for the browser.
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

