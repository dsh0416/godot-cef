mod app;
mod browser_process;
mod loader;
mod render_handler;
mod render_process;
mod types;
mod v8_handlers;

pub use app::{GodotRenderBackend, GpuDeviceIds, OsrApp, OsrAppBuilder, SecurityConfig};
pub use loader::{load_cef_framework_from_path, load_sandbox_from_path};
pub use render_handler::OsrRenderHandler;
pub use types::{CursorType, FrameBuffer, PhysicalSize, PopupRect, PopupState};

use crate::browser_process::{BrowserProcessHandlerBuilder, OsrBrowserProcessHandler};
use crate::render_process::{OsrRenderProcessHandler, RenderProcessHandlerBuilder};
use cef::{self, App, ImplApp, ImplCommandLine, ImplSchemeRegistrar, WrapApp, rc::Rc, wrap_app};

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
            command_line.append_switch(Some(&"no-startup-window".into()));
            command_line.append_switch(Some(&"noerrdialogs".into()));
            command_line.append_switch(Some(&"hide-crash-restore-bubble".into()));
            command_line.append_switch(Some(&"use-mock-keychain".into()));
            command_line.append_switch(Some(&"enable-logging=stderr".into()));
            command_line.append_switch(Some(&"transparent-painting-enabled".into()));
            command_line.append_switch(Some(&"enable-zero-copy".into()));
            command_line.append_switch(Some(&"off-screen-rendering-enabled".into()));
            command_line.append_switch(Some(&"use-views".into()));

            // Only enable remote debugging in debug builds or when running from the editor
            // for security purposes. In production builds, this should be disabled.
            if self.app.enable_remote_debugging() {
                command_line
                    .append_switch_with_value(Some(&"remote-debugging-port".into()), Some(&"9229".into()));
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
