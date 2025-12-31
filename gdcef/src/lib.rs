mod webrender;
mod utils;

use cef::{BrowserSettings, ImplBrowser, ImplBrowserHost, RequestContextSettings, Settings, WindowInfo, api_hash, quit_message_loop, run_message_loop};
use cef_app::FrameBuffer;
use godot::classes::notify::ControlNotification;
use godot::classes::{ITextureRect, Image, ImageTexture, Os, TextureRect};
use godot::classes::image::Format as ImageFormat;
use godot::init::*;
use godot::prelude::*;
use winit::dpi::LogicalSize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use crate::utils::get_subprocess_path;

struct GodotCef;
#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

struct App {
    browser: Option<cef::Browser>,
    frame_buffer: Option<Arc<Mutex<FrameBuffer>>>,
    texture: Option<Gd<ImageTexture>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            browser: None,
            frame_buffer: None,
            texture: None,
        }
    }
}

#[derive(GodotClass)]
#[class(base=TextureRect)]
struct CefTexture {
    base: Base<TextureRect>,

    // internal states
    app: App,

    #[export]
    url: GString,
}

#[godot_api]
impl ITextureRect for CefTexture {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
        }
    }

    fn ready(&mut self) {
        self.on_ready();
    }

    fn process(&mut self, _delta: f64) {
        self.on_process();
    }

    fn on_notification(&mut self, what: ControlNotification) {
        match what {
            ControlNotification::WM_CLOSE_REQUEST => {
                self.shutdown_cef();
            }
            _ => {}
        }
    }
}

#[godot_api]
impl CefTexture {
    fn load_cef_framework() {
        #[cfg(target_os = "macos")]
        {
            use cef::sys::cef_load_library;

            let framework_path = utils::get_framework_path();
            let path = framework_path
                .unwrap()
                .join("Chromium Embedded Framework")
                .canonicalize()
                .unwrap();

            use std::os::unix::ffi::OsStrExt;
            let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
                panic!("Failed to convert library path to CString");
            };
            let result = unsafe {
                let arg_path = Some(&*path.as_ptr().cast());
                let arg_path = arg_path.map(std::ptr::from_ref).unwrap_or(std::ptr::null());
                cef_load_library(arg_path) == 1
            };

            assert!(result, "Failed to load macOS CEF framework");

            // set the API hash
            let _ = api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        };
    }

    #[cfg(all(target_os = "macos"))]
    fn load_sandbox(args: &cef::MainArgs) {
        use libloading::Library;

        let framework_path = utils::get_framework_path();
        let path = framework_path
            .unwrap()
            .join("Libraries/libcef_sandbox.dylib")
            .canonicalize()
            .unwrap();

        unsafe {
            let lib = Library::new(path).unwrap();
            let func = lib.get::<unsafe extern "C" fn(
                argc: std::os::raw::c_int,
                argv: *mut *mut ::std::os::raw::c_char,
            )>(b"cef_sandbox_initialize\0").unwrap();
            func(args.argc, args.argv);
        }
    }

    fn initialize_cef() {
        let args = cef::args::Args::new();
        let mut app = cef_app::AppBuilder::build(cef_app::OsrApp::new());

        #[cfg(all(target_os = "macos"))]
        Self::load_sandbox(args.as_main_args());

        // FIXME: cross-platform
        let subprocess_path = get_subprocess_path().unwrap();

        godot_print!("subprocess_path: {}", subprocess_path.to_str().unwrap());

        let user_data_dir = PathBuf::from(Os::singleton().get_user_data_dir().to_string());
        let root_cache_path = user_data_dir.join("Godot CEF/Cache");

        let settings = Settings {
            browser_subprocess_path: subprocess_path.to_str().unwrap().into(),
            windowless_rendering_enabled: true as _,
            external_message_pump: true as _,
            log_severity: cef::LogSeverity::VERBOSE as _,
            // log_file: "/tmp/cef.log".into(),
            root_cache_path: root_cache_path.to_str().unwrap().into(),
            ..Default::default()
        };

        #[cfg(target_os = "macos")]
        let settings = Settings {
            framework_dir_path: utils::get_framework_path().unwrap().to_str().unwrap().into(),
            main_bundle_path: get_subprocess_path().unwrap().join("../../..").canonicalize().unwrap().to_str().unwrap().into(),
            ..settings
        };

        let ret = cef::initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut()
        );

        assert_eq!(ret, 1, "failed to initialize CEF");
    }

    fn shutdown_cef(&mut self) {
        self.app.browser = None;
        self.app.frame_buffer = None;
        self.app.texture = None;

        if CEF_INITIALIZED.is_completed() {
            cef::shutdown();
        }
    }

    fn create_browser(&mut self) {
        let size = self.base().get_size();
        let window_info = WindowInfo {
            bounds: cef::Rect {
                x: 0 as _,
                y: 0 as _,
                width: size.x as _,
                height: size.y as _,
            },
            windowless_rendering_enabled: true as _,
            shared_texture_enabled: false as _,
            external_begin_frame_enabled: true as _,
            ..Default::default()
        };

        let browser_settings = BrowserSettings {
            // windowless_frame_rate: 60, // FIXME: should be dynamic
            ..Default::default()
        };

        let mut context = cef::request_context_create_context(
            Some(&RequestContextSettings::default()),
            Some(&mut webrender::RequestContextHandlerBuilder::build(webrender::OsrRequestContextHandler {})),
        );

        // Create the render handler and get a reference to its frame buffer
        let render_handler = cef_app::OsrRenderHandler::new(
            1.0,
            LogicalSize::new(size.x as f32, size.y as f32)
        );
        let frame_buffer = render_handler.get_frame_buffer();
        
        let mut client = webrender::ClientBuilder::build(render_handler);

        let browser = cef::browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(&browser_settings),
            None,
            context.as_mut(),
        );

        assert!(browser.is_some(), "failed to create browser");

        // Create the ImageTexture that will display CEF frames
        let texture = ImageTexture::new_gd();
        self.base_mut().set_texture(&texture);

        self.app.browser = browser;
        self.app.frame_buffer = Some(frame_buffer);
        self.app.texture = Some(texture);
    }

    fn on_ready(&mut self) {
        CEF_INITIALIZED.call_once(|| {
            Self::load_cef_framework();
            Self::initialize_cef();
        });

        self.create_browser();
    }

    fn on_process(&mut self) {
        run_message_loop();
        quit_message_loop();

        if let Some(browser) = self.app.browser.as_mut() {
            // TODO: resize handling

            if let Some(host) = browser.host() {
                host.send_external_begin_frame();
            }
        }

        // Update texture from frame buffer if dirty
        self.update_texture_from_buffer();
    }

    fn update_texture_from_buffer(&mut self) {
        let Some(frame_buffer_arc) = &self.app.frame_buffer else {
            return;
        };

        let Some(texture) = &mut self.app.texture else {
            return;
        };

        // Try to lock the frame buffer
        let Ok(mut frame_buffer) = frame_buffer_arc.lock() else {
            return;
        };

        // Only update if the buffer has new data
        if !frame_buffer.dirty || frame_buffer.data.is_empty() {
            return;
        }

        let width = frame_buffer.width as i32;
        let height = frame_buffer.height as i32;

        // Create a PackedByteArray from the RGBA data
        let byte_array = PackedByteArray::from(frame_buffer.data.as_slice());

        // Create a Godot Image from the raw RGBA data
        let image = Image::create_from_data(
            width,
            height,
            false, // no mipmaps
            ImageFormat::RGBA8,
            &byte_array,
        );

        if let Some(image) = image {
            // Update the ImageTexture with the new image
            texture.set_image(&image);
        }

        // Mark the buffer as consumed
        frame_buffer.mark_clean();
    }
}

static CEF_INITIALIZED: Once = Once::new();
