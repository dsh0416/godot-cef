use cef::{ImplBrowser, ImplBrowserHost, ImplFrame, ImplListValue, ImplProcessMessage};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ObjectNotification;
use godot::classes::{
    Engine, ITexture2D, Image, ImageTexture, InputEvent, InputEventKey, InputEventMagnifyGesture,
    InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture, InputEventScreenDrag,
    InputEventScreenTouch, RenderingServer, SceneTree, Texture2D,
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
    permission_policy: i32,
    preload_script: GString,
    preload_script_path: GString,
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
    #[var(
        get = get_enable_accelerated_osr,
        set = set_enable_accelerated_osr
    )]
    enable_accelerated_osr: bool,

    #[export]
    #[var(
        get = get_background_color,
        set = set_background_color
    )]
    background_color: Color,

    #[export(enum = (Block = 0, Redirect = 1, SignalOnly = 2))]
    #[var(get = get_popup_policy, set = set_popup_policy)]
    popup_policy: i32,

    #[export(enum = (ProjectDefault = -1, DenyAll = 0, AllowAll = 1, Signal = 2))]
    #[var(get = get_permission_policy, set = set_permission_policy)]
    permission_policy: i32,

    #[export]
    #[var(get = get_preload_script, set = set_preload_script)]
    preload_script: GString,

    #[export]
    #[var(get = get_preload_script_path, set = set_preload_script_path)]
    preload_script_path: GString,

    #[export]
    #[var(get = get_texture_size_property, set = set_texture_size_property)]
    texture_size: Vector2i,

    last_find_query: GString,
    last_find_match_case: bool,
    touch_id_map: HashMap<i32, i32>,
    next_touch_id: i32,
    process_hook: Option<(InstanceId, Callable)>,
    frame_hook: Option<Callable>,
}

#[godot_api]
impl ITexture2D for CefTexture2D {
    fn init(base: Base<Texture2D>) -> Self {
        let texture_size = Vector2i::new(1024, 1024);
        let fallback_texture = Self::make_placeholder_texture(texture_size);
        let editor_hint = Engine::singleton().is_editor_hint();
        // Wait until construction and scene loading finish. Method callables only
        // keep the object ID, so neither this queued call nor the signal hooks
        // keep an otherwise unused resource alive.
        if !editor_hint {
            base.to_init_gd()
                .call_deferred("_connect_runtime_hooks", &[]);
        }

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
            permission_policy: -1,
            preload_script: GString::new(),
            preload_script_path: GString::new(),
            texture_size,
            last_find_query: GString::new(),
            last_find_match_case: false,
            touch_id_map: HashMap::new(),
            next_touch_id: 0,
            process_hook: None,
            frame_hook: None,
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
