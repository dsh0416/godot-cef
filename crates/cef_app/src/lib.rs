mod app;
mod browser_process;
pub mod ipc_contract;
mod loader;
pub mod message_pump;
mod render_handler;
mod render_process;
mod types;
mod v8_handlers;

pub use app::{GodotRenderBackend, GpuDeviceIds, OsrApp, OsrAppBuilder, SecurityConfig};
pub use loader::{load_cef_framework_from_path, load_sandbox_from_path};
pub use render_handler::OsrRenderHandler;
pub use types::{CursorType, FrameBuffer, PhysicalSize, PopupRect, PopupState};

pub const GDCEF_PARENT_PID_SWITCH: &str = "gdcef-parent-pid";

use crate::browser_process::{BrowserProcessHandlerBuilder, OsrBrowserProcessHandler};
use crate::render_process::{OsrRenderProcessHandler, RenderProcessHandlerBuilder};
use cef::{self, App, ImplApp, ImplCommandLine, ImplSchemeRegistrar, WrapApp, rc::Rc, wrap_app};

/// Returns the Chromium features that must be enabled for the current host.
///
/// Chromium's Wayland desktop capturer uses PipeWire through the XDG desktop
/// portal. The feature is intentionally enabled only for a Wayland session;
/// X11 keeps the existing capturer and behavior.
#[allow(unused_mut, unused_variables)]
fn default_enable_features(
    backend: GodotRenderBackend,
    wayland_session: bool,
) -> Vec<&'static str> {
    let mut features = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if backend == GodotRenderBackend::Vulkan {
            features.extend(["Vulkan", "VulkanFromANGLE", "DefaultANGLEVulkan"]);
        }

        if wayland_session {
            features.push("WebRTCPipeWireCapturer");
        }
    }

    features
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value.eq_ignore_ascii_case("wayland"))
}

#[cfg(not(target_os = "linux"))]
fn is_wayland_session() -> bool {
    false
}

fn merge_enable_features(features: &mut Vec<String>, value: &str) {
    for feature in value.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        if !features.iter().any(|existing| existing == feature) {
            features.push(feature.to_owned());
        }
    }
}

wrap_app! {
    pub struct AppBuilder {
        app: OsrApp,
    }

    impl App {
        fn on_register_custom_schemes(&self, registrar: Option<&mut cef::SchemeRegistrar>) {
            let Some(registrar) = registrar else {
                return;
            };

            let options = cef::SchemeOptions::STANDARD.get_raw()
                | cef::SchemeOptions::LOCAL.get_raw()
                | cef::SchemeOptions::SECURE.get_raw()
                | cef::SchemeOptions::CORS_ENABLED.get_raw()
                | cef::SchemeOptions::FETCH_ENABLED.get_raw()
                | cef::SchemeOptions::CSP_BYPASSING.get_raw();

            #[cfg(target_os = "windows")]
            {
                registrar.add_custom_scheme(Some(&"res".into()), options);
                registrar.add_custom_scheme(Some(&"user".into()), options);
            }
            #[cfg(not(target_os = "windows"))]
            {
                registrar.add_custom_scheme(Some(&"res".into()), options as i32);
                registrar.add_custom_scheme(Some(&"user".into()), options as i32);
            }
        }

        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&cef::CefStringUtf16>,
            command_line: Option<&mut cef::CommandLine>,
        ) {
            let Some(command_line) = command_line else {
                return;
            };

            command_line.append_switch(Some(&"no-sandbox".into()));
            command_line.append_switch(Some(&"disable-in-process-stack-traces".into()));
            command_line.append_switch(Some(&"disable-breakpad".into()));
            command_line.append_switch(Some(&"disable-crash-reporter".into()));
            command_line.append_switch(Some(&"no-startup-window".into()));
            command_line.append_switch(Some(&"noerrdialogs".into()));
            command_line.append_switch(Some(&"hide-crash-restore-bubble".into()));
            command_line.append_switch(Some(&"use-mock-keychain".into()));
            command_line.append_switch(Some(&"enable-logging=stderr".into()));
            command_line.append_switch(Some(&"transparent-painting-enabled".into()));
            command_line.append_switch(Some(&"enable-zero-copy".into()));
            command_line.append_switch(Some(&"off-screen-rendering-enabled".into()));
            command_line.append_switch(Some(&"use-views".into()));

            let mut enable_features: Vec<String> = default_enable_features(
                self.app.godot_backend(),
                is_wayland_session(),
            )
            .into_iter()
            .map(str::to_owned)
            .collect();

            // Only enable remote debugging in debug builds or when running from the editor
            // for security purposes. In production builds, this should be disabled.
            if self.app.enable_remote_debugging() {
                let port = self.app.remote_debugging_port().to_string();
                command_line
                    .append_switch_with_value(Some(&"remote-debugging-port".into()), Some(&port.as_str().into()));
            }

            // Apply custom user agent if configured
            let user_agent = self.app.user_agent();
            if !user_agent.is_empty() {
                command_line
                    .append_switch_with_value(Some(&"user-agent".into()), Some(&user_agent.into()));
            }

            // Apply proxy settings if configured
            let proxy_server = self.app.proxy_server();
            if !proxy_server.is_empty() {
                command_line
                    .append_switch_with_value(Some(&"proxy-server".into()), Some(&proxy_server.into()));

                // Apply proxy bypass list if configured
                let proxy_bypass_list = self.app.proxy_bypass_list();
                if !proxy_bypass_list.is_empty() {
                    command_line
                        .append_switch_with_value(Some(&"proxy-bypass-list".into()), Some(&proxy_bypass_list.into()));
                }
            }

            // Apply cache size limit if configured (in bytes)
            let cache_size_mb = self.app.cache_size_mb();
            if cache_size_mb > 0
                && let Some(cache_size_bytes) = (cache_size_mb as i64).checked_mul(1024 * 1024) {
                    let cache_size_bytes = cache_size_bytes.to_string();
                    command_line
                        .append_switch_with_value(Some(&"disk-cache-size".into()), Some(&cache_size_bytes.as_str().into()));
                }

            // Apply custom command-line switches
            for switch in self.app.custom_switches() {
                let trimmed = switch.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Handle switches with and without values
                // Format: "--switch-name" or "--switch-name=value" or "switch-name" or "switch-name=value"
                let switch_str = trimmed.trim_start_matches('-');
                if let Some((name, value)) = switch_str.split_once('=') {
                    if name == "enable-features" {
                        merge_enable_features(&mut enable_features, value);
                        continue;
                    }

                    command_line
                        .append_switch_with_value(Some(&name.into()), Some(&value.into()));
                } else {
                    command_line.append_switch(Some(&switch_str.into()));
                }
            }

            if !enable_features.is_empty() {
                let enable_features = enable_features.join(",");
                command_line.append_switch_with_value(
                    Some(&"enable-features".into()),
                    Some(&enable_features.as_str().into()),
                );
            }
        }

        fn browser_process_handler(&self) -> Option<cef::BrowserProcessHandler> {
            Some(BrowserProcessHandlerBuilder::build(
                OsrBrowserProcessHandler::new(
                    self.app.security_config().clone(),
                    self.app.gpu_device_ids(),
                ),
            ))
        }

        fn render_process_handler(&self) -> Option<cef::RenderProcessHandler> {
            Some(RenderProcessHandlerBuilder::build(
                OsrRenderProcessHandler::new(),
            ))
        }
    }
}

impl AppBuilder {
    pub fn build(app: OsrApp) -> cef::App {
        Self::new(app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_features_are_empty_without_linux_specific_options() {
        assert!(default_enable_features(GodotRenderBackend::Unknown, false).is_empty());
    }

    #[test]
    fn custom_features_are_merged_without_duplicates() {
        let mut features = vec!["WebRTCPipeWireCapturer".to_owned()];

        merge_enable_features(
            &mut features,
            "WebRTC, WebRTCPipeWireCapturer, ,UseOzonePlatform",
        );

        assert_eq!(
            features,
            vec!["WebRTCPipeWireCapturer", "WebRTC", "UseOzonePlatform"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wayland_enables_pipewire_without_dropping_vulkan_features() {
        assert_eq!(
            default_enable_features(GodotRenderBackend::Vulkan, true),
            vec![
                "Vulkan",
                "VulkanFromANGLE",
                "DefaultANGLEVulkan",
                "WebRTCPipeWireCapturer"
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn x11_does_not_enable_pipewire() {
        assert_eq!(
            default_enable_features(GodotRenderBackend::Unknown, false),
            Vec::<&str>::new()
        );
    }
}
