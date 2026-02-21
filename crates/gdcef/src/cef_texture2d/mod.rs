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

impl CefTextureRuntime {
    pub(crate) fn new(runtime_enabled: bool) -> Self {
        Self {
            app: App::default(),
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_max_fps: 0,
            runtime_enabled,
        }
    }

    pub(crate) fn app(&self) -> &App {
        &self.app
    }

    pub(crate) fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub(crate) fn runtime_enabled(&self) -> bool {
        self.runtime_enabled
    }

    pub(crate) fn set_runtime_enabled(&mut self, enabled: bool) {
        self.runtime_enabled = enabled;
    }

    pub(crate) fn set_url(&self, url: GString) {
        if let Some(state) = self.app.state.as_ref()
            && let Some(frame) = state.browser.main_frame()
        {
            let url_str: cef::CefStringUtf16 = url.to_string().as_str().into();
            frame.load_url(Some(&url_str));
        }
    }

    pub(crate) fn get_live_url_or(&self, fallback: &GString) -> GString {
        if let Some(state) = self.app.state.as_ref()
            && let Some(frame) = state.browser.main_frame()
        {
            let frame_url = frame.url();
            let url_string = cef::CefStringUtf16::from(&frame_url).to_string();
            return GString::from(url_string.as_str());
        }
        fallback.clone()
    }

    pub(crate) fn apply_popup_policy(&self, policy: i32) {
        backend::apply_popup_policy(&self.app, policy);
    }

    pub(crate) fn shutdown(&mut self) {
        self.runtime_enabled = false;
    }

    pub(crate) fn try_create_browser(&mut self, config: RuntimeCreateConfig) {
        let RuntimeCreateConfig {
            logical_size,
            dpi,
            url,
            enable_accelerated_osr,
            background_color,
            popup_policy,
            software_target_texture,
            log_prefix,
        } = config;
        if !self.runtime_enabled || self.app.state.is_some() {
            return;
        }
        if let Err(e) = cef_init::cef_retain() {
            godot::global::godot_error!("[{}] {}", log_prefix, e);
            return;
        }
        self.app.mark_cef_retained();
        let params = backend::BackendCreateParams {
            logical_size,
            dpi,
            max_fps: backend::get_max_fps(),
            url: url.to_string(),
            enable_accelerated_osr,
            background_color,
            popup_policy,
            software_target_texture,
            log_prefix,
        };
        if let Err(e) = backend::try_create_browser(&mut self.app, &params) {
            godot::global::godot_error!("[{}] {}", log_prefix, e);
            self.app.release_cef_if_retained();
            return;
        }
        self.last_size = logical_size;
        self.last_dpi = dpi;
    }

    pub(crate) fn handle_max_fps_change(&mut self) {
        let max_fps = backend::get_max_fps();
        backend::handle_max_fps_change(&self.app, &mut self.last_max_fps, max_fps);
    }

    pub(crate) fn handle_size_change(&mut self, logical_size: Vector2, dpi: f32) -> bool {
        backend::handle_size_change(
            &self.app,
            &mut self.last_size,
            &mut self.last_dpi,
            logical_size,
            dpi,
        )
    }

    pub(crate) fn update_primary_texture(
        &mut self,
        log_prefix: &str,
    ) -> Option<Gd<godot::classes::Texture2Drd>> {
        let Some(state) = &mut self.app.state else {
            return None;
        };
        backend::update_primary_texture(state, log_prefix)
    }

    pub(crate) fn message_loop_and_begin_frame(&self) {
        if self.app.state.is_some() {
            cef::do_message_loop_work();
        }
        backend::request_external_begin_frame(&self.app);
    }

    pub(crate) fn cleanup_runtime(
        &mut self,
        popup_texture_2d_rd: Option<&mut Gd<godot::classes::Texture2Drd>>,
    ) {
        backend::cleanup_runtime(&mut self.app, popup_texture_2d_rd);
    }

    pub(crate) fn drain_event_queues(&self, log_prefix: &str) {
        let Some(event_queues) = self.app.state.as_ref().map(|state| &state.event_queues) else {
            return;
        };

        let Ok(mut queues) = event_queues.lock() else {
            godot::global::godot_warn!(
                "[{}] Failed to lock event queues while draining events",
                log_prefix
            );
            return;
        };

        let _ = std::mem::take(&mut *queues);
    }
}

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
        let fallback_texture = make_placeholder_texture(texture_size);
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

#[godot_api]
impl CefTexture2D {
    pub(crate) fn runtime_app(&self) -> &App {
        self.runtime.app()
    }

    pub(crate) fn runtime_app_mut(&mut self) -> &mut App {
        self.runtime.app_mut()
    }

    pub(crate) fn set_enable_accelerated_osr_property(&mut self, enabled: bool) {
        self.enable_accelerated_osr = enabled;
    }

    pub(crate) fn set_background_color_property(&mut self, color: Color) {
        self.background_color = color;
    }

    /// Updates local URL property without triggering navigation.
    pub(crate) fn set_url_state_property(&mut self, url: GString) {
        self.url = url;
    }

    /// Updates local popup policy property without applying to a live browser.
    pub(crate) fn set_popup_policy_state(&mut self, policy: i32) {
        self.popup_policy = policy;
    }

    #[func]
    fn _on_frame_pre_draw(&mut self) {
        self.tick();
    }

    #[func]
    pub(crate) fn set_url_property(&mut self, url: GString) {
        self.url = url.clone();
        self.runtime.set_url(url);
    }

    #[func]
    pub(crate) fn get_url_property(&self) -> GString {
        self.runtime.get_live_url_or(&self.url)
    }

    #[func]
    pub(crate) fn get_popup_policy(&self) -> i32 {
        self.popup_policy
    }

    #[func]
    pub(crate) fn set_popup_policy(&mut self, policy: i32) {
        self.popup_policy = policy;
        self.runtime.apply_popup_policy(policy);
    }

    #[func]
    pub(crate) fn get_texture_size_property(&self) -> Vector2i {
        self.texture_size
    }

    #[func]
    pub(crate) fn set_texture_size_property(&mut self, size: Vector2i) {
        let clamped = Vector2i::new(size.x.max(1), size.y.max(1));
        if clamped == self.texture_size {
            return;
        }

        self.texture_size = clamped;
        self.refresh_fallback_texture();

        // Notify the backend that the browser size has changed so it can
        // resize the off-screen rendering accordingly.
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        self.runtime.handle_size_change(logical_size, dpi);
        self.base_mut().emit_changed();
    }

    #[func]
    pub fn shutdown(&mut self) {
        self.runtime.shutdown();
        self.cleanup_instance();
    }

    #[func]
    pub fn eval(&mut self, code: GString) {
        let Some(state) = self.runtime.app().state.as_ref() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot execute JS: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot execute JS: no main frame");
            return;
        };
        let code_str: cef::CefStringUtf16 = code.to_string().as_str().into();
        frame.execute_java_script(Some(&code_str), None, 0);
    }

    #[func]
    pub fn go_back(&mut self) {
        if let Some(browser) = self.runtime.app_mut().browser_mut() {
            browser.go_back();
        }
    }

    #[func]
    pub fn go_forward(&mut self) {
        if let Some(browser) = self.runtime.app_mut().browser_mut() {
            browser.go_forward();
        }
    }

    #[func]
    pub fn can_go_back(&self) -> bool {
        self.runtime
            .app()
            .browser()
            .map(|b| b.can_go_back() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn can_go_forward(&self) -> bool {
        self.runtime
            .app()
            .browser()
            .map(|b| b.can_go_forward() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn reload(&mut self) {
        if let Some(browser) = self.runtime.app_mut().browser_mut() {
            browser.reload();
        }
    }

    #[func]
    pub fn reload_ignore_cache(&mut self) {
        if let Some(browser) = self.runtime.app_mut().browser_mut() {
            browser.reload_ignore_cache();
        }
    }

    #[func]
    pub fn stop_loading(&mut self) {
        if let Some(browser) = self.runtime.app_mut().browser_mut() {
            browser.stop_load();
        }
    }

    #[func]
    pub fn is_loading(&self) -> bool {
        self.runtime
            .app()
            .browser()
            .map(|b| b.is_loading() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn set_zoom_level(&mut self, level: f64) {
        if let Some(host) = self.runtime.app().host() {
            host.set_zoom_level(level);
        }
    }

    #[func]
    pub fn get_zoom_level(&self) -> f64 {
        self.runtime
            .app()
            .host()
            .map(|h| h.zoom_level())
            .unwrap_or(0.0)
    }

    #[func]
    pub fn set_audio_muted(&mut self, muted: bool) {
        if let Some(host) = self.runtime.app().host() {
            host.set_audio_muted(muted as i32);
        }
    }

    #[func]
    pub fn is_audio_muted(&self) -> bool {
        self.runtime
            .app()
            .host()
            .map(|h| h.is_audio_muted() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn send_ipc_message(&mut self, message: GString) {
        let Some(state) = self.runtime.app().state.as_ref() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot send IPC message: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot send IPC message: no main frame");
            return;
        };

        let route = cef::CefStringUtf16::from(ROUTE_IPC_GODOT_TO_RENDERER);
        let msg_str: cef::CefStringUtf16 = message.to_string().as_str().into();

        if let Some(mut process_message) = cef::process_message_create(Some(&route)) {
            if let Some(argument_list) = process_message.argument_list() {
                argument_list.set_string(0, Some(&msg_str));
            }
            frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut process_message));
        }
    }

    #[func]
    pub fn send_ipc_binary_message(&mut self, data: PackedByteArray) {
        let Some(state) = self.runtime.app().state.as_ref() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot send binary IPC message: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send binary IPC message: no main frame"
            );
            return;
        };

        let route = cef::CefStringUtf16::from(ROUTE_IPC_BINARY_GODOT_TO_RENDERER);
        let bytes = data.to_vec();
        let Some(mut binary_value) = cef::binary_value_create(Some(&bytes)) else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send binary IPC message: failed to create BinaryValue"
            );
            return;
        };
        let Some(mut process_message) = cef::process_message_create(Some(&route)) else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send binary IPC message: failed to create process message"
            );
            return;
        };
        let Some(argument_list) = process_message.argument_list() else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send binary IPC message: failed to get argument list"
            );
            return;
        };
        argument_list.set_binary(0, Some(&mut binary_value));
        frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut process_message));
    }

    #[func]
    pub fn send_ipc_data(&mut self, data: Variant) {
        let Some(state) = self.runtime.app().state.as_ref() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot send IPC data: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture2D] Cannot send IPC data: no main frame");
            return;
        };
        let bytes = match crate::ipc_data::encode_variant_to_cbor_bytes(&data) {
            Ok(bytes) => bytes,
            Err(err) => {
                godot::global::godot_warn!("[CefTexture2D] Cannot encode IPC data: {}", err);
                return;
            }
        };
        if bytes.len() > crate::ipc_data::max_ipc_data_bytes() {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send IPC data: payload too large ({} bytes)",
                bytes.len()
            );
            return;
        }
        let route = cef::CefStringUtf16::from(ROUTE_IPC_DATA_GODOT_TO_RENDERER);
        let Some(mut binary_value) = cef::binary_value_create(Some(&bytes)) else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send IPC data: failed to create BinaryValue"
            );
            return;
        };
        let Some(mut process_message) = cef::process_message_create(Some(&route)) else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send IPC data: failed to create process message"
            );
            return;
        };
        let Some(argument_list) = process_message.argument_list() else {
            godot::global::godot_warn!(
                "[CefTexture2D] Cannot send IPC data: failed to get argument list"
            );
            return;
        };
        argument_list.set_binary(0, Some(&mut binary_value));
        frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut process_message));
    }

    #[func]
    pub fn find_text(&mut self, query: GString, forward: bool, match_case: bool) {
        let Some(host) = self.runtime.app().host() else {
            return;
        };
        let query_string = query.to_string();
        if query_string.is_empty() {
            host.stop_finding(true as _);
            self.last_find_query = GString::new();
            self.last_find_match_case = false;
            return;
        }
        let query_cef: cef::CefStringUtf16 = query_string.as_str().into();
        host.find(
            Some(&query_cef),
            forward as _,
            match_case as _,
            false as _, // new search
        );
        self.last_find_query = query;
        self.last_find_match_case = match_case;
    }

    #[func]
    pub fn find_next(&mut self) {
        let Some(host) = self.runtime.app().host() else {
            return;
        };
        if self.last_find_query.is_empty() {
            return;
        }
        let query_string = self.last_find_query.to_string();
        let query_cef: cef::CefStringUtf16 = query_string.as_str().into();
        host.find(
            Some(&query_cef),
            true as _,
            self.last_find_match_case as _,
            true as _, // continue existing search
        );
    }

    #[func]
    pub fn find_previous(&mut self) {
        let Some(host) = self.runtime.app().host() else {
            return;
        };
        if self.last_find_query.is_empty() {
            return;
        }
        let query_string = self.last_find_query.to_string();
        let query_cef: cef::CefStringUtf16 = query_string.as_str().into();
        host.find(
            Some(&query_cef),
            false as _,
            self.last_find_match_case as _,
            true as _, // continue existing search
        );
    }

    #[func]
    pub fn stop_finding(&mut self) {
        if let Some(host) = self.runtime.app().host() {
            host.stop_finding(true as _);
        }
        self.last_find_query = GString::new();
        self.last_find_match_case = false;
    }

    fn with_host<R>(&self, f: impl FnOnce(cef::BrowserHost) -> R) -> Option<R> {
        self.runtime.app().host().map(f)
    }

    #[func]
    pub fn forward_mouse_button_event(
        &self,
        event: Gd<InputEventMouseButton>,
        pixel_scale_factor: f32,
        device_scale_factor: f32,
    ) {
        let _ = self.with_host(|host| {
            input::handle_mouse_button(&host, &event, pixel_scale_factor, device_scale_factor);
        });
    }

    #[func]
    pub fn forward_mouse_motion_event(
        &self,
        event: Gd<InputEventMouseMotion>,
        pixel_scale_factor: f32,
        device_scale_factor: f32,
    ) {
        let _ = self.with_host(|host| {
            input::handle_mouse_motion(&host, &event, pixel_scale_factor, device_scale_factor);
        });
    }

    #[func]
    pub fn forward_pan_gesture_event(
        &self,
        event: Gd<InputEventPanGesture>,
        pixel_scale_factor: f32,
        device_scale_factor: f32,
    ) {
        let _ = self.with_host(|host| {
            input::handle_pan_gesture(&host, &event, pixel_scale_factor, device_scale_factor);
        });
    }

    #[func]
    pub fn forward_magnify_gesture_event(&self, event: Gd<InputEventMagnifyGesture>) {
        let _ = self.with_host(|host| {
            input::handle_magnify_gesture(&host, &event);
        });
    }

    #[func]
    pub fn forward_key_event(&self, event: Gd<InputEventKey>, focus_on_editable_field: bool) {
        let Some(host) = self.runtime.app().host() else {
            return;
        };
        let frame = self
            .runtime
            .app()
            .state
            .as_ref()
            .and_then(|state| state.browser.main_frame());
        input::handle_key_event(&host, frame.as_ref(), &event, focus_on_editable_field);
    }

    #[func]
    pub fn forward_screen_touch_event(
        &mut self,
        event: Gd<InputEventScreenTouch>,
        pixel_scale_factor: f32,
        device_scale_factor: f32,
    ) {
        let Some(host) = self.runtime.app().host() else {
            return;
        };
        input::handle_screen_touch(
            &host,
            &event,
            pixel_scale_factor,
            device_scale_factor,
            &mut self.touch_id_map,
            &mut self.next_touch_id,
        );
    }

    #[func]
    pub fn forward_screen_drag_event(
        &mut self,
        event: Gd<InputEventScreenDrag>,
        pixel_scale_factor: f32,
        device_scale_factor: f32,
    ) {
        let Some(host) = self.runtime.app().host() else {
            return;
        };
        input::handle_screen_drag(
            &host,
            &event,
            pixel_scale_factor,
            device_scale_factor,
            &mut self.touch_id_map,
            &mut self.next_touch_id,
        );
    }

    #[func]
    pub fn forward_input_event(
        &mut self,
        event: Gd<InputEvent>,
        pixel_scale_factor: f32,
        device_scale_factor: f32,
        focus_on_editable_field: bool,
    ) {
        if let Ok(mouse_button) = event.clone().try_cast::<InputEventMouseButton>() {
            self.forward_mouse_button_event(mouse_button, pixel_scale_factor, device_scale_factor);
        } else if let Ok(mouse_motion) = event.clone().try_cast::<InputEventMouseMotion>() {
            self.forward_mouse_motion_event(mouse_motion, pixel_scale_factor, device_scale_factor);
        } else if let Ok(pan_gesture) = event.clone().try_cast::<InputEventPanGesture>() {
            self.forward_pan_gesture_event(pan_gesture, pixel_scale_factor, device_scale_factor);
        } else if let Ok(screen_touch) = event.clone().try_cast::<InputEventScreenTouch>() {
            self.forward_screen_touch_event(screen_touch, pixel_scale_factor, device_scale_factor);
        } else if let Ok(screen_drag) = event.clone().try_cast::<InputEventScreenDrag>() {
            self.forward_screen_drag_event(screen_drag, pixel_scale_factor, device_scale_factor);
        } else if let Ok(magnify_gesture) = event.clone().try_cast::<InputEventMagnifyGesture>() {
            self.forward_magnify_gesture_event(magnify_gesture);
        } else if let Ok(key_event) = event.try_cast::<InputEventKey>() {
            self.forward_key_event(key_event, focus_on_editable_field);
        }
    }

    fn disconnect_frame_hook(&mut self) {
        if !self.frame_hook_connected {
            return;
        }

        if let Some(callable) = self.frame_hook_callable.as_ref() {
            RenderingServer::singleton().disconnect("frame_pre_draw", callable);
        }

        self.frame_hook_callable = None;
        self.frame_hook_connected = false;
    }

    fn get_dpi(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    fn logical_size(&self) -> Vector2 {
        Vector2::new(self.texture_size.x as f32, self.texture_size.y as f32)
    }

    fn refresh_fallback_texture(&mut self) {
        self.fallback_texture = make_placeholder_texture(self.texture_size);
    }

    fn try_create_browser(&mut self) {
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        self.runtime.try_create_browser(RuntimeCreateConfig {
            logical_size,
            dpi,
            url: self.url.clone(),
            enable_accelerated_osr: self.enable_accelerated_osr,
            background_color: self.background_color,
            popup_policy: self.popup_policy,
            software_target_texture: Some(self.fallback_texture.clone()),
            log_prefix: "CefTexture2D",
        });
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if self.enable_accelerated_osr
            && let Some(state) = self.runtime.app_mut().state.as_mut()
            && let RenderMode::Accelerated {
                texture_2d_rd,
                render_state,
                ..
            } = &mut state.render_mode
            && let Some(stable) = &mut self.stable_texture_2d_rd
        {
            let dst_rd_rid = render_state
                .lock()
                .ok()
                .map(|rs| rs.dst_rd_rid)
                .unwrap_or(Rid::Invalid);
            stable.set_texture_rd_rid(dst_rd_rid);
            *texture_2d_rd = stable.clone();
        }
        self.base_mut().emit_changed();
    }

    fn cleanup_instance(&mut self) {
        self.disconnect_frame_hook();
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if let Some(ref mut stable) = self.stable_texture_2d_rd {
                stable.set_texture_rd_rid(Rid::Invalid);
            }
            if self.placeholder_rd_rid.is_valid() {
                render::free_rd_texture(self.placeholder_rd_rid);
                self.placeholder_rd_rid = Rid::Invalid;
            }
        }
        self.runtime.cleanup_runtime(None);
    }

    fn update_texture(&mut self) {
        let Some(state) = &mut self.runtime.app_mut().state else {
            return;
        };

        let software_had_dirty = match &state.render_mode {
            RenderMode::Software { frame_buffer, .. } => {
                let fb_dirty = frame_buffer.lock().ok().is_some_and(|fb| fb.dirty);
                let popup_dirty =
                    state.popup_state.lock().ok().is_some_and(|popup| {
                        popup.visible && popup.dirty && !popup.buffer.is_empty()
                    });
                fb_dirty || popup_dirty
            }
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            RenderMode::Accelerated { .. } => false,
        };

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        let had_pending_copy =
            if let RenderMode::Accelerated { render_state, .. } = &state.render_mode {
                render_state
                    .lock()
                    .ok()
                    .is_some_and(|rs| rs.has_pending_copy)
            } else {
                false
            };
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let had_pending_copy = false;

        let replacement = self.runtime.update_primary_texture("CefTexture2D");
        let has_replacement = replacement.is_some();

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if let Some(new_t2d) = replacement
                && let Some(ref mut stable) = self.stable_texture_2d_rd
            {
                let new_rd_rid = new_t2d.get_texture_rd_rid();
                stable.set_texture_rd_rid(new_rd_rid);
                if let Some(state) = self.runtime.app_mut().state.as_mut()
                    && let RenderMode::Accelerated { texture_2d_rd, .. } = &mut state.render_mode
                {
                    *texture_2d_rd = stable.clone();
                }
            }
        }

        let _ = replacement;
        let should_emit_changed = software_had_dirty || has_replacement || had_pending_copy;
        if should_emit_changed {
            self.base_mut().emit_changed();
        }
    }

    fn drain_event_queues(&self) {
        self.runtime.drain_event_queues("CefTexture2D");
    }

    fn tick(&mut self) {
        if !self.runtime.runtime_enabled() {
            if Engine::singleton().is_editor_hint() {
                return;
            }
            self.runtime.set_runtime_enabled(true);
        }

        self.try_create_browser();

        self.runtime.handle_max_fps_change();
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        let _ = self.runtime.handle_size_change(logical_size, dpi);
        self.update_texture();
        self.runtime.message_loop_and_begin_frame();
        self.drain_event_queues();
    }
}

impl Drop for CefTexture2D {
    fn drop(&mut self) {
        self.cleanup_instance();
    }
}

fn make_placeholder_texture(size: Vector2i) -> Gd<ImageTexture> {
    let width = size.x.max(1);
    let height = size.y.max(1);
    let pixel_count = (width as usize) * (height as usize);
    let bytes = vec![0u8; pixel_count * 4];
    let byte_array = PackedByteArray::from(bytes.as_slice());

    let mut texture = ImageTexture::new_gd();
    if let Some(image) =
        Image::create_from_data(width, height, false, ImageFormat::RGBA8, &byte_array)
    {
        texture.set_image(&image);
    }

    texture
}
