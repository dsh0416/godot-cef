mod webrender;
mod utils;

use cef::{BrowserSettings, ImplBrowser, ImplBrowserHost, RequestContextSettings, Settings, WindowInfo, api_hash, quit_message_loop, run_message_loop};
use godot::classes::{ITextureRect, Os, TextureRect};
use godot::init::*;
use godot::prelude::*;
use winit::dpi::LogicalSize;
use std::sync::Once;

use crate::utils::get_subprocess_path;

struct GodotCef;
#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

struct State {
    // window: Arc<Window>,
    // device: wgpu::Device,
    // pipeline: wgpu::RenderPipeline,
    // queue: wgpu::Queue,
    // size: winit::dpi::PhysicalSize<u32>,
    // surface: wgpu::Surface<'static>,
    // surface_format: wgpu::TextureFormat,
    // quad: Geometry,
}

struct App {
    state: Option<State>,
    browser: Option<cef::Browser>,
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
            app: App {
                state: None,
                browser: None,
            },
            url: "https://google.com".into(),
        }
    }

    fn ready(&mut self) {
        self.create_cef_texture();
    }

    fn process(&mut self, _delta: f64) {
        self.update_cef_texture();
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

        let user_data_dir = Os::singleton().get_user_data_dir();

        let settings = Settings {
            browser_subprocess_path: subprocess_path.to_str().unwrap().into(),
            windowless_rendering_enabled: true as _,
            external_message_pump: true as _,
            log_severity: cef::LogSeverity::VERBOSE as _,
            log_file: "/tmp/cef.log".into(),
            root_cache_path: user_data_dir.to_string().as_str().into(),
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
        
        let mut client = webrender::ClientBuilder::build(cef_app::OsrRenderHandler::new(
            1.0,
            LogicalSize::new(size.x as f32, size.y as f32)
        ));

        let browser = cef::browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(&browser_settings),
            None,
            context.as_mut(),
        );

        assert!(browser.is_some(), "failed to create browser");

        self.app.browser = browser;
    }

    fn create_cef_texture(&mut self) {
        CEF_INITIALIZED.call_once(|| {
            Self::load_cef_framework();
            Self::initialize_cef();
        });

        self.create_browser();
    }

    fn update_cef_texture(&mut self) {
        run_message_loop();
        quit_message_loop();
        if let Some(browser) = self.app.browser.as_mut() {
            // TODO: resize handling

            if let Some(host) = browser.host() {
                host.send_external_begin_frame();
                godot_print!("send_external_begin_frame");
            }
        }
    }
}

static CEF_INITIALIZED: Once = Once::new();
