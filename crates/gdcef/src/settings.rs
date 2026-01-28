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

const DEFAULT_DATA_PATH: &str = "user://cef-data";
const DEFAULT_ALLOW_INSECURE_CONTENT: bool = false;
const DEFAULT_IGNORE_CERTIFICATE_ERRORS: bool = false;
const DEFAULT_DISABLE_WEB_SECURITY: bool = false;
const DEFAULT_ENABLE_AUDIO_CAPTURE: bool = false;

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

pub fn warn_if_insecure_settings() {
    let config = get_security_config();

    if config.allow_insecure_content {
        godot::global::godot_warn!(
            "[GodotCef] Security Warning: 'allow_insecure_content' is enabled. \
             This allows loading HTTP content in HTTPS pages."
        );
    }

    if config.ignore_certificate_errors {
        godot::global::godot_warn!(
            "[GodotCef] Security Warning: 'ignore_certificate_errors' is enabled. \
             SSL/TLS certificate validation is disabled."
        );
    }

    if config.disable_web_security {
        godot::global::godot_warn!(
            "[GodotCef] Security Warning: 'disable_web_security' is enabled. \
             CORS and same-origin policy are disabled."
        );
    }
}
