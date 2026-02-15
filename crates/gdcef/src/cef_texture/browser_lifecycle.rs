use super::{CefTexture, backend};
use crate::error::CefError;
use godot::prelude::*;

impl CefTexture {
    fn log_cleanup_state_violations(&self) {
        if self.app.state.is_some() {
            godot::global::godot_warn!(
                "[CefTexture] Cleanup invariant violation: runtime state not fully cleared"
            );
        }
    }

    pub(super) fn cleanup_instance(&mut self) {
        self.base_mut().set_visible(false);
        backend::cleanup_runtime(&mut self.app, self.popup_texture_2d_rd.as_mut());

        self.ime_active = false;
        self.ime_proxy = None;
        self.last_find_query = GString::new();
        self.last_find_match_case = false;

        if let Some(mut overlay) = self.popup_overlay.take() {
            overlay.queue_free();
        }
        self.popup_texture = None;

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            self.popup_texture_2d_rd = None;
        }

        self.log_cleanup_state_violations();
    }

    pub(super) fn create_browser(&mut self) {
        if let Err(e) = self.try_create_browser() {
            godot::global::godot_error!("[CefTexture] {}", e);
        }
    }

    pub(super) fn try_create_browser(&mut self) -> Result<(), CefError> {
        if self.app.state.is_some() {
            return Ok(());
        }

        let logical_size = self.base().get_size();
        let dpi = self.get_pixel_scale_factor();
        let params = backend::BackendCreateParams {
            logical_size,
            dpi,
            max_fps: self.get_max_fps(),
            url: self.url.to_string(),
            enable_accelerated_osr: self.enable_accelerated_osr,
            background_color: self.background_color,
            popup_policy: self.popup_policy,
            log_prefix: "CefTexture",
        };
        backend::try_create_browser(&mut self.app, &params)?;
        if let Some(state) = self.app.state.as_ref() {
            let texture = state.render_mode.texture_2d();
            self.base_mut().set_texture(&texture);
        }

        self.last_size = logical_size;
        self.last_dpi = dpi;
        Ok(())
    }
}
