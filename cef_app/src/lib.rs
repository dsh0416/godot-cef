mod loader;

pub use loader::{load_cef_framework_from_path, load_sandbox_from_path};

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use cef::sys::cef_v8_propertyattribute_t;
use cef::{
    self, BrowserProcessHandler, ImplBrowserProcessHandler, WrapBrowserProcessHandler, rc::Rc, *,
};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> PhysicalSize<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

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

#[derive(Default, Clone)]
pub struct PopupState {
    pub visible: bool,
    pub rect: PopupRect,
    pub buffer: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub dirty: bool,
}

#[derive(Default, Clone, Copy)]
pub struct PopupRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PopupState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.buffer.clear();
            self.width = 0;
            self.height = 0;
        }
        self.dirty = true;
    }

    pub fn set_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.rect = PopupRect {
            x,
            y,
            width,
            height,
        };
        self.dirty = true;
    }

    pub fn update_buffer(&mut self, data: Vec<u8>, width: u32, height: u32) {
        self.buffer = data;
        self.width = width;
        self.height = height;
        self.dirty = true;
    }

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

/// GPU device identifiers for GPU selection across all platforms.
///
/// These vendor and device IDs are passed to CEF via `--gpu-vendor-id` and
/// `--gpu-device-id` command-line switches to ensure CEF uses the same GPU as Godot.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuDeviceIds {
    pub vendor_id: u32,
    pub device_id: u32,
}

impl GpuDeviceIds {
    pub fn new(vendor_id: u32, device_id: u32) -> Self {
        Self {
            vendor_id,
            device_id,
        }
    }

    /// Format vendor ID as decimal string for command line argument
    pub fn to_vendor_arg(&self) -> String {
        format!("{}", self.vendor_id)
    }

    /// Format device ID as decimal string for command line argument
    pub fn to_device_arg(&self) -> String {
        format!("{}", self.device_id)
    }
}

#[derive(Clone)]
pub struct OsrApp {
    godot_backend: GodotRenderBackend,
    enable_remote_debugging: bool,
    security_config: SecurityConfig,
    /// GPU device IDs for GPU selection (all platforms)
    gpu_device_ids: Option<GpuDeviceIds>,
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
            gpu_device_ids: None,
        }
    }

    pub fn with_godot_backend(godot_backend: GodotRenderBackend) -> Self {
        Self {
            godot_backend,
            enable_remote_debugging: false,
            security_config: SecurityConfig::default(),
            gpu_device_ids: None,
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
            gpu_device_ids: None,
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
            gpu_device_ids: None,
        }
    }

    pub fn with_gpu_device_ids(mut self, vendor_id: u32, device_id: u32) -> Self {
        self.gpu_device_ids = Some(GpuDeviceIds::new(vendor_id, device_id));
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

    pub fn gpu_device_ids(&self) -> Option<GpuDeviceIds> {
        self.gpu_device_ids
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
struct OsrImeCaretHandler {
    frame: Option<Arc<Mutex<Frame>>>,
}

impl OsrImeCaretHandler {
    pub fn new(frame: Option<Arc<Mutex<Frame>>>) -> Self {
        Self { frame }
    }
}

impl OsrImeCaretHandlerBuilder {
    pub(crate) fn build(handler: OsrImeCaretHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrImeCaretHandlerBuilder {
        handler: OsrImeCaretHandler,
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
                && arguments.len() >= 3
                && let Some(Some(x_arg)) = arguments.first()
                && let Some(Some(y_arg)) = arguments.get(1)
                && let Some(Some(height_arg)) = arguments.get(2)
            {
                let x = x_arg.int_value();
                let y = y_arg.int_value();
                let height = height_arg.int_value();

                if let Some(frame) = self.handler.frame.as_ref() {
                    match frame.lock() {
                        Ok(frame) => {
                            let route = CefStringUtf16::from("imeCaretPosition");
                            let process_message = process_message_create(Some(&route));
                            if let Some(mut process_message) = process_message {
                                if let Some(argument_list) = process_message.argument_list() {
                                    argument_list.set_int(0, x);
                                    argument_list.set_int(1, y);
                                    argument_list.set_int(2, height);
                                }

                                frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));

                                if let Some(retval) = retval {
                                    *retval = v8_value_create_bool(true as _);
                                }

                                return 1;
                            }
                        }
                        Err(_) => {
                            if let Some(retval) = retval {
                                *retval = v8_value_create_bool(false as _);
                            }
                            return 0;
                        }
                    }
                }
            }

            if let Some(retval) = retval {
                *retval = v8_value_create_bool(false as _);
            }

            0
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
                        let frame_arc = Arc::new(Mutex::new(frame.clone()));

                        let key: CefStringUtf16 = "sendIpcMessage".to_string().as_str().into();
                        let mut handler = OsrIpcHandlerBuilder::build(OsrIpcHandler::new(Some(frame_arc.clone())));
                        let mut func = v8_value_create_function(Some(&"sendIpcMessage".into()), Some(&mut handler)).unwrap();
                        global.set_value_bykey(Some(&key), Some(&mut func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

                        let caret_key: CefStringUtf16 = "__sendImeCaretPosition".into();
                        let mut caret_handler = OsrImeCaretHandlerBuilder::build(OsrImeCaretHandler::new(Some(frame_arc)));
                        let mut caret_func = v8_value_create_function(Some(&"__sendImeCaretPosition".into()), Some(&mut caret_handler)).unwrap();
                        global.set_value_bykey(Some(&caret_key), Some(&mut caret_func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

                        let helper_script: CefStringUtf16 = include_str!("ime_helper.js").into();
                        frame.execute_java_script(Some(&helper_script), None, 0);
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
                            let report_script: CefStringUtf16 = "if(window.__activateImeTracking)window.__activateImeTracking();".into();
                            frame.execute_java_script(Some(&report_script), None, 0);
                        }
                    }
                    return;
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
                    let deactivate_script: CefStringUtf16 = "if(window.__deactivateImeTracking)window.__deactivateImeTracking();".into();
                    frame.execute_java_script(Some(&deactivate_script), None, 0);
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
    pub size: Arc<Mutex<PhysicalSize<f32>>>,
    pub frame_buffer: Arc<Mutex<FrameBuffer>>,
    pub cursor_type: Arc<Mutex<CursorType>>,
    pub popup_state: Arc<Mutex<PopupState>>,
}

impl OsrRenderHandler {
    pub fn new(device_scale_factor: f32, size: PhysicalSize<f32>) -> Self {
        Self {
            size: Arc::new(Mutex::new(size)),
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            frame_buffer: Arc::new(Mutex::new(FrameBuffer::new())),
            cursor_type: Arc::new(Mutex::new(CursorType::default())),
            popup_state: Arc::new(Mutex::new(PopupState::new())),
        }
    }

    pub fn get_frame_buffer(&self) -> Arc<Mutex<FrameBuffer>> {
        self.frame_buffer.clone()
    }

    pub fn get_size(&self) -> Arc<Mutex<PhysicalSize<f32>>> {
        self.size.clone()
    }

    pub fn get_device_scale_factor(&self) -> Arc<Mutex<f32>> {
        self.device_scale_factor.clone()
    }

    pub fn get_cursor_type(&self) -> Arc<Mutex<CursorType>> {
        self.cursor_type.clone()
    }

    pub fn get_popup_state(&self) -> Arc<Mutex<PopupState>> {
        self.popup_state.clone()
    }
}
