use super::CefTexture;
use crate::browser::LifecycleState;
use crate::cef_texture::backend;
use crate::error::CefError;
use godot::prelude::*;

impl CefTexture {
    fn log_cleanup_state_violations(&self) {
        if self.with_app(|app| app.state.is_some()) {
            godot::global::godot_warn!(
                "[CefTexture] Cleanup invariant violation: runtime state not fully cleared"
            );
        }
        let lifecycle_state = self.with_app(|app| app.lifecycle_state());
        if !matches!(
            lifecycle_state,
            LifecycleState::Closed | LifecycleState::Retained
        ) {
            godot::global::godot_warn!(
                "[CefTexture] Cleanup invariant violation: unexpected lifecycle state {:?}",
                lifecycle_state
            );
        }
    }

    pub(super) fn cleanup_instance(&mut self) {
        self.base_mut().set_visible(false);
        let mut popup_texture_2d_rd = self.popup_texture_2d_rd.take();
        self.with_app_mut(|app| backend::cleanup_runtime(app, popup_texture_2d_rd.as_mut()));
        self.popup_texture_2d_rd = popup_texture_2d_rd;

        self.ime_active = false;
        self.ime_proxy = None;

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
        if self.with_app(|app| app.state.is_some()) {
            return Ok(());
        }
        if !self.with_app_mut(|app| app.begin_browser_create()) {
            return Ok(());
        }

        let logical_size = self.base().get_size();
        let dpi = self.get_pixel_scale_factor();
        let max_fps = self.get_max_fps();
        let params = backend::BackendCreateParams {
            logical_size,
            dpi,
            max_fps,
            url: self.url.to_string(),
            enable_accelerated_osr: self.enable_accelerated_osr,
            background_color: self.background_color,
            popup_policy: self.popup_policy,
            software_target_texture: None,
            log_prefix: "CefTexture",
        };
        if let Err(err) = self.with_app_mut(|app| backend::try_create_browser(app, &params)) {
            self.with_app_mut(|app| app.mark_browser_closed());
            return Err(err);
        }
        self.with_app_mut(|app| app.mark_browser_running());
        if let Some(texture) =
            self.with_app(|app| app.state.as_ref().map(|s| s.render_mode.texture_2d()))
        {
            self.base_mut().set_texture(&texture);
        }

        self.last_size = logical_size;
        self.last_dpi = dpi;
        Ok(())
    }
}
