use cef::{ImplBrowser, ImplBrowserHost, ImplFrame, ImplListValue, ImplProcessMessage};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ObjectNotification;
use godot::classes::{
    Engine, ITexture2D, Image, ImageTexture, InputEvent, InputEventKey, InputEventMagnifyGesture,
    InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture, InputEventScreenDrag,
    InputEventScreenTouch, RenderingServer, Texture2D,
};
use godot::prelude::*;
use std::collections::HashMap;

use crate::browser::{App, RenderMode};
use crate::cef_init;
use crate::cef_texture::backend;
use crate::input;
use crate::render;
use cef_app::ipc_contract::{
    ROUTE_IPC_BINARY_GODOT_TO_RENDERER, ROUTE_IPC_DATA_GODOT_TO_RENDERER,
    ROUTE_IPC_GODOT_TO_RENDERER,
};

mod lifecycle;
mod rendering;
mod runtime;

pub(crate) struct CefTextureRuntime {
    app: App,
    last_size: Vector2,
    last_dpi: f32,
    last_max_fps: i32,
    runtime_enabled: bool,
}

pub(crate) struct RuntimeCreateConfig {
    logical_size: Vector2,
    dpi: f32,
    url: GString,
    enable_accelerated_osr: bool,
    background_color: Color,
    popup_policy: i32,
    software_target_texture: Option<Gd<ImageTexture>>,
    log_prefix: &'static str,
}

// Runtime implementation moved to `runtime.rs`.

#[derive(GodotClass)]
#[class(base=Texture2D, tool)]
pub struct CefTexture2D {
    base: Base<Texture2D>,
    runtime: CefTextureRuntime,
    fallback_texture: Gd<ImageTexture>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    stable_texture_2d_rd: Option<Gd<godot::classes::Texture2Drd>>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    placeholder_rd_rid: Rid,

    #[export]
    #[var(get = get_url_property, set = set_url_property)]
    url: GString,

    #[export]
    enable_accelerated_osr: bool,

    #[export]
    background_color: Color,

    #[export(enum = (Block = 0, Redirect = 1, SignalOnly = 2))]
    #[var(get = get_popup_policy, set = set_popup_policy)]
    popup_policy: i32,

    #[export]
    #[var(get = get_texture_size_property, set = set_texture_size_property)]
    texture_size: Vector2i,

    last_find_query: GString,
    last_find_match_case: bool,
    touch_id_map: HashMap<i32, i32>,
    next_touch_id: i32,
    frame_hook_callable: Option<Callable>,
    frame_hook_connected: bool,
}

#[godot_api]
impl ITexture2D for CefTexture2D {
    fn init(base: Base<Texture2D>) -> Self {
        let texture_size = Vector2i::new(1024, 1024);
        let fallback_texture = Self::make_placeholder_texture(texture_size);
        let editor_hint = Engine::singleton().is_editor_hint();
        let frame_hook_callable = base.to_init_gd().callable("_on_frame_pre_draw");
        RenderingServer::singleton().connect("frame_pre_draw", &frame_hook_callable);

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        let (stable_texture_2d_rd, placeholder_rd_rid) =
            match render::create_rd_texture(texture_size.x, texture_size.y) {
                Ok((rd_rid, t2d)) => (Some(t2d), rd_rid),
                Err(_) => (None, Rid::Invalid),
            };

        Self {
            base,
            runtime: CefTextureRuntime::new(!editor_hint),
            fallback_texture,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            stable_texture_2d_rd,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            placeholder_rd_rid,
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
            background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
            popup_policy: crate::browser::popup_policy::BLOCK,
            texture_size,
            last_find_query: GString::new(),
            last_find_match_case: false,
            touch_id_map: HashMap::new(),
            next_touch_id: 0,
            frame_hook_callable: Some(frame_hook_callable),
            frame_hook_connected: true,
        }
    }

    fn on_notification(&mut self, what: ObjectNotification) {
        if what == ObjectNotification::PREDELETE {
            self.cleanup_instance()
        }
    }

    fn get_width(&self) -> i32 {
        self.texture_size.x
    }

    fn get_height(&self) -> i32 {
        self.texture_size.y
    }

    fn has_alpha(&self) -> bool {
        true
    }

    fn get_rid(&self) -> Rid {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if self.enable_accelerated_osr
            && let Some(stable) = &self.stable_texture_2d_rd
        {
            return stable.get_rid();
        }

        self.fallback_texture.get_rid()
    }
}

include!("api.rs");
