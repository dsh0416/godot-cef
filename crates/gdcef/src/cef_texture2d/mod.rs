use cef::{ImplBrowser, ImplFrame};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ObjectNotification;
use godot::classes::{Engine, ITexture2D, Image, ImageTexture, RenderingServer, Texture2D};
use godot::prelude::*;

use crate::browser::{App, RenderMode};
use crate::cef_init;
use crate::cef_texture::backend;
use crate::render;

#[derive(GodotClass)]
#[class(base=Texture2D, tool)]
pub struct CefTexture2D {
    base: Base<Texture2D>,
    app: App,
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

    last_size: Vector2,
    last_dpi: f32,
    last_max_fps: i32,
    frame_hook_callable: Option<Callable>,
    frame_hook_connected: bool,
    runtime_enabled: bool,
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
            app: App::default(),
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
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_max_fps: 0,
            frame_hook_callable: Some(frame_hook_callable),
            frame_hook_connected: true,
            runtime_enabled: !editor_hint,
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
        self.refresh_fallback_texture();

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
        self.base_mut().emit_changed();
    }

    #[func]
    pub fn shutdown(&mut self) {
        self.runtime_enabled = false;
        self.cleanup_instance();
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

    fn get_max_fps(&self) -> i32 {
        backend::get_max_fps()
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
            software_target_texture: Some(self.fallback_texture.clone()),
            log_prefix: "CefTexture2D",
        };
        if let Err(e) = backend::try_create_browser(&mut self.app, &params) {
            godot::global::godot_error!("[CefTexture2D] {}", e);
            self.app.release_cef_if_retained();
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if self.enable_accelerated_osr
            && let Some(state) = self.app.state.as_mut()
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
        self.last_size = logical_size;
        self.last_dpi = dpi;
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
        backend::cleanup_runtime(&mut self.app, None);
    }

    fn update_texture(&mut self) {
        let Some(state) = &mut self.app.state else {
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

        let replacement = backend::update_primary_texture(state, "CefTexture2D");
        let has_replacement = replacement.is_some();

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if let Some(new_t2d) = replacement
                && let Some(ref mut stable) = self.stable_texture_2d_rd
            {
                let new_rd_rid = new_t2d.get_texture_rd_rid();
                stable.set_texture_rd_rid(new_rd_rid);
                if let Some(state) = self.app.state.as_mut()
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
        let Some(event_queues) = self.app.state.as_ref().map(|state| &state.event_queues) else {
            return;
        };

        let Ok(mut queues) = event_queues.lock() else {
            godot::global::godot_warn!(
                "[CefTexture2D] Failed to lock event queues while draining events"
            );
            return;
        };

        let _ = std::mem::take(&mut *queues);
    }

    fn tick(&mut self) {
        if !self.runtime_enabled {
            if Engine::singleton().is_editor_hint() {
                return;
            }
            self.runtime_enabled = true;
        }

        if Engine::singleton().is_editor_hint() {
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
