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

#[derive(Clone)]
pub struct OsrApp {
    godot_backend: GodotRenderBackend,
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
        }
    }

    pub fn with_godot_backend(godot_backend: GodotRenderBackend) -> Self {
        Self { godot_backend }
    }

    pub fn godot_backend(&self) -> GodotRenderBackend {
        self.godot_backend
    }
}

wrap_app! {
    pub struct AppBuilder {
        app: OsrApp,
    }

    impl App {
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
            command_line
                .append_switch_with_value(Some(&"remote-debugging-port".into()), Some(&"9229".into()));

            match self.app.godot_backend() {
                GodotRenderBackend::Direct3D12 => {
                    command_line.append_switch_with_value(Some(&"use-gl".into()), Some(&"angle".into()));
                    command_line.append_switch_with_value(Some(&"use-angle".into()), Some(&"d3d11on12".into()));
                }
                GodotRenderBackend::Metal => {
                    command_line.append_switch_with_value(Some(&"use-gl".into()), Some(&"angle".into()));
                    command_line.append_switch_with_value(Some(&"use-angle".into()), Some(&"metal".into()));
                }
                #[cfg(target_os = "macos")]
                // using --use=angle=vulkan would disables GPU acceleration on macOS.
                // thus we keep using metal backend for Vulkan on macOS.
                // We use MoltenVK on macOS to translate Vulkan to Metal.
                GodotRenderBackend::Vulkan => {
                    command_line.append_switch_with_value(Some(&"use-gl".into()), Some(&"angle".into()));
                    command_line.append_switch_with_value(Some(&"use-angle".into()), Some(&"metal".into()));
                }
                #[cfg(not(target_os = "macos"))]
                GodotRenderBackend::Vulkan => {
                    command_line.append_switch_with_value(Some(&"use-gl".into()), Some(&"angle".into()));
                    command_line.append_switch_with_value(Some(&"use-angle".into()), Some(&"vulkan".into()));
                }
                _ => {}
            }
        }

        fn browser_process_handler(&self) -> Option<cef::BrowserProcessHandler> {
            Some(BrowserProcessHandlerBuilder::build(
                OsrBrowserProcessHandler::new(),
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
}

impl Default for OsrBrowserProcessHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl OsrBrowserProcessHandler {
    pub fn new() -> Self {
        Self {
            is_cef_ready: RefCell::new(false),
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

            command_line.append_switch(Some(&"disable-web-security".into()));
            command_line.append_switch(Some(&"allow-running-insecure-content".into()));
            command_line.append_switch(Some(&"disable-session-crashed-bubble".into()));
            command_line.append_switch(Some(&"ignore-certificate-errors".into()));
            command_line.append_switch(Some(&"ignore-ssl-errors".into()));
            command_line.append_switch(Some(&"enable-logging=stderr".into()));
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
