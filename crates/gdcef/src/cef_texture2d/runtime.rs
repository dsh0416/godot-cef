use super::*;

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
