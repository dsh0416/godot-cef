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
use std::cell::RefCell;
use std::path::PathBuf;

use super::mime::get_mime_type;
use super::multipart::{read_multipart_streaming, MultipartStreamState, MULTIPART_BOUNDARY};
use super::range::{parse_range_header, ParsedRanges};
use super::GodotScheme;

/// Parse a URL into a Godot filesystem path.
pub(crate) fn parse_godot_url(url: &str, scheme: GodotScheme) -> String {
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
    multipart_stream: Option<MultipartStreamState>,
    file_path: Option<String>,
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
                // Clone values needed for streaming before mutable borrow
                let file_path = state.file_path.clone().unwrap();
                let mime_type = state.mime_type.clone();
                let file_size = state.total_file_size;
                let stream = state.multipart_stream.as_mut().unwrap();

                let written = read_multipart_streaming(
                    stream,
                    &file_path,
                    &mime_type,
                    file_size,
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
}

