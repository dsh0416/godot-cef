use cef_app::SecurityConfig;
use godot::classes::ProjectSettings;
use godot::global::PropertyHint;
use godot::prelude::*;
use std::path::PathBuf;

const SETTING_DATA_PATH: &str = "godot_cef/storage/data_path";
const SETTING_ALLOW_INSECURE_CONTENT: &str = "godot_cef/security/allow_insecure_content";
const SETTING_IGNORE_CERTIFICATE_ERRORS: &str = "godot_cef/security/ignore_certificate_errors";
const SETTING_DISABLE_WEB_SECURITY: &str = "godot_cef/security/disable_web_security";
const SETTING_ENABLE_AUDIO_CAPTURE: &str = "godot_cef/audio/enable_audio_capture";
const SETTING_REMOTE_DEVTOOLS_PORT: &str = "godot_cef/debug/remote_devtools_port";
const SETTING_MAX_FRAME_RATE: &str = "godot_cef/performance/max_frame_rate";
const SETTING_CACHE_SIZE_MB: &str = "godot_cef/storage/cache_size_mb";
const SETTING_USER_AGENT: &str = "godot_cef/network/user_agent";
const SETTING_PROXY_SERVER: &str = "godot_cef/network/proxy_server";
const SETTING_PROXY_BYPASS_LIST: &str = "godot_cef/network/proxy_bypass_list";
const SETTING_CUSTOM_SWITCHES: &str = "godot_cef/advanced/custom_command_line_switches";

const DEFAULT_DATA_PATH: &str = "user://cef-data";
const DEFAULT_ALLOW_INSECURE_CONTENT: bool = false;
const DEFAULT_IGNORE_CERTIFICATE_ERRORS: bool = false;
const DEFAULT_DISABLE_WEB_SECURITY: bool = false;
const DEFAULT_ENABLE_AUDIO_CAPTURE: bool = false;
const DEFAULT_REMOTE_DEVTOOLS_PORT: i64 = 9229;
const DEFAULT_MAX_FRAME_RATE: i64 = 0; // 0 = follow Godot engine FPS
const DEFAULT_CACHE_SIZE_MB: i64 = 0; // 0 = use CEF default
const DEFAULT_USER_AGENT: &str = ""; // Empty = use CEF default
const DEFAULT_PROXY_SERVER: &str = ""; // Empty = direct connection
const DEFAULT_PROXY_BYPASS_LIST: &str = ""; // Empty = no bypass
const DEFAULT_CUSTOM_SWITCHES: &str = ""; // Empty = no custom switches

pub fn register_project_settings() {
    let mut settings = ProjectSettings::singleton();

    register_string_setting(
        &mut settings,
        SETTING_DATA_PATH,
        DEFAULT_DATA_PATH,
        PropertyHint::DIR,
        "",
    );

    register_bool_setting(
        &mut settings,
        SETTING_ALLOW_INSECURE_CONTENT,
        DEFAULT_ALLOW_INSECURE_CONTENT,
    );

    register_bool_setting(
        &mut settings,
        SETTING_IGNORE_CERTIFICATE_ERRORS,
        DEFAULT_IGNORE_CERTIFICATE_ERRORS,
    );

    register_bool_setting(
        &mut settings,
        SETTING_DISABLE_WEB_SECURITY,
        DEFAULT_DISABLE_WEB_SECURITY,
    );

    register_bool_setting(
        &mut settings,
        SETTING_ENABLE_AUDIO_CAPTURE,
        DEFAULT_ENABLE_AUDIO_CAPTURE,
    );

    register_int_setting(
        &mut settings,
        SETTING_REMOTE_DEVTOOLS_PORT,
        DEFAULT_REMOTE_DEVTOOLS_PORT,
        PropertyHint::RANGE,
        "1,65535",
    );

    // Performance settings
    register_int_setting(
        &mut settings,
        SETTING_MAX_FRAME_RATE,
        DEFAULT_MAX_FRAME_RATE,
        PropertyHint::RANGE,
        "0,240,or_greater",
    );

    // Storage settings
    register_int_setting(
        &mut settings,
        SETTING_CACHE_SIZE_MB,
        DEFAULT_CACHE_SIZE_MB,
        PropertyHint::RANGE,
        "0,10240,or_greater",
    );

    // Network settings
    register_string_setting(
        &mut settings,
        SETTING_USER_AGENT,
        DEFAULT_USER_AGENT,
        PropertyHint::PLACEHOLDER_TEXT,
        "Custom user agent string (empty = CEF default)",
    );

    register_string_setting(
        &mut settings,
        SETTING_PROXY_SERVER,
        DEFAULT_PROXY_SERVER,
        PropertyHint::PLACEHOLDER_TEXT,
        "e.g., socks5://127.0.0.1:1080 or http://proxy:8080",
    );

    register_string_setting(
        &mut settings,
        SETTING_PROXY_BYPASS_LIST,
        DEFAULT_PROXY_BYPASS_LIST,
        PropertyHint::PLACEHOLDER_TEXT,
        "Comma-separated list, e.g., localhost,127.0.0.1",
    );

    // Advanced settings
    register_string_setting(
        &mut settings,
        SETTING_CUSTOM_SWITCHES,
        DEFAULT_CUSTOM_SWITCHES,
        PropertyHint::MULTILINE_TEXT,
        "",
    );
}

fn register_string_setting(
    settings: &mut Gd<ProjectSettings>,
    name: &str,
    default: &str,
    hint: PropertyHint,
    hint_string: &str,
) {
    let name_gstring: GString = name.into();

    if !settings.has_setting(&name_gstring) {
        settings.set_setting(&name_gstring, &default.to_variant());
    }

    settings.set_initial_value(&name_gstring, &default.to_variant());
    settings.set_as_basic(&name_gstring, true);

    let property_info = vdict! {
        "name": name_gstring.clone(),
        "type": VariantType::STRING.ord(),
        "hint": hint.ord(),
        "hint_string": hint_string,
    };

    settings.add_property_info(&property_info);
}

fn register_bool_setting(settings: &mut Gd<ProjectSettings>, name: &str, default: bool) {
    let name_gstring: GString = name.into();

    if !settings.has_setting(&name_gstring) {
        settings.set_setting(&name_gstring, &default.to_variant());
    }

    settings.set_initial_value(&name_gstring, &default.to_variant());
    settings.set_as_basic(&name_gstring, true);

    let property_info = vdict! {
        "name": name_gstring.clone(),
        "type": VariantType::BOOL.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };

    settings.add_property_info(&property_info);
}

fn register_int_setting(
    settings: &mut Gd<ProjectSettings>,
    name: &str,
    default: i64,
    hint: PropertyHint,
    hint_string: &str,
) {
    let name_gstring: GString = name.into();

    if !settings.has_setting(&name_gstring) {
        settings.set_setting(&name_gstring, &default.to_variant());
    }

    settings.set_initial_value(&name_gstring, &default.to_variant());
    settings.set_as_basic(&name_gstring, true);

    let property_info = vdict! {
        "name": name_gstring.clone(),
        "type": VariantType::INT.ord(),
        "hint": hint.ord(),
        "hint_string": hint_string,
    };

    settings.add_property_info(&property_info);
}

pub fn get_data_path() -> PathBuf {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_DATA_PATH.into();

    let path_variant = settings.get_setting(&name_gstring);
    let path_gstring: GString = if path_variant.is_nil() {
        DEFAULT_DATA_PATH.into()
    } else {
        path_variant.to::<GString>()
    };

    let absolute_path = settings.globalize_path(&path_gstring).to_string();

    PathBuf::from(absolute_path)
}

pub fn get_security_config() -> SecurityConfig {
    let settings = ProjectSettings::singleton();

    SecurityConfig {
        allow_insecure_content: get_bool_setting(&settings, SETTING_ALLOW_INSECURE_CONTENT),
        ignore_certificate_errors: get_bool_setting(&settings, SETTING_IGNORE_CERTIFICATE_ERRORS),
        disable_web_security: get_bool_setting(&settings, SETTING_DISABLE_WEB_SECURITY),
    }
}

fn get_bool_setting(settings: &Gd<ProjectSettings>, name: &str) -> bool {
    let name_gstring: GString = name.into();
    let variant = settings.get_setting(&name_gstring);

    if variant.is_nil() {
        match name {
            SETTING_ALLOW_INSECURE_CONTENT => DEFAULT_ALLOW_INSECURE_CONTENT,
            SETTING_IGNORE_CERTIFICATE_ERRORS => DEFAULT_IGNORE_CERTIFICATE_ERRORS,
            SETTING_DISABLE_WEB_SECURITY => DEFAULT_DISABLE_WEB_SECURITY,
            SETTING_ENABLE_AUDIO_CAPTURE => DEFAULT_ENABLE_AUDIO_CAPTURE,
            _ => false,
        }
    } else {
        variant.to::<bool>()
    }
}

pub fn is_audio_capture_enabled() -> bool {
    let settings = ProjectSettings::singleton();
    get_bool_setting(&settings, SETTING_ENABLE_AUDIO_CAPTURE)
}

pub fn get_remote_devtools_port() -> u16 {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_REMOTE_DEVTOOLS_PORT.into();
    let variant = settings.get_setting(&name_gstring);

    let port = if variant.is_nil() {
        DEFAULT_REMOTE_DEVTOOLS_PORT
    } else {
        variant.to::<i64>()
    };

    // Clamp to valid port range
    port.clamp(1, 65535) as u16
}

/// Returns the max frame rate setting. Returns 0 if using Godot engine's FPS.
pub fn get_max_frame_rate() -> i32 {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_MAX_FRAME_RATE.into();
    let variant = settings.get_setting(&name_gstring);

    let fps = if variant.is_nil() {
        DEFAULT_MAX_FRAME_RATE
    } else {
        variant.to::<i64>()
    };

    fps.max(0) as i32
}

/// Returns the cache size limit in megabytes. Returns 0 for CEF default.
pub fn get_cache_size_mb() -> i32 {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_CACHE_SIZE_MB.into();
    let variant = settings.get_setting(&name_gstring);

    let size = if variant.is_nil() {
        DEFAULT_CACHE_SIZE_MB
    } else {
        variant.to::<i64>()
    };

    size.max(0) as i32
}

/// Returns the custom user agent string. Empty string means use CEF default.
pub fn get_user_agent() -> String {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_USER_AGENT.into();
    let variant = settings.get_setting(&name_gstring);

    if variant.is_nil() {
        DEFAULT_USER_AGENT.to_string()
    } else {
        variant.to::<GString>().to_string()
    }
}

/// Returns the proxy server URL. Empty string means direct connection.
pub fn get_proxy_server() -> String {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_PROXY_SERVER.into();
    let variant = settings.get_setting(&name_gstring);

    if variant.is_nil() {
        DEFAULT_PROXY_SERVER.to_string()
    } else {
        variant.to::<GString>().to_string()
    }
}

/// Returns the proxy bypass list. Empty string means no bypass.
pub fn get_proxy_bypass_list() -> String {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_PROXY_BYPASS_LIST.into();
    let variant = settings.get_setting(&name_gstring);

    if variant.is_nil() {
        DEFAULT_PROXY_BYPASS_LIST.to_string()
    } else {
        variant.to::<GString>().to_string()
    }
}

/// Returns custom command-line switches as a list of strings.
/// Each line in the multiline string is treated as a separate switch.
pub fn get_custom_switches() -> Vec<String> {
    let settings = ProjectSettings::singleton();
    let name_gstring: GString = SETTING_CUSTOM_SWITCHES.into();
    let variant = settings.get_setting(&name_gstring);

    let raw = if variant.is_nil() {
        DEFAULT_CUSTOM_SWITCHES.to_string()
    } else {
        variant.to::<GString>().to_string()
    };

    raw.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

pub fn warn_if_insecure_settings() {
    let config = get_security_config();

    if config.allow_insecure_content {
        godot::global::godot_warn!(
            "[GodotCef] Security Warning: '{}' is enabled. This allows HTTP content on HTTPS pages. \
             Disable it for production builds unless strictly required.",
            SETTING_ALLOW_INSECURE_CONTENT
        );
    }

    if config.ignore_certificate_errors {
        godot::global::godot_warn!(
            "[GodotCef] Security Warning: '{}' is enabled. SSL/TLS certificate validation is disabled. \
             Disable it for production builds.",
            SETTING_IGNORE_CERTIFICATE_ERRORS
        );
    }

    if config.disable_web_security {
        godot::global::godot_warn!(
            "[GodotCef] Security Warning: '{}' is enabled. CORS and same-origin policy are disabled. \
             Disable it for production builds.",
            SETTING_DISABLE_WEB_SECURITY
        );
    }

    for switch in get_custom_switches() {
        let normalized = switch.trim().trim_start_matches('-');
        if normalized.starts_with("disable-web-security")
            || normalized.starts_with("ignore-certificate-errors")
            || normalized.starts_with("allow-running-insecure-content")
        {
            godot::global::godot_warn!(
                "[GodotCef] Security Warning: '{}' contains custom switch '{}', which weakens browser security.",
                SETTING_CUSTOM_SWITCHES,
                switch
            );
        }
    }
}

pub fn log_production_security_baseline() {
    godot::global::godot_print!(
        "[GodotCef] Production security baseline: \
         {}=false, {}=false, {}=false, {} should not include insecure switches.",
        SETTING_ALLOW_INSECURE_CONTENT,
        SETTING_IGNORE_CERTIFICATE_ERRORS,
        SETTING_DISABLE_WEB_SECURITY,
        SETTING_CUSTOM_SWITCHES
    );
}
