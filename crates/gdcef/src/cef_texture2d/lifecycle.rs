use super::*;

impl CefTexture2D {
    pub(super) fn disconnect_frame_hook(&mut self) {
        if !self.frame_hook_connected {
            return;
        }

        if let Some(callable) = self.frame_hook_callable.as_ref() {
            RenderingServer::singleton().disconnect("frame_pre_draw", callable);
        }

        self.frame_hook_callable = None;
        self.frame_hook_connected = false;
    }

    pub(super) fn get_dpi(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    pub(super) fn logical_size(&self) -> Vector2 {
        Vector2::new(self.texture_size.x as f32, self.texture_size.y as f32)
    }

    pub(super) fn refresh_fallback_texture(&mut self) {
        self.fallback_texture = Self::make_placeholder_texture(self.texture_size);
    }

    pub(super) fn try_create_browser(&mut self) {
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

    pub(super) fn cleanup_instance(&mut self) {
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

    pub(super) fn drain_event_queues(&self) {
        self.runtime.drain_event_queues("CefTexture2D");
    }

    pub(super) fn tick(&mut self) {
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
