use std::cell::RefCell;

use cef::{
    BrowserProcessHandler, CommandLine, ImplBrowserProcessHandler, ImplCommandLine,
    WrapBrowserProcessHandler, rc::Rc, wrap_browser_process_handler,
};

use crate::app::{GpuDeviceIds, SecurityConfig};

#[derive(Clone)]
pub struct OsrBrowserProcessHandler {
    is_cef_ready: RefCell<bool>,
    security_config: SecurityConfig,
    gpu_device_ids: Option<GpuDeviceIds>,
}

impl Default for OsrBrowserProcessHandler {
    fn default() -> Self {
        Self::new(SecurityConfig::default(), None)
    }
}

impl OsrBrowserProcessHandler {
    pub fn new(security_config: SecurityConfig, gpu_device_ids: Option<GpuDeviceIds>) -> Self {
        Self {
            is_cef_ready: RefCell::new(false),
            security_config,
            gpu_device_ids,
        }
    }
}

wrap_browser_process_handler! {
    pub(crate) struct BrowserProcessHandlerBuilder {
        handler: OsrBrowserProcessHandler,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            *self.handler.is_cef_ready.borrow_mut() = true;
        }

        fn on_before_child_process_launch(&self, command_line: Option<&mut CommandLine>) {
            let Some(command_line) = command_line else {
                return;
            };

            let security_config = &self.handler.security_config;
            if security_config.disable_web_security {
                command_line.append_switch(Some(&"disable-web-security".into()));
            }
            if security_config.allow_insecure_content {
                command_line.append_switch(Some(&"allow-running-insecure-content".into()));
            }
            if security_config.ignore_certificate_errors {
                command_line.append_switch(Some(&"ignore-certificate-errors".into()));
                command_line.append_switch(Some(&"ignore-ssl-errors".into()));
            }

            command_line.append_switch(Some(&"disable-session-crashed-bubble".into()));
            command_line.append_switch(Some(&"enable-logging=stderr".into()));

            if let Some(ids) = &self.handler.gpu_device_ids {
                command_line.append_switch_with_value(
                    Some(&"gpu-vendor-id".into()),
                    Some(&ids.to_vendor_arg().as_str().into()),
                );
                command_line.append_switch_with_value(
                    Some(&"gpu-device-id".into()),
                    Some(&ids.to_device_arg().as_str().into()),
                );
            }
        }
    }
}

impl BrowserProcessHandlerBuilder {
    pub(crate) fn build(handler: OsrBrowserProcessHandler) -> BrowserProcessHandler {
        Self::new(handler)
    }
}
