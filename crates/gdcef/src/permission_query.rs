//! Read Chromium's current scalar content setting for an explicit URL pair.
//!
//! This is not the history of Godot permission decisions, nor a prediction of
//! API success: per-browser policy, OS access, Permissions Policy and other
//! Chromium checks remain separate. Queries never create or resolve a prompt.

use cef::{ImplBrowserHost, ImplRequestContext};

use crate::browser::App;

pub(crate) fn query(
    app: &App,
    permission_type: &str,
    requesting_url: &str,
    top_level_url: &str,
) -> &'static str {
    // Do not call even CEF's thread check until this browser exists: the process
    // runtime may not have been initialized (or loaded on macOS) yet.
    if app.state.is_none() {
        return "unavailable";
    }
    let request = match ContentQuery::new(permission_type, requesting_url, top_level_url) {
        Ok(request) => request,
        Err(reason) => return reason,
    };
    if cef::currently_on(cef::ThreadId::UI) == 0 {
        return "unavailable";
    }
    let Some(context) = app.host().and_then(|host| host.request_context()) else {
        return "unavailable";
    };
    read_setting(&context, &request)
}

struct ContentQuery {
    content_type: cef::ContentSettingTypes,
    requesting_url: url::Url,
    top_level_url: url::Url,
}

impl ContentQuery {
    fn new(
        permission_type: &str,
        requesting_url: &str,
        top_level_url: &str,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            content_type: content_type(permission_type).ok_or("unsupported")?,
            requesting_url: explicit_http_url(requesting_url).ok_or("invalid_url")?,
            top_level_url: explicit_http_url(top_level_url).ok_or("invalid_url")?,
        })
    }
}

fn read_setting(context: &cef::RequestContext, request: &ContentQuery) -> &'static str {
    let mut requesting_utf16: Vec<u16> = request.requesting_url.as_str().encode_utf16().collect();
    let mut top_level_utf16: Vec<u16> = request.top_level_url.as_str().encode_utf16().collect();
    // From<cef_string_utf16_t> uses cef-rs's Borrowed representation, unlike
    // From<&str>, which invokes CEF conversion/allocation functions. Both Rust
    // buffers outlive the synchronous getter and neither wrapper owns a dtor.
    // The mock therefore also runs on macOS without loading a CEF framework.
    let requesting_url = cef::CefStringUtf16::from(cef::sys::cef_string_utf16_t {
        str_: requesting_utf16.as_mut_ptr(),
        length: requesting_utf16.len(),
        dtor: None,
    });
    let top_level_url = cef::CefStringUtf16::from(cef::sys::cef_string_utf16_t {
        str_: top_level_utf16.as_mut_ptr(),
        length: top_level_utf16.len(),
        dtor: None,
    });
    setting_name(context.content_setting(
        Some(&requesting_url),
        Some(&top_level_url),
        request.content_type,
    ))
}

fn explicit_http_url(input: &str) -> Option<url::Url> {
    // Url::parse deliberately repairs some browser input (whitespace, missing
    // slashes and empty userinfo). This API requires an explicit site URL;
    // never let malformed input turn into a query of the profile's default.
    if input
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let (_, after_scheme) = input.split_once("://")?;
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return None;
    }
    let url = url::Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url)
}

fn content_type(permission_type: &str) -> Option<cef::ContentSettingTypes> {
    use cef::ContentSettingTypes as T;
    // Only scalar types registered on every supported desktop platform belong
    // here. Chromium CHECKs registry membership; enum presence alone is not
    // evidence that GetContentSetting is safe for that permission/platform.
    Some(match permission_type {
        "ar_session" => T::AR,
        "camera_pan_tilt_zoom" => T::CAMERA_PAN_TILT_ZOOM,
        "camera" => T::MEDIASTREAM_CAMERA,
        "captured_surface_control" => T::CAPTURED_SURFACE_CONTROL,
        "clipboard" => T::CLIPBOARD_READ_WRITE,
        "top_level_storage_access" => T::TOP_LEVEL_STORAGE_ACCESS,
        "local_fonts" => T::LOCAL_FONTS,
        "hand_tracking" => T::HAND_TRACKING,
        "idle_detection" => T::IDLE_DETECTION,
        "microphone" => T::MEDIASTREAM_MIC,
        "midi_sysex" => T::MIDI_SYSEX,
        "notifications" => T::NOTIFICATIONS,
        "keyboard_lock" => T::KEYBOARD_LOCK,
        "pointer_lock" => T::POINTER_LOCK,
        "storage_access" => T::STORAGE_ACCESS,
        "vr_session" => T::VR,
        "web_app_installation" => T::WEB_APP_INSTALLATION,
        "window_management" => T::WINDOW_MANAGEMENT,
        "local_network" => T::LOCAL_NETWORK,
        "loopback_network" => T::LOOPBACK_NETWORK,
        "sensors" => T::SENSORS,
        // In Chromium 154 geolocation may use a dictionary-valued setting;
        // deprecated LAN access has no current permission-request mapping.
        // Chooser permissions and platform-specific registrations are likewise
        // excluded instead of manufacturing a misleading default/ask result.
        _ => return None,
    })
}

fn setting_name(setting: cef::ContentSettingValues) -> &'static str {
    use cef::ContentSettingValues as V;
    match setting {
        V::ALLOW => "allow",
        V::BLOCK => "block",
        V::ASK => "ask",
        V::DEFAULT => "default",
        V::SESSION_ONLY => "session_only",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cef::rc::{ConvertReturnValue, RcImpl};
    use std::sync::{Arc, Mutex};

    fn query_context(
        context: &cef::RequestContext,
        permission_type: &str,
        requesting_url: &str,
        top_level_url: &str,
    ) -> &'static str {
        match ContentQuery::new(permission_type, requesting_url, top_level_url) {
            Ok(request) => read_setting(context, &request),
            Err(reason) => reason,
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ObservedQuery {
        requesting_url: Option<String>,
        top_level_url: Option<String>,
        content_type: cef::ContentSettingTypes,
        uses_borrowed_buffers: bool,
    }

    type Queries = Arc<Mutex<Vec<ObservedQuery>>>;

    struct ContextProbe {
        result: cef::ContentSettingValues,
        queries: Queries,
    }

    fn request_context(result: cef::ContentSettingValues) -> (cef::RequestContext, Queries) {
        unsafe extern "C" fn get_content_setting(
            this: *mut cef::sys::cef_request_context_t,
            requesting_url: *const cef::sys::cef_string_t,
            top_level_url: *const cef::sys::cef_string_t,
            content_type: cef::sys::cef_content_setting_types_t,
        ) -> cef::sys::cef_content_setting_values_t {
            // RcImpl is repr(C) with the CEF object as its first member. Its
            // reference count owns this probe for the whole native call.
            let probe = unsafe {
                &(*this.cast::<RcImpl<cef::sys::cef_request_context_t, ContextProbe>>()).interface
            };
            fn read_url(raw: *const cef::sys::cef_string_t) -> Option<String> {
                // These strings are borrowed solely for this callback. Read
                // them without taking ownership of cef-rs's UTF-16 allocation.
                let raw = unsafe { raw.as_ref() }?;
                if raw.length == 0 {
                    return Some(String::new());
                }
                if raw.str_.is_null() {
                    return None;
                }
                let utf16 = unsafe { std::slice::from_raw_parts(raw.str_, raw.length) };
                Some(String::from_utf16_lossy(utf16))
            }
            probe
                .queries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(ObservedQuery {
                    requesting_url: read_url(requesting_url),
                    top_level_url: read_url(top_level_url),
                    content_type: content_type.into(),
                    // The getter receives live stack-scoped descriptors; an
                    // absent destructor confirms the buffers remain Rust-owned.
                    uses_borrowed_buffers: unsafe {
                        requesting_url
                            .as_ref()
                            .is_some_and(|url| url.dtor.is_none())
                            && top_level_url.as_ref().is_some_and(|url| url.dtor.is_none())
                    },
                });
            probe.result.into()
        }

        let queries = Arc::new(Mutex::new(Vec::new()));
        // CEF structs contain integers and optional function pointers. RcImpl
        // fills the nested base's reference counting callbacks. Only the read
        // API exists in this mock: a setting mutation is never supplied.
        let mut raw: cef::sys::cef_request_context_t = unsafe { std::mem::zeroed() };
        raw.get_content_setting = Some(get_content_setting);
        let pointer = RcImpl::new(
            raw,
            ContextProbe {
                result,
                queries: queries.clone(),
            },
        );
        (
            pointer
                .cast::<cef::sys::cef_request_context_t>()
                .wrap_result(),
            queries,
        )
    }

    #[test]
    fn unavailable_before_browser_creation_does_not_call_into_cef() {
        assert_eq!(
            query(
                &App::default(),
                "camera",
                "https://requester.test",
                "https://top.test"
            ),
            "unavailable"
        );
    }

    #[test]
    fn query_passes_both_explicit_urls_and_the_exact_scalar_type() {
        let (context, queries) = request_context(cef::ContentSettingValues::ASK);
        assert_eq!(
            query_context(
                &context,
                "local_network",
                "https://Requester.test:8443/path?q=1",
                "https://top.test/page"
            ),
            "ask"
        );
        assert_eq!(
            *queries.lock().unwrap_or_else(|error| error.into_inner()),
            [ObservedQuery {
                requesting_url: Some("https://requester.test:8443/path?q=1".to_owned()),
                top_level_url: Some("https://top.test/page".to_owned()),
                content_type: cef::ContentSettingTypes::LOCAL_NETWORK,
                uses_borrowed_buffers: true,
            }]
        );
    }

    #[test]
    fn all_cef_scalar_results_remain_distinct() {
        use cef::ContentSettingValues as V;
        for (raw, expected) in [
            (V::ALLOW, "allow"),
            (V::BLOCK, "block"),
            (V::ASK, "ask"),
            (V::DEFAULT, "default"),
            (V::SESSION_ONLY, "session_only"),
            (V::DETECT_IMPORTANT_CONTENT_DEPRECATED, "unknown"),
            (V::NUM_VALUES, "unknown"),
        ] {
            let (context, queries) = request_context(raw);
            assert_eq!(
                query_context(&context, "camera", "https://a.test", "https://b.test"),
                expected
            );
            assert_eq!(
                queries
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .len(),
                1
            );
        }
    }

    #[test]
    fn invalid_url_pairs_never_call_the_content_setting_api() {
        let (context, queries) = request_context(cef::ContentSettingValues::ALLOW);
        for invalid in [
            "",
            " ",
            "https://",
            "https:///example.test",
            "https:example.test",
            "//example.test",
            "example.test",
            "file:///tmp/page",
            "godot://index.html",
            "about:blank",
            "data:text/html,hello",
            "https://user@example.test",
            "https://user:secret@example.test",
            "https://@example.test",
            "https://:secret@example.test",
            " https://example.test",
            "https://exa\nmple.test",
            "https://example.test\0",
            "https://example.test\\other",
            "https://example.test:invalid",
        ] {
            assert_eq!(
                query_context(&context, "camera", invalid, "https://top.test"),
                "invalid_url",
                "requester: {invalid:?}"
            );
            assert_eq!(
                query_context(&context, "camera", "https://requester.test", invalid),
                "invalid_url",
                "top level: {invalid:?}"
            );
        }
        assert!(
            queries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );
    }

    #[test]
    fn http_localhost_ip_addresses_and_encoded_paths_are_valid() {
        for input in [
            "http://localhost:3000/",
            "http://192.168.0.10:8080/path",
            "https://[::1]:8443/",
            "HTTPS://EXAMPLE.TEST/page",
            "https://example.test/a%20b?q=value#fragment",
        ] {
            assert!(explicit_http_url(input).is_some(), "{input}");
        }
    }

    #[test]
    fn unsupported_and_unsafe_content_types_never_call_cef() {
        let (context, queries) = request_context(cef::ContentSettingValues::ALLOW);
        for permission in [
            "",
            "unknown_permission",
            "unknown_media_permission",
            "geolocation",
            "local_network_access",
            "protected_media_identifier",
            "disk_quota",
            "file_system_access",
            "identity_provider",
            "multiple_downloads",
            "register_protocol_handler",
            "desktop_audio_capture",
            "desktop_video_capture",
            "cookies",
            "javascript",
            "permission_autoblocker_data",
        ] {
            assert_eq!(
                query_context(&context, permission, "https://a.test", "https://b.test"),
                "unsupported",
                "{permission}"
            );
        }
        assert!(
            queries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );
    }

    #[test]
    fn principal_permissions_map_to_their_actual_chromium_setting() {
        use cef::ContentSettingTypes as T;
        for (permission, expected) in [
            ("camera", T::MEDIASTREAM_CAMERA),
            ("microphone", T::MEDIASTREAM_MIC),
            ("local_network", T::LOCAL_NETWORK),
            ("loopback_network", T::LOOPBACK_NETWORK),
            ("clipboard", T::CLIPBOARD_READ_WRITE),
            ("notifications", T::NOTIFICATIONS),
            ("midi_sysex", T::MIDI_SYSEX),
            ("sensors", T::SENSORS),
            ("idle_detection", T::IDLE_DETECTION),
        ] {
            let (context, queries) = request_context(cef::ContentSettingValues::ASK);
            assert_eq!(
                query_context(&context, permission, "https://a.test", "https://b.test"),
                "ask"
            );
            assert_eq!(
                queries.lock().unwrap_or_else(|error| error.into_inner())[0].content_type,
                expected
            );
        }
    }
}
