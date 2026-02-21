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
            godot::global::godot_warn!("[CefTexture2D] Cannot send binary IPC message: no main frame");
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
        host.find(Some(&query_cef), forward as _, match_case as _, false as _);
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
            true as _,
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
            true as _,
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
}
