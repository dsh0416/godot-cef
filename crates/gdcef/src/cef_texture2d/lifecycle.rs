use super::*;

impl CefTexture2D {
    pub(super) fn connect_runtime_hooks(&mut self) {
        // CefTexture shuts down its helper during construction and drives that
        // helper itself. Its queued installation must not start a second browser
        // or double-tick the shared runtime.
        if !self.runtime.runtime_enabled()
            || Engine::singleton().is_editor_hint()
            || self.process_hook.is_some()
        {
            return;
        }

        let Some(mut tree) = Engine::singleton()
            .get_main_loop()
            .and_then(|main_loop| main_loop.try_cast::<SceneTree>().ok())
        else {
            godot::global::godot_error!(
                "[CefTexture2D] Browser runtime requires a SceneTree main loop"
            );
            return;
        };

        let process_callable = self.base().callable("_on_process_frame");
        let error = tree.connect("process_frame", &process_callable);
        if error != godot::global::Error::OK {
            godot::global::godot_error!(
                "[CefTexture2D] Failed to connect process frame: {error:?}"
            );
            return;
        }
        self.process_hook = Some((tree.instance_id(), process_callable));

        let frame_callable = self.base().callable("_on_frame_pre_draw");
        let error = RenderingServer::singleton().connect("frame_pre_draw", &frame_callable);
        if error == godot::global::Error::OK {
            self.frame_hook = Some(frame_callable);
        } else {
            godot::global::godot_error!("[CefTexture2D] Failed to connect render frame: {error:?}");
        }
    }

    pub(super) fn disconnect_runtime_hooks(&mut self) {
        if let Some((tree_id, callable)) = self.process_hook.take()
            && let Ok(mut tree) = Gd::<SceneTree>::try_from_instance_id(tree_id)
            && tree.is_connected("process_frame", &callable)
        {
            tree.disconnect("process_frame", &callable);
        }
        if let Some(callable) = self.frame_hook.take() {
            let mut rendering_server = RenderingServer::singleton();
            if rendering_server.is_connected("frame_pre_draw", &callable) {
                rendering_server.disconnect("frame_pre_draw", &callable);
            }
        }
    }

    pub(super) fn get_dpi(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    pub(super) fn logical_size(&self) -> Vector2 {
        Vector2::new(self.texture_size.x as f32, self.texture_size.y as f32)
    }

    pub(super) fn try_create_browser(&mut self) {
        if self.runtime.app().state.is_some() || !self.runtime.app().can_create_browser() {
            return;
        }
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        self.runtime.try_create_browser(RuntimeCreateConfig {
            logical_size,
            dpi,
            url: self.url.clone(),
            enable_accelerated_osr: self.enable_accelerated_osr,
            background_color: self.background_color,
            popup_policy: self.popup_policy,
            permission_policy: self.permission_policy,
            preload_script: self.preload_script.clone(),
            preload_script_path: self.preload_script_path.clone(),
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
            self.popup_compositor = Some(popup_compositor::PopupCompositor::new(stable.get_rid()));
        }
        self.base_mut().emit_changed();
    }

    pub(super) fn cleanup_instance(&mut self) {
        self.runtime.shutdown();
        self.disconnect_runtime_hooks();
        self.cancel_active_touches();
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            if let Some(compositor) = self.popup_compositor.take() {
                compositor.dispose();
            }
            if let Some(ref mut stable) = self.stable_texture_2d_rd {
                stable.set_texture_rd_rid(Rid::Invalid);
            }
        }
        self.runtime.cleanup_runtime(None);
    }

    pub(super) fn drain_event_queues(&mut self) {
        let events = self.runtime.drain_event_queues("CefTexture2D");
        for (request_id, result) in events.permission_request_finished {
            self.base_mut().emit_signal(
                "permission_request_finished",
                &[request_id.to_variant(), GString::from(&result).to_variant()],
            );
        }
        for event in events.permission_requests {
            if !self.is_permission_pending(event.request_id) {
                continue;
            }
            if self
                .base()
                .get_signal_connection_list("permission_requested")
                .is_empty()
            {
                if let Some(state) = self.runtime.app().state.as_ref() {
                    state.permissions.dismiss_request(event.request_id);
                }
                continue;
            }
            self.base_mut().emit_signal(
                "permission_requested",
                &[
                    GString::from(&event.permission_type).to_variant(),
                    GString::from(&event.url).to_variant(),
                    event.request_id.to_variant(),
                ],
            );
        }
    }

    pub(super) fn tick(&mut self) {
        if !self.runtime.runtime_enabled() || Engine::singleton().is_editor_hint() {
            return;
        }

        self.try_create_browser();

        self.runtime.handle_max_fps_change();
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        let _ = self.runtime.handle_size_change(logical_size, dpi);
        // Headless Godot does not emit frame_pre_draw. Browser lifecycle and
        // callbacks must also progress without drawing when minimized or with
        // the render loop disabled. Software frames can be produced here;
        // accelerated begin requests follow publication on frame_pre_draw.
        if self
            .runtime
            .app()
            .state
            .as_ref()
            .is_some_and(|state| matches!(state.render_mode, RenderMode::Software { .. }))
        {
            self.runtime.request_external_begin_frame();
        }
        self.drain_event_queues();
    }
}

impl Drop for CefTexture2D {
    fn drop(&mut self) {
        self.cleanup_instance();
    }
}
