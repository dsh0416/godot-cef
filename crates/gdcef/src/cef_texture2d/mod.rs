use cef::{ImplBrowser, ImplFrame};
use godot::classes::{ITexture2D, RenderingServer, Texture2D};
use godot::prelude::*;

use crate::browser::{App, RenderMode};
use crate::cef_init;
use crate::cef_texture::backend;

#[derive(GodotClass)]
#[class(base=Texture2D)]
pub struct CefTexture2D {
    base: Base<Texture2D>,
    app: App,

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

    last_size: Vector2,
    last_dpi: f32,
    last_max_fps: i32,
    frame_hook_connected: bool,
    runtime_enabled: bool,
}

#[godot_api]
impl ITexture2D for CefTexture2D {
    fn init(base: Base<Texture2D>) -> Self {
        let mut texture = Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
            background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
            popup_policy: crate::browser::popup_policy::BLOCK,
            texture_size: Vector2i::new(1024, 1024),
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_max_fps: 0,
            frame_hook_connected: false,
            runtime_enabled: true,
        };
        texture.ensure_frame_hook();
        texture
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
        let Some(state) = self.app.state.as_ref() else {
            return Rid::Invalid;
        };

        match &state.render_mode {
            RenderMode::Software { texture, .. } => texture.get_rid(),
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            RenderMode::Accelerated { texture_2d_rd, .. } => texture_2d_rd.get_rid(),
        }
    }
}

#[godot_api]
impl CefTexture2D {
    #[func]
    fn _on_frame_pre_draw(&mut self) {
        self.tick();
    }

    #[func]
    fn set_url_property(&mut self, url: GString) {
        self.url = url.clone();
        if let Some(state) = self.app.state.as_ref()
            && let Some(frame) = state.browser.main_frame()
        {
            let url_str: cef::CefStringUtf16 = url.to_string().as_str().into();
            frame.load_url(Some(&url_str));
        }
    }

    #[func]
    fn get_url_property(&self) -> GString {
        if let Some(state) = self.app.state.as_ref()
            && let Some(frame) = state.browser.main_frame()
        {
            let frame_url = frame.url();
            let url_string = cef::CefStringUtf16::from(&frame_url).to_string();
            return GString::from(url_string.as_str());
        }
        self.url.clone()
    }

    #[func]
    fn get_popup_policy(&self) -> i32 {
        self.popup_policy
    }

    #[func]
    fn set_popup_policy(&mut self, policy: i32) {
        self.popup_policy = policy;
        backend::apply_popup_policy(&self.app, policy);
    }

    #[func]
    fn get_texture_size_property(&self) -> Vector2i {
        self.texture_size
    }

    #[func]
    fn set_texture_size_property(&mut self, size: Vector2i) {
        let clamped = Vector2i::new(size.x.max(1), size.y.max(1));
        if clamped == self.texture_size {
            return;
        }

        self.texture_size = clamped;

        // Notify the backend that the browser size has changed so it can
        // resize the off-screen rendering accordingly.
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        backend::handle_size_change(
            &self.app,
            &mut self.last_size,
            &mut self.last_dpi,
            logical_size,
            dpi,
        );
    }

    #[func]
    pub fn shutdown(&mut self) {
        self.runtime_enabled = false;
        self.cleanup_instance();
    }

    fn ensure_frame_hook(&mut self) {
        if self.frame_hook_connected {
            return;
        }
        let callable = self.base().callable("_on_frame_pre_draw");
        RenderingServer::singleton().connect("frame_pre_draw", &callable);
        self.frame_hook_connected = true;
    }

    fn disconnect_frame_hook(&mut self) {
        if !self.frame_hook_connected {
            return;
        }
        let callable = self.base().callable("_on_frame_pre_draw");
        RenderingServer::singleton().disconnect("frame_pre_draw", &callable);
        self.frame_hook_connected = false;
    }

    fn get_max_fps(&self) -> i32 {
        backend::get_max_fps()
    }

    fn get_dpi(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    fn logical_size(&self) -> Vector2 {
        Vector2::new(self.texture_size.x as f32, self.texture_size.y as f32)
    }

    fn try_create_browser(&mut self) {
        if !self.runtime_enabled || self.app.state.is_some() {
            return;
        }
        if let Err(e) = cef_init::cef_retain() {
            godot::global::godot_error!("[CefTexture2D] {}", e);
            return;
        }
        self.app.mark_cef_retained();
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        let params = backend::BackendCreateParams {
            logical_size,
            dpi,
            max_fps: self.get_max_fps(),
            url: self.url.to_string(),
            enable_accelerated_osr: self.enable_accelerated_osr,
            background_color: self.background_color,
            popup_policy: self.popup_policy,
            log_prefix: "CefTexture2D",
        };
        if let Err(e) = backend::try_create_browser(&mut self.app, &params) {
            godot::global::godot_error!("[CefTexture2D] {}", e);
            self.app.release_cef_if_retained();
            return;
        }
        self.last_size = logical_size;
        self.last_dpi = dpi;
    }

    fn cleanup_instance(&mut self) {
        self.disconnect_frame_hook();
        backend::cleanup_runtime(&mut self.app, None);
    }

    fn update_texture(&mut self) {
        let Some(state) = &mut self.app.state else {
            return;
        };
        let _ = backend::update_primary_texture(state, "CefTexture2D");
    }

    fn drain_event_queues(&self) {
        let Some(event_queues) = self.app.state.as_ref().map(|state| &state.event_queues) else {
            return;
        };

        let Ok(mut queues) = event_queues.lock() else {
            godot::global::godot_warn!(
                "[CefTexture2D] Failed to lock event queues while draining events"
            );
            return;
        };

        // `CefTexture2D` is intentionally render-only and does not expose these
        // events to script; drain every frame to avoid unbounded queue growth.
        let _ = std::mem::take(&mut *queues);
    }

    fn tick(&mut self) {
        if !self.runtime_enabled {
            // Defensive: if the frame callback was not disconnected for any reason,
            // prevent re-creating runtime state after an explicit shutdown().
            self.disconnect_frame_hook();
            return;
        }

        self.try_create_browser();

        let max_fps = self.get_max_fps();
        backend::handle_max_fps_change(&self.app, &mut self.last_max_fps, max_fps);
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        let _ = backend::handle_size_change(
            &self.app,
            &mut self.last_size,
            &mut self.last_dpi,
            logical_size,
            dpi,
        );
        self.update_texture();
        if self.app.state.is_some() {
            cef::do_message_loop_work();
        }
        backend::request_external_begin_frame(&self.app);
        self.drain_event_queues();
    }
}

impl Drop for CefTexture2D {
    fn drop(&mut self) {
        self.cleanup_instance();
    }
}
