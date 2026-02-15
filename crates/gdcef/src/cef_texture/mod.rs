pub(crate) mod backend;
mod browser_lifecycle;
mod ime;
mod rendering;
mod signals;

use cef::{
    self, ImplBrowser, ImplBrowserHost, ImplDragData, ImplFrame, ImplListValue,
    ImplMediaAccessCallback, ImplPermissionPromptCallback, ImplProcessMessage,
    do_message_loop_work,
};
use godot::classes::notify::ControlNotification;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{
    ITextureRect, ImageTexture, InputEvent, InputEventKey, InputEventMagnifyGesture,
    InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture, InputEventScreenDrag,
    InputEventScreenTouch, LineEdit, TextureRect,
};
use godot::prelude::*;
use std::collections::HashMap;

use crate::browser::App;
use crate::{cef_init, input};

#[derive(GodotClass)]
#[class(base=TextureRect)]
pub struct CefTexture {
    base: Base<TextureRect>,
    app: App,

    #[export]
    #[var(get = get_url_property, set = set_url_property)]
    /// The URL to load. Changing this triggers a navigation.
    /// Supported schemes: http://, https://, res://, user://
    url: GString,

    #[export]
    /// Enable GPU-accelerated Off-Screen Rendering (OSR).
    /// If true, uses shared textures (Vulkan/D3D12/Metal) for high performance.
    /// If false or unsupported, falls back to software rendering.
    enable_accelerated_osr: bool,

    #[export]
    /// The background color of the browser view.
    /// Useful for transparent pages.
    background_color: Color,

    #[export(enum = (Block = 0, Redirect = 1, SignalOnly = 2))]
    #[var(get = get_popup_policy, set = set_popup_policy)]
    /// Controls how popup windows (window.open, target="_blank") are handled.
    /// Block: suppress all popups silently (default).
    /// Redirect: navigate the current browser to the popup URL.
    /// SignalOnly: emit `popup_requested` signal and let GDScript decide.
    popup_policy: i32,

    #[var]
    /// Stores the IME cursor position in local coordinates (relative to this `CefTexture` node),
    /// automatically updated from the browser's caret position.
    ime_position: Vector2i,

    // Change detection state
    last_size: Vector2,
    last_dpi: f32,
    last_cursor: cef_app::CursorType,
    last_max_fps: i32,
    browser_create_deferred_pending: bool,

    // IME state
    ime_active: bool,
    ime_proxy: Option<Gd<LineEdit>>,
    ime_focus_regrab_pending: bool,

    // Popup state
    popup_overlay: Option<Gd<TextureRect>>,
    popup_texture: Option<Gd<ImageTexture>>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    popup_texture_2d_rd: Option<Gd<godot::classes::Texture2Drd>>,

    // Touch state
    touch_id_map: HashMap<i32, i32>,
    next_touch_id: i32,

    // Find-in-page state
    last_find_query: GString,
    last_find_match_case: bool,
}

#[godot_api]
impl ITextureRect for CefTexture {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
            background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
            popup_policy: crate::browser::popup_policy::BLOCK,
            ime_position: Vector2i::new(0, 0),
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_cursor: cef_app::CursorType::Arrow,
            last_max_fps: 0,
            browser_create_deferred_pending: false,
            ime_active: false,
            ime_proxy: None,
            ime_focus_regrab_pending: false,
            popup_overlay: None,
            popup_texture: None,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            popup_texture_2d_rd: None,
            touch_id_map: HashMap::new(),
            next_touch_id: 0,
            last_find_query: GString::new(),
            last_find_match_case: false,
        }
    }

    fn on_notification(&mut self, what: ControlNotification) {
        match what {
            ControlNotification::READY => {
                self.on_ready();
            }
            ControlNotification::PROCESS => {
                self.on_process();
            }
            ControlNotification::PREDELETE => {
                self.cleanup_instance();
            }
            ControlNotification::FOCUS_ENTER => {
                if let Some(host) = self.app.host() {
                    host.set_focus(true as _);
                }
            }
            ControlNotification::FOCUS_EXIT => {
                if let Some(host) = self.app.host() {
                    host.set_focus(false as _);
                }
            }
            ControlNotification::OS_IME_UPDATE => {
                self.handle_os_ime_update();
            }
            _ => {}
        }
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        self.handle_input_event(event);
    }
}

#[godot_api]
impl CefTexture {
    #[signal]
    fn ipc_message(message: GString);

    #[signal]
    fn ipc_binary_message(data: PackedByteArray);

    #[signal]
    fn ipc_data_message(data: Variant);

    #[signal]
    fn url_changed(url: GString);

    #[signal]
    fn title_changed(title: GString);

    #[signal]
    fn load_started(url: GString);

    #[signal]
    fn load_finished(url: GString, http_status_code: i32);

    #[signal]
    fn load_error(url: GString, error_code: i32, error_text: GString);

    #[signal]
    fn console_message(level: u32, message: GString, source: GString, line: i32);

    #[signal]
    fn drag_started(drag_data: Gd<crate::drag::DragDataInfo>, position: Vector2, allowed_ops: i32);

    #[signal]
    fn drag_cursor_updated(operation: i32);

    #[signal]
    fn drag_entered(drag_data: Gd<crate::drag::DragDataInfo>, mask: i32);

    #[signal]
    fn download_requested(download_info: Gd<crate::cef_texture::signals::DownloadRequestInfo>);

    #[signal]
    fn download_updated(download_info: Gd<crate::cef_texture::signals::DownloadUpdateInfo>);

    #[signal]
    fn render_process_terminated(status: i32, error_message: GString);

    #[signal]
    fn popup_requested(url: GString, disposition: i32, user_gesture: bool);

    #[signal]
    fn permission_requested(permission_type: GString, url: GString, request_id: i64);

    /// Emitted after a find-in-page operation completes or is updated.
    ///
    /// - `count` is the total number of matches found.
    /// - `active_index` corresponds to CEF's `active_match_ordinal` and is **1-based**.
    ///   A value of `0` means there is no active match.
    #[signal]
    fn find_result(count: i32, active_index: i32, final_update: bool);

    /// Emitted when `get_cookies` or `get_all_cookies` completes.
    /// Contains an `Array` of `CookieInfo` objects.
    #[signal]
    fn cookies_received(cookies: Array<Gd<crate::cef_texture::signals::CookieInfo>>);

    /// Emitted when `set_cookie` completes.
    #[signal]
    fn cookie_set(success: bool);

    /// Emitted when `delete_cookies` or `clear_cookies` completes.
    #[signal]
    fn cookies_deleted(num_deleted: i32);

    /// Emitted when `flush_cookies` completes.
    #[signal]
    fn cookies_flushed();

    #[func]
    fn on_ready(&mut self) {
        use godot::classes::control::FocusMode;
        if let Err(e) = cef_init::cef_retain() {
            godot::global::godot_error!("[CefTexture] {}", e);
            return;
        }
        self.app.mark_cef_retained();

        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);
        // Must explicitly enable processing when using on_notification instead of fn process()
        self.base_mut().set_process(true);
        // Enable focus so we receive FOCUS_ENTER/EXIT notifications and can forward to CEF.
        self.base_mut().set_focus_mode(FocusMode::CLICK);

        // Create hidden LineEdit for IME proxy
        self.create_ime_proxy();

        // Never create the browser inside READY notification. CEF context/browser
        // creation can synchronously trigger callbacks, causing re-entrant mutable
        // borrows while Godot is still inside `on_notification`.
        // Browser creation is handled in `on_process()` once notification returns.
    }

    #[func]
    fn on_process(&mut self) {
        // Lazy browser creation: if browser doesn't exist yet (e.g., size was 0 in on_ready
        // because we're inside a Container), try to create it now that layout may be complete.
        if self.app.state.is_none() {
            let size = self.base().get_size();
            if size.x > 0.0 && size.y > 0.0 && !self.browser_create_deferred_pending {
                self.browser_create_deferred_pending = true;
                self.base_mut().set_process(false);
                self.base_mut()
                    .call_deferred("_deferred_create_browser", &[]);
            }
        }

        self.handle_max_fps_change();
        _ = self.handle_size_change();
        self.update_texture();

        if self.app.state.is_some() {
            do_message_loop_work();
        }

        self.request_external_begin_frame();
        self.update_cursor();

        // Process all event queues with a single lock (more efficient than per-queue locks)
        self.process_all_event_queues();
    }

    #[func]
    fn _deferred_create_browser(&mut self) {
        self.browser_create_deferred_pending = false;
        if self.app.state.is_none() {
            self.create_browser();
        }
        self.base_mut().set_process(true);
    }

    fn handle_input_event(&mut self, event: Gd<InputEvent>) {
        let Some(state) = self.app.state.as_mut() else {
            return;
        };
        let Some(host) = state.browser.host() else {
            return;
        };

        if let Ok(mouse_button) = event.clone().try_cast::<InputEventMouseButton>() {
            input::handle_mouse_button(
                &host,
                &mouse_button,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
            );
        } else if let Ok(mouse_motion) = event.clone().try_cast::<InputEventMouseMotion>() {
            input::handle_mouse_motion(
                &host,
                &mouse_motion,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
            );
        } else if let Ok(pan_gesture) = event.clone().try_cast::<InputEventPanGesture>() {
            input::handle_pan_gesture(
                &host,
                &pan_gesture,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
            );
        } else if let Ok(screen_touch) = event.clone().try_cast::<InputEventScreenTouch>() {
            input::handle_screen_touch(
                &host,
                &screen_touch,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
                &mut self.touch_id_map,
                &mut self.next_touch_id,
            );
        } else if let Ok(screen_drag) = event.clone().try_cast::<InputEventScreenDrag>() {
            input::handle_screen_drag(
                &host,
                &screen_drag,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
                &mut self.touch_id_map,
                &mut self.next_touch_id,
            );
        } else if let Ok(magnify_gesture) = event.clone().try_cast::<InputEventMagnifyGesture>() {
            input::handle_magnify_gesture(&host, &magnify_gesture);
        } else if let Ok(key_event) = event.try_cast::<InputEventKey>() {
            input::handle_key_event(
                &host,
                state.browser.main_frame().as_ref(),
                &key_event,
                self.ime_active,
            );
        }
    }

    #[func]
    /// Executes JavaScript code in the browser's main frame.
    /// This is a fire-and-forget operation.
    pub fn eval(&mut self, code: GString) {
        let Some(state) = self.app.state.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot execute JS: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot execute JS: no main frame");
            return;
        };

        let code_str: cef::CefStringUtf16 = code.to_string().as_str().into();
        frame.execute_java_script(Some(&code_str), None, 0);
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
    /// Sends a message into the page via `window.onIpcMessage`.
    ///
    /// This is intentionally separate from [`eval`]: callers could achieve a
    /// similar effect with `eval("window.onIpcMessage(...);")`, but this
    /// helper enforces a consistent IPC pattern (`window.onIpcMessage(message)`).
    ///
    /// Uses native CEF process messaging for efficient transfer without
    /// script injection overhead.
    ///
    /// Use this when you want structured IPC into the page, and `eval` when
    /// you truly need arbitrary JavaScript execution.
    pub fn send_ipc_message(&mut self, message: GString) {
        let Some(state) = self.app.state.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC message: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC message: no main frame");
            return;
        };

        let route = cef::CefStringUtf16::from("ipcGodotToRenderer");
        let msg_str: cef::CefStringUtf16 = message.to_string().as_str().into();

        if let Some(mut process_message) = cef::process_message_create(Some(&route)) {
            if let Some(argument_list) = process_message.argument_list() {
                argument_list.set_string(0, Some(&msg_str));
            }
            frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut process_message));
        }
    }

    #[func]
    /// Sends binary data into the page via `window.onIpcBinaryMessage`.
    ///
    /// The data will be delivered as an ArrayBuffer to the JavaScript callback
    /// `window.onIpcBinaryMessage(arrayBuffer)` if it is registered.
    ///
    /// Uses native CEF process messaging with BinaryValue for zero-copy
    /// binary transfer without encoding overhead.
    pub fn send_ipc_binary_message(&mut self, data: PackedByteArray) {
        let Some(state) = self.app.state.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot send binary IPC message: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send binary IPC message: no main frame"
            );
            return;
        };

        let route = cef::CefStringUtf16::from("ipcBinaryGodotToRenderer");
        let bytes = data.to_vec();

        let Some(mut binary_value) = cef::binary_value_create(Some(&bytes)) else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send binary IPC message: failed to create BinaryValue"
            );
            return;
        };

        let Some(mut process_message) = cef::process_message_create(Some(&route)) else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send binary IPC message: failed to create process message"
            );
            return;
        };

        let Some(argument_list) = process_message.argument_list() else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send binary IPC message: failed to get argument list"
            );
            return;
        };

        argument_list.set_binary(0, Some(&mut binary_value));
        frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut process_message));
    }

    #[func]
    /// Sends typed data into the page via the CBOR-based IPC lane.
    ///
    /// Supported inputs include primitive types, arrays, dictionaries and
    /// packed byte arrays. Unsupported Godot-specific types are tagged as
    /// metadata maps to keep transport failure-safe.
    pub fn send_ipc_data(&mut self, data: Variant) {
        let Some(state) = self.app.state.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC data: no browser");
            return;
        };
        let Some(frame) = state.browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC data: no main frame");
            return;
        };

        let bytes = match crate::ipc_data::encode_variant_to_cbor_bytes(&data) {
            Ok(bytes) => bytes,
            Err(err) => {
                godot::global::godot_warn!("[CefTexture] Cannot encode IPC data: {}", err);
                return;
            }
        };

        if bytes.len() > crate::ipc_data::max_ipc_data_bytes() {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send IPC data: payload too large ({} bytes)",
                bytes.len()
            );
            return;
        }

        let route = cef::CefStringUtf16::from("ipcDataGodotToRenderer");
        let Some(mut binary_value) = cef::binary_value_create(Some(&bytes)) else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send IPC data: failed to create BinaryValue"
            );
            return;
        };
        let Some(mut process_message) = cef::process_message_create(Some(&route)) else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send IPC data: failed to create process message"
            );
            return;
        };
        let Some(argument_list) = process_message.argument_list() else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot send IPC data: failed to get argument list"
            );
            return;
        };

        argument_list.set_binary(0, Some(&mut binary_value));
        frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut process_message));
    }

    #[func]
    /// Navigates back in the browser history.
    pub fn go_back(&mut self) {
        if let Some(browser) = self.app.browser_mut() {
            browser.go_back();
        }
    }

    #[func]
    /// Navigates forward in the browser history.
    pub fn go_forward(&mut self) {
        if let Some(browser) = self.app.browser_mut() {
            browser.go_forward();
        }
    }

    #[func]
    pub fn can_go_back(&self) -> bool {
        self.app
            .browser()
            .map(|b| b.can_go_back() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn can_go_forward(&self) -> bool {
        self.app
            .browser()
            .map(|b| b.can_go_forward() != 0)
            .unwrap_or(false)
    }

    #[func]
    /// Reloads the current page.
    pub fn reload(&mut self) {
        if let Some(browser) = self.app.browser_mut() {
            browser.reload();
        }
    }

    #[func]
    /// Reloads the current page, ignoring cached content.
    pub fn reload_ignore_cache(&mut self) {
        if let Some(browser) = self.app.browser_mut() {
            browser.reload_ignore_cache();
        }
    }

    #[func]
    /// Stops the current page load.
    pub fn stop_loading(&mut self) {
        if let Some(browser) = self.app.browser_mut() {
            browser.stop_load();
        }
    }

    #[func]
    /// Starts a new find-in-page search.
    pub fn find_text(&mut self, query: GString, forward: bool, match_case: bool) {
        let Some(host) = self.app.host() else {
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
    /// Jumps to the next result for the last find query.
    pub fn find_next(&mut self) {
        let Some(host) = self.app.host() else {
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
    /// Jumps to the previous result for the last find query.
    pub fn find_previous(&mut self) {
        let Some(host) = self.app.host() else {
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
    /// Stops active find-in-page highlighting and clears selection.
    pub fn stop_finding(&mut self) {
        if let Some(host) = self.app.host() {
            host.stop_finding(true as _);
        }
        self.last_find_query = GString::new();
        self.last_find_match_case = false;
    }

    #[func]
    /// Returns true if the browser is currently loading a page.
    pub fn is_loading(&self) -> bool {
        self.app
            .browser()
            .map(|b| b.is_loading() != 0)
            .unwrap_or(false)
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
    /// Sets the zoom level. 0.0 is 100%.
    /// Positive values zoom in, negative values zoom out.
    pub fn set_zoom_level(&mut self, level: f64) {
        if let Some(host) = self.app.host() {
            host.set_zoom_level(level);
        }
    }

    #[func]
    /// Returns the current zoom level.
    pub fn get_zoom_level(&self) -> f64 {
        self.app.host().map(|h| h.zoom_level()).unwrap_or(0.0)
    }

    #[func]
    /// Mutes or unmutes audio from this browser instance.
    pub fn set_audio_muted(&mut self, muted: bool) {
        if let Some(host) = self.app.host() {
            host.set_audio_muted(muted as i32);
        }
    }

    #[func]
    /// Returns true if audio is currently muted.
    pub fn is_audio_muted(&self) -> bool {
        self.app
            .host()
            .map(|h| h.is_audio_muted() != 0)
            .unwrap_or(false)
    }

    /// Creates an AudioStreamGenerator configured for this browser's audio.
    /// Only works when `godot_cef/audio/enable_audio_capture` is enabled.
    #[func]
    pub fn create_audio_stream(&self) -> Gd<godot::classes::AudioStreamGenerator> {
        use godot::classes::AudioStreamGenerator;

        let mut stream = AudioStreamGenerator::new_gd();

        let sample_rate = self
            .app
            .state
            .as_ref()
            .and_then(|s| s.audio.as_ref())
            .and_then(|a| a.sample_rate.lock().ok().map(|sr| *sr))
            .unwrap_or(48000.0);

        stream.set_mix_rate(sample_rate);
        stream.set_buffer_length(0.1);

        stream
    }

    /// Pushes buffered audio data to the given playback. Call every frame.
    /// Returns the number of frames pushed.
    #[func]
    pub fn push_audio_to_playback(
        &mut self,
        mut playback: Gd<godot::classes::AudioStreamGeneratorPlayback>,
    ) -> i32 {
        let Some(queue) = self
            .app
            .state
            .as_ref()
            .and_then(|s| s.audio.as_ref())
            .map(|a| &a.packet_queue)
        else {
            return 0;
        };

        let mut total_frames = 0i32;

        if let Ok(mut queue) = queue.lock() {
            'outer: while let Some(mut packet) = queue.pop_front() {
                let mut frame_index = 0;
                let frame_count = packet.data.len() / 2;

                while frame_index < frame_count {
                    if playback.can_push_buffer(1) {
                        let i = frame_index * 2;
                        let frame = Vector2::new(packet.data[i], packet.data[i + 1]);
                        playback.push_frame(frame);
                        total_frames += 1;
                        frame_index += 1;
                    } else {
                        // Playback buffer is full. Re-queue remaining data in this packet
                        // at the front of the queue so it can be processed next frame.
                        if frame_index < frame_count {
                            let samples_consumed = frame_index * 2;
                            packet.data.drain(..samples_consumed);
                            queue.push_front(packet);
                        }
                        break 'outer;
                    }
                }
            }
        }

        total_frames
    }

    /// Returns true if there is audio data available in the buffer.
    #[func]
    pub fn has_audio_data(&self) -> bool {
        self.app
            .state
            .as_ref()
            .and_then(|s| s.audio.as_ref())
            .and_then(|a| a.packet_queue.lock().ok())
            .is_some_and(|q| !q.is_empty())
    }

    /// Returns the number of audio packets currently buffered.
    #[func]
    pub fn get_audio_buffer_size(&self) -> i32 {
        self.app
            .state
            .as_ref()
            .and_then(|s| s.audio.as_ref())
            .and_then(|a| a.packet_queue.lock().ok())
            .map(|q| q.len() as i32)
            .unwrap_or(0)
    }

    /// Returns true if audio capture mode is enabled in project settings.
    #[func]
    pub fn is_audio_capture_enabled(&self) -> bool {
        crate::settings::is_audio_capture_enabled()
    }

    /// Called when the IME proxy LineEdit text changes during composition.
    #[func]
    fn on_ime_proxy_text_changed(&mut self, new_text: GString) {
        self.on_ime_proxy_text_changed_impl(new_text);
    }

    #[func]
    fn on_ime_proxy_focus_exited(&mut self) {
        self.on_ime_proxy_focus_exited_impl();
    }

    #[func]
    fn _check_ime_focus_after_exit(&mut self) {
        self.check_ime_focus_after_exit_impl();
    }

    fn get_pixel_scale_factor(&self) -> f32 {
        self.base()
            .get_viewport()
            .map(|viewport| viewport.get_stretch_transform().a.x)
            .unwrap_or(1.0)
    }

    fn get_device_scale_factor(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    #[func]
    pub fn drag_enter(&mut self, file_paths: Array<GString>, position: Vector2, allowed_ops: i32) {
        let Some(host) = self.app.host() else {
            return;
        };

        let Some(mut drag_data) = cef::drag_data_create() else {
            return;
        };

        for path in file_paths.iter_shared() {
            let path_str: cef::CefStringUtf16 = path.to_string().as_str().into();
            drag_data.add_file(Some(&path_str), None);
        }

        let mouse_event = input::create_mouse_event(
            position,
            self.get_pixel_scale_factor(),
            self.get_device_scale_factor(),
            0,
        );

        #[cfg(target_os = "windows")]
        let ops = cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(allowed_ops));
        #[cfg(not(target_os = "windows"))]
        let ops =
            cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(allowed_ops as u32));

        host.drag_target_drag_enter(Some(&mut drag_data), Some(&mouse_event), ops);

        self.app.drag_state.is_drag_over = true;
        self.app.drag_state.allowed_ops = allowed_ops as u32;
    }

    #[func]
    pub fn drag_over(&mut self, position: Vector2, allowed_ops: i32) {
        let Some(host) = self.app.host() else {
            return;
        };

        let mouse_event = input::create_mouse_event(
            position,
            self.get_pixel_scale_factor(),
            self.get_device_scale_factor(),
            0,
        );

        #[cfg(target_os = "windows")]
        let ops = cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(allowed_ops));
        #[cfg(not(target_os = "windows"))]
        let ops =
            cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(allowed_ops as u32));

        host.drag_target_drag_over(Some(&mouse_event), ops);
    }

    #[func]
    pub fn drag_leave(&mut self) {
        let Some(host) = self.app.host() else {
            return;
        };

        host.drag_target_drag_leave();

        self.app.drag_state.is_drag_over = false;
    }

    #[func]
    pub fn drag_drop(&mut self, position: Vector2) {
        let Some(host) = self.app.host() else {
            return;
        };

        let mouse_event = input::create_mouse_event(
            position,
            self.get_pixel_scale_factor(),
            self.get_device_scale_factor(),
            0,
        );

        host.drag_target_drop(Some(&mouse_event));

        self.app.drag_state.is_drag_over = false;
    }

    #[func]
    pub fn drag_source_ended(&mut self, position: Vector2, operation: i32) {
        let Some(host) = self.app.host() else {
            return;
        };

        #[cfg(target_os = "windows")]
        let op = cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(operation));
        #[cfg(not(target_os = "windows"))]
        let op =
            cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(operation as u32));

        host.drag_source_ended_at(position.x as i32, position.y as i32, op);

        self.app.drag_state.is_dragging_from_browser = false;
    }

    #[func]
    pub fn drag_source_system_ended(&mut self) {
        if let Some(host) = self.app.host() {
            host.drag_source_system_drag_ended();
        }
    }

    #[func]
    pub fn is_dragging_from_browser(&self) -> bool {
        self.app.drag_state.is_dragging_from_browser
    }

    #[func]
    pub fn is_drag_over(&self) -> bool {
        self.app.drag_state.is_drag_over
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
    pub fn grant_permission(&self, request_id: i64) -> bool {
        self.resolve_permission_request(request_id, true)
    }

    #[func]
    pub fn deny_permission(&self, request_id: i64) -> bool {
        self.resolve_permission_request(request_id, false)
    }

    fn media_permission_none_mask() -> u32 {
        #[cfg(target_os = "windows")]
        {
            cef::MediaAccessPermissionTypes::NONE.get_raw() as u32
        }
        #[cfg(not(target_os = "windows"))]
        {
            cef::MediaAccessPermissionTypes::NONE.get_raw()
        }
    }

    fn resolve_permission_request(&self, request_id: i64, grant: bool) -> bool {
        use crate::browser::{PendingPermissionAggregate, PendingPermissionDecision};

        let Some(state) = self.app.state.as_ref() else {
            godot::global::godot_warn!(
                "[CefTexture] Cannot resolve permission request {}: no active browser",
                request_id
            );
            return false;
        };

        let (decision, remaining_for_callback) = {
            let Ok(mut pending) = state.pending_permission_requests.lock() else {
                godot::global::godot_warn!(
                    "[CefTexture] Failed to lock pending permission requests"
                );
                return false;
            };
            let Some(decision) = pending.remove(&request_id) else {
                return {
                    godot::global::godot_warn!(
                        "[CefTexture] Unknown or stale permission request id: {}",
                        request_id
                    );
                    false
                };
            };

            let token = match &decision {
                PendingPermissionDecision::Media { callback_token, .. } => *callback_token,
                PendingPermissionDecision::Prompt { callback_token, .. } => *callback_token,
            };
            let remaining_for_callback = pending
                .values()
                .filter(|entry| match entry {
                    PendingPermissionDecision::Media { callback_token, .. } => {
                        *callback_token == token
                    }
                    PendingPermissionDecision::Prompt { callback_token, .. } => {
                        *callback_token == token
                    }
                })
                .count();

            (decision, remaining_for_callback)
        };

        let mut aggregates = match state.pending_permission_aggregates.lock() {
            Ok(aggregates) => aggregates,
            Err(_) => {
                godot::global::godot_warn!(
                    "[CefTexture] Failed to lock pending permission aggregates"
                );
                return false;
            }
        };

        match decision {
            PendingPermissionDecision::Media {
                callback,
                permission_bit,
                callback_token,
            } => {
                let entry = aggregates
                    .entry(callback_token)
                    .or_insert_with(|| PendingPermissionAggregate::new_media(callback.clone(), 0));
                match entry {
                    PendingPermissionAggregate::Media { granted_mask, .. } => {
                        if grant {
                            *granted_mask |= permission_bit;
                        }
                    }
                    PendingPermissionAggregate::Prompt { .. } => {
                        godot::global::godot_warn!(
                            "[CefTexture] Permission aggregate type mismatch for callback token {}",
                            callback_token
                        );
                        *entry = PendingPermissionAggregate::new_media(
                            callback.clone(),
                            if grant { permission_bit } else { 0 },
                        );
                    }
                }

                if remaining_for_callback == 0
                    && let Some(PendingPermissionAggregate::Media {
                        callback,
                        granted_mask,
                    }) = aggregates.remove(&callback_token)
                {
                    let allowed_mask = if granted_mask == 0 {
                        Self::media_permission_none_mask()
                    } else {
                        granted_mask
                    };
                    callback.cont(allowed_mask);
                }
            }
            PendingPermissionDecision::Prompt {
                callback,
                callback_token,
                ..
            } => {
                let entry = aggregates.entry(callback_token).or_insert_with(|| {
                    PendingPermissionAggregate::new_prompt(callback.clone(), true)
                });
                match entry {
                    PendingPermissionAggregate::Prompt { all_granted, .. } => {
                        *all_granted &= grant;
                    }
                    PendingPermissionAggregate::Media { .. } => {
                        godot::global::godot_warn!(
                            "[CefTexture] Permission aggregate type mismatch for callback token {}",
                            callback_token
                        );
                        *entry = PendingPermissionAggregate::new_prompt(callback.clone(), grant);
                    }
                }

                if remaining_for_callback == 0
                    && let Some(PendingPermissionAggregate::Prompt {
                        callback,
                        all_granted,
                    }) = aggregates.remove(&callback_token)
                {
                    let result = if all_granted {
                        cef::PermissionRequestResult::ACCEPT
                    } else {
                        cef::PermissionRequestResult::DENY
                    };
                    callback.cont(result);
                }
            }
        }

        true
    }

    // ── Cookie & Session Management ─────────────────────────────────────────

    /// Gets the `CookieManager` for this browser's `RequestContext`.
    fn cookie_manager(&self) -> Option<cef::CookieManager> {
        use cef::ImplBrowserHost;
        let host = self.app.host()?;
        let ctx = host.request_context()?;
        use cef::ImplRequestContext;
        ctx.cookie_manager(None)
    }

    /// Retrieves all cookies. Results are emitted via `cookies_received` signal.
    /// Returns `true` if the request was initiated, `false` on failure.
    #[func]
    pub fn get_all_cookies(&self) -> bool {
        let Some(event_queues) = self.app.state.as_ref().map(|s| &s.event_queues) else {
            return false;
        };
        let Some(manager) = self.cookie_manager() else {
            return false;
        };
        use cef::ImplCookieManager;
        let mut visitor = crate::cookie::CookieVisitorImpl::build(event_queues.clone());
        manager.visit_all_cookies(Some(&mut visitor)) != 0
    }

    /// Retrieves cookies for a specific URL. Results are emitted via `cookies_received` signal.
    /// If `include_http_only` is true, HTTP-only cookies are included.
    /// Returns `true` if the request was initiated, `false` on failure.
    #[func]
    pub fn get_cookies(&self, url: GString, include_http_only: bool) -> bool {
        let Some(event_queues) = self.app.state.as_ref().map(|s| &s.event_queues) else {
            return false;
        };
        let Some(manager) = self.cookie_manager() else {
            return false;
        };
        use cef::ImplCookieManager;
        let url_cef = cef::CefStringUtf16::from(url.to_string().as_str());
        let mut visitor = crate::cookie::CookieVisitorImpl::build(event_queues.clone());
        manager.visit_url_cookies(Some(&url_cef), include_http_only as _, Some(&mut visitor)) != 0
    }

    /// Sets a cookie for the given URL.
    /// The result is emitted via the `cookie_set` signal.
    /// Returns `true` if the request was initiated, `false` on failure.
    #[func]
    #[allow(clippy::too_many_arguments)] // GDScript-facing API; each parameter is user-visible
    pub fn set_cookie(
        &self,
        url: GString,
        name: GString,
        value: GString,
        domain: GString,
        path: GString,
        secure: bool,
        httponly: bool,
    ) -> bool {
        let Some(event_queues) = self.app.state.as_ref().map(|s| &s.event_queues) else {
            return false;
        };
        let Some(manager) = self.cookie_manager() else {
            return false;
        };
        use cef::ImplCookieManager;

        let url_cef = cef::CefStringUtf16::from(url.to_string().as_str());
        let cookie = cef::Cookie {
            size: std::mem::size_of::<cef::Cookie>(),
            name: cef::CefStringUtf16::from(name.to_string().as_str()),
            value: cef::CefStringUtf16::from(value.to_string().as_str()),
            domain: cef::CefStringUtf16::from(domain.to_string().as_str()),
            path: cef::CefStringUtf16::from(path.to_string().as_str()),
            secure: secure as _,
            httponly: httponly as _,
            ..Default::default()
        };
        let mut callback = crate::cookie::SetCookieCallbackImpl::build(event_queues.clone());
        manager.set_cookie(Some(&url_cef), Some(&cookie), Some(&mut callback)) != 0
    }

    /// Deletes cookies matching the given URL and/or name.
    /// Pass empty strings to delete all cookies.
    /// The result is emitted via the `cookies_deleted` signal.
    /// Returns `true` if the request was initiated, `false` on failure.
    #[func]
    pub fn delete_cookies(&self, url: GString, cookie_name: GString) -> bool {
        let Some(event_queues) = self.app.state.as_ref().map(|s| &s.event_queues) else {
            return false;
        };
        let Some(manager) = self.cookie_manager() else {
            return false;
        };
        use cef::ImplCookieManager;

        let url_str = url.to_string();
        let name_str = cookie_name.to_string();
        let url_opt = if url_str.is_empty() {
            None
        } else {
            Some(cef::CefStringUtf16::from(url_str.as_str()))
        };
        let name_opt = if name_str.is_empty() {
            None
        } else {
            Some(cef::CefStringUtf16::from(name_str.as_str()))
        };
        let mut callback = crate::cookie::DeleteCookiesCallbackImpl::build(event_queues.clone());
        manager.delete_cookies(url_opt.as_ref(), name_opt.as_ref(), Some(&mut callback)) != 0
    }

    /// Convenience method to delete all cookies.
    /// Equivalent to `delete_cookies("", "")`.
    #[func]
    pub fn clear_cookies(&self) -> bool {
        self.delete_cookies("".into(), "".into())
    }

    /// Flushes the cookie store to disk.
    /// The `cookies_flushed` signal is emitted when complete.
    /// Returns `true` if the request was initiated, `false` on failure.
    #[func]
    pub fn flush_cookies(&self) -> bool {
        let Some(event_queues) = self.app.state.as_ref().map(|s| &s.event_queues) else {
            return false;
        };
        let Some(manager) = self.cookie_manager() else {
            return false;
        };
        use cef::ImplCookieManager;
        let mut callback = crate::cookie::FlushCookieStoreCallbackImpl::build(event_queues.clone());
        manager.flush_store(Some(&mut callback)) != 0
    }
}
