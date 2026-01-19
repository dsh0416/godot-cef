mod loader;

pub use loader::{load_cef_framework_from_path, load_sandbox_from_path};

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use cef::sys::cef_v8_propertyattribute_t;
use cef::{
    self, BrowserProcessHandler, ImplBrowserProcessHandler, WrapBrowserProcessHandler, rc::Rc, *,
};

#[derive(Default)]
pub struct FrameBuffer {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub dirty: bool,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the buffer with new RGBA pixel data
    pub fn update(&mut self, data: Vec<u8>, width: u32, height: u32) {
        self.data = data;
        self.width = width;
        self.height = height;
        self.dirty = true;
    }

    /// Mark the buffer as consumed (not dirty)
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GodotRenderBackend {
    #[default]
    Unknown,
    Direct3D12,
    Metal,
    Vulkan,
}

#[derive(Clone, Default)]
pub struct SecurityConfig {
    /// Allow loading insecure (HTTP) content in HTTPS pages.
    pub allow_insecure_content: bool,
    /// Ignore SSL/TLS certificate errors.
    pub ignore_certificate_errors: bool,
    /// Disable web security (CORS, same-origin policy).
    pub disable_web_security: bool,
}

/// Adapter LUID (Locally Unique Identifier) for GPU selection on Windows.
#[derive(Clone, Copy, Debug, Default)]
pub struct AdapterLuid {
    pub high: i32,
    pub low: u32,
}

impl AdapterLuid {
    pub fn new(high: i32, low: u32) -> Self {
        Self { high, low }
    }

    pub fn to_arg_string(&self) -> String {
        format!("{},{}", self.high, self.low)
    }

    pub fn from_arg_string(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 2 {
            return None;
        }
        let high = parts[0].parse().ok()?;
        let low = parts[1].parse().ok()?;
        Some(Self { high, low })
    }
}

/// Device UUID for GPU selection on Linux.
///
/// This 16-byte UUID uniquely identifies a Vulkan physical device and can be
/// used to ensure CEF subprocesses use the same GPU as Godot.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeviceUuid {
    pub bytes: [u8; 16],
}

impl DeviceUuid {
    pub fn new(bytes: [u8; 16]) -> Self {
        Self { bytes }
    }

    /// Convert to a hex string for passing as command line argument
    pub fn to_arg_string(&self) -> String {
        self.bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Parse from hex string (32 hex characters)
    pub fn from_arg_string(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex_str = std::str::from_utf8(chunk).ok()?;
            bytes[i] = u8::from_str_radix(hex_str, 16).ok()?;
        }
        Some(Self { bytes })
    }
}

#[derive(Clone)]
pub struct OsrApp {
    godot_backend: GodotRenderBackend,
    enable_remote_debugging: bool,
    security_config: SecurityConfig,
    /// Adapter LUID for GPU selection (Windows only)
    adapter_luid: Option<AdapterLuid>,
    /// Device UUID for GPU selection (Linux only)
    device_uuid: Option<DeviceUuid>,
}

impl Default for OsrApp {
    fn default() -> Self {
        Self::new()
    }
}

impl OsrApp {
    pub fn new() -> Self {
        Self {
            godot_backend: GodotRenderBackend::Unknown,
            enable_remote_debugging: false,
            security_config: SecurityConfig::default(),
            adapter_luid: None,
            device_uuid: None,
        }
    }

    pub fn with_godot_backend(godot_backend: GodotRenderBackend) -> Self {
        Self {
            godot_backend,
            enable_remote_debugging: false,
            security_config: SecurityConfig::default(),
            adapter_luid: None,
            device_uuid: None,
        }
    }

    /// Creates an OsrApp with the specified Godot render backend and remote debugging setting.
    ///
    /// Remote debugging should only be enabled in debug builds or when running from the editor
    /// for security purposes.
    pub fn with_options(godot_backend: GodotRenderBackend, enable_remote_debugging: bool) -> Self {
        Self {
            godot_backend,
            enable_remote_debugging,
            security_config: SecurityConfig::default(),
            adapter_luid: None,
            device_uuid: None,
        }
    }

    pub fn with_security_options(
        godot_backend: GodotRenderBackend,
        enable_remote_debugging: bool,
        security_config: SecurityConfig,
    ) -> Self {
        Self {
            godot_backend,
            enable_remote_debugging,
            security_config,
            adapter_luid: None,
            device_uuid: None,
        }
    }

    pub fn with_adapter_luid(mut self, high: i32, low: u32) -> Self {
        self.adapter_luid = Some(AdapterLuid::new(high, low));
        self
    }

    pub fn with_device_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.device_uuid = Some(DeviceUuid::new(uuid));
        self
    }

    pub fn godot_backend(&self) -> GodotRenderBackend {
        self.godot_backend
    }

    pub fn enable_remote_debugging(&self) -> bool {
        self.enable_remote_debugging
    }

    pub fn security_config(&self) -> &SecurityConfig {
        &self.security_config
    }

    pub fn adapter_luid(&self) -> Option<AdapterLuid> {
        self.adapter_luid
    }

    pub fn device_uuid(&self) -> Option<DeviceUuid> {
        self.device_uuid
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
            registrar.add_custom_scheme(Some(&"res".into()), options);
            #[cfg(not(target_os = "windows"))]
            registrar.add_custom_scheme(Some(&"res".into()), options as i32);
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
                    self.app.adapter_luid(),
                    self.app.device_uuid(),
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

#[derive(Clone)]
pub struct OsrBrowserProcessHandler {
    is_cef_ready: RefCell<bool>,
    security_config: SecurityConfig,
    adapter_luid: Option<AdapterLuid>,
    device_uuid: Option<DeviceUuid>,
}

impl Default for OsrBrowserProcessHandler {
    fn default() -> Self {
        Self::new(SecurityConfig::default(), None, None)
    }
}

impl OsrBrowserProcessHandler {
    pub fn new(
        security_config: SecurityConfig,
        adapter_luid: Option<AdapterLuid>,
        device_uuid: Option<DeviceUuid>,
    ) -> Self {
        Self {
            is_cef_ready: RefCell::new(false),
            security_config,
            adapter_luid,
            device_uuid,
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

            if let Some(luid) = &self.handler.adapter_luid {
                let luid_str = luid.to_arg_string();
                command_line.append_switch_with_value(
                    Some(&"godot-adapter-luid".into()),
                    Some(&luid_str.as_str().into()),
                );
            }

            if let Some(uuid) = &self.handler.device_uuid {
                let uuid_str = uuid.to_arg_string();
                command_line.append_switch_with_value(
                    Some(&"godot-device-uuid".into()),
                    Some(&uuid_str.as_str().into()),
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

#[derive(Clone)]
struct OsrIpcHandler {
    frame: Option<Arc<Mutex<Frame>>>,
}

impl OsrIpcHandler {
    pub fn new(frame: Option<Arc<Mutex<Frame>>>) -> Self {
        Self { frame }
    }
}

impl OsrIpcHandlerBuilder {
    pub(crate) fn build(handler: OsrIpcHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrIpcHandlerBuilder {
        handler: OsrIpcHandler,
    }

    impl V8Handler {
        fn execute(
            &self,
            _name: Option<&CefStringUtf16>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<cef::V8Value>>,
            _exception: Option<&mut CefStringUtf16>
        ) -> i32 {
            if let Some(arguments) = arguments
                && let Some(arg) = arguments.first()
                    && let Some(arg) = arg {
                        if arg.is_string() != 1 {
                            if let Some(retval) = retval {
                                *retval = v8_value_create_bool(false as _);
                            }

                            return 0;
                        }

                        let route = CefStringUtf16::from("ipcRendererToGodot");
                        let msg_str = CefStringUtf16::from(&arg.string_value());
                        if let Some(frame) = self.handler.frame.as_ref() {
                            let frame = frame.lock().unwrap();

                            let process_message = process_message_create(Some(&route));
                            if let Some(mut process_message) = process_message {
                                if let Some(argument_list) = process_message.argument_list() {
                                    argument_list.set_string(0, Some(&msg_str));
                                }

                                frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));

                                if let Some(retval) = retval {
                                    *retval = v8_value_create_bool(true as _);
                                }

                                return 1;
                            }
                        }
                    }

            if let Some(retval) = retval {
                *retval = v8_value_create_bool(false as _);
            }

            return 0;
        }
    }
}

#[derive(Clone)]
struct OsrRenderProcessHandler {}

impl OsrRenderProcessHandler {
    pub fn new() -> Self {
        Self {}
    }
}

wrap_render_process_handler! {
    pub(crate) struct RenderProcessHandlerBuilder {
        handler: OsrRenderProcessHandler,
    }

    impl RenderProcessHandler {
        fn on_context_created(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            if let Some(context) = context {
                let global = context.global();
                if let Some(global) = global
                    && let Some(frame) = frame {
                        let key: CefStringUtf16 = "sendIpcMessage".to_string().as_str().into();
                        let mut handler = OsrIpcHandlerBuilder::build(OsrIpcHandler::new(Some(Arc::new(Mutex::new(frame.clone())))));
                        let mut func = v8_value_create_function(Some(&"sendIpcMessage".into()), Some(&mut handler)).unwrap();
                        global.set_value_bykey(Some(&key), Some(&mut func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));
                    }
            }
        }

        fn on_focused_node_changed(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, node: Option<&mut Domnode>) {
            if let Some(node) = node
                && node.is_editable() == 1 {
                    // send to the browser process to activate IME
                    let route = CefStringUtf16::from("triggerIme");
                    let process_message = process_message_create(Some(&route));
                    if let Some(mut process_message) = process_message {
                        if let Some(argument_list) = process_message.argument_list() {
                            argument_list.set_bool(0, true as _);
                        }

                        if let Some(frame) = frame {
                            frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));
                            return;
                        }
                    }
                }

            // send to the browser process to deactivate IME
            let route = CefStringUtf16::from("triggerIme");
            let process_message = process_message_create(Some(&route));
            if let Some(mut process_message) = process_message {
                if let Some(argument_list) = process_message.argument_list() {
                    argument_list.set_bool(0, false as _);
                }

                if let Some(frame) = frame {
                    frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));
                }
            }
        }
    }
}

impl RenderProcessHandlerBuilder {
    pub(crate) fn build(handler: OsrRenderProcessHandler) -> RenderProcessHandler {
        Self::new(handler)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorType {
    #[default]
    Arrow,
    IBeam,
    Hand,
    Cross,
    Wait,
    Help,
    Move,
    ResizeNS,
    ResizeEW,
    ResizeNESW,
    ResizeNWSE,
    NotAllowed,
    Progress,
}

#[derive(Clone)]
pub struct OsrRenderHandler {
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<winit::dpi::PhysicalSize<f32>>>,
    pub frame_buffer: Arc<Mutex<FrameBuffer>>,
    pub cursor_type: Arc<Mutex<CursorType>>,
}

impl OsrRenderHandler {
    pub fn new(device_scale_factor: f32, size: winit::dpi::PhysicalSize<f32>) -> Self {
        Self {
            size: Arc::new(Mutex::new(size)),
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            frame_buffer: Arc::new(Mutex::new(FrameBuffer::new())),
            cursor_type: Arc::new(Mutex::new(CursorType::default())),
        }
    }

    pub fn get_frame_buffer(&self) -> Arc<Mutex<FrameBuffer>> {
        self.frame_buffer.clone()
    }

    pub fn get_size(&self) -> Arc<Mutex<winit::dpi::PhysicalSize<f32>>> {
        self.size.clone()
    }

    pub fn get_device_scale_factor(&self) -> Arc<Mutex<f32>> {
        self.device_scale_factor.clone()
    }

    pub fn get_cursor_type(&self) -> Arc<Mutex<CursorType>> {
        self.cursor_type.clone()
    }
}
