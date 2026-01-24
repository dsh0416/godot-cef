mod browser_lifecycle;
mod ime;
mod rendering;
mod signals;

use cef::{self, ImplBrowser, ImplBrowserHost, ImplDragData, ImplFrame, do_message_loop_work};
use cef_app::SecurityConfig;
use godot::classes::notify::ControlNotification;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{
    ITextureRect, ImageTexture, InputEvent, InputEventKey, InputEventMouseButton,
    InputEventMouseMotion, InputEventPanGesture, LineEdit, TextureRect,
};
use godot::prelude::*;

use crate::browser::App;
use crate::{cef_init, input};

#[derive(GodotClass)]
#[class(base=TextureRect)]
pub struct CefTexture {
    base: Base<TextureRect>,
    app: App,

    #[export]
    #[var(get = get_url_property, set = set_url_property)]
    url: GString,

    #[export]
    enable_accelerated_osr: bool,

    #[export]
    allow_insecure_content: bool,

    #[export]
    ignore_certificate_errors: bool,

    #[export]
    disable_web_security: bool,

    #[var]
    /// Stores the IME cursor position in local coordinates (relative to this `CefTexture` node),
    /// automatically updated from the browser's caret position.
    ime_position: Vector2i,

    // Change detection state
    last_size: Vector2,
    last_dpi: f32,
    last_cursor: cef_app::CursorType,
    last_max_fps: i32,

    // IME state
    ime_active: bool,
    ime_proxy: Option<Gd<LineEdit>>,
    ime_focus_regrab_pending: bool,

    // Popup state
    popup_overlay: Option<Gd<TextureRect>>,
    popup_texture: Option<Gd<ImageTexture>>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    popup_texture_2d_rd: Option<Gd<godot::classes::Texture2Drd>>,
}

#[godot_api]
impl ITextureRect for CefTexture {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
            allow_insecure_content: false,
            ignore_certificate_errors: false,
            disable_web_security: false,
            ime_position: Vector2i::new(0, 0),
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_cursor: cef_app::CursorType::Arrow,
            last_max_fps: 0,
            ime_active: false,
            ime_proxy: None,
            ime_focus_regrab_pending: false,
            popup_overlay: None,
            popup_texture: None,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            popup_texture_2d_rd: None,
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
                self.on_focus_enter();
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

    #[func]
    fn on_ready(&mut self) {
        use godot::classes::control::FocusMode;
        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);
        // Must explicitly enable processing when using on_notification instead of fn process()
        self.base_mut().set_process(true);
        // Enable focus so we receive FOCUS_ENTER/EXIT notifications and can forward to CEF
        self.base_mut().set_focus_mode(FocusMode::CLICK);

        if let Err(e) = cef_init::cef_retain_with_security(SecurityConfig {
            allow_insecure_content: self.allow_insecure_content,
            ignore_certificate_errors: self.ignore_certificate_errors,
            disable_web_security: self.disable_web_security,
        }) {
            godot::global::godot_error!("[CefTexture] {}", e);
            return;
        }

        // Create hidden LineEdit for IME proxy
        self.create_ime_proxy();
        self.create_browser();
    }

    #[func]
    fn on_process(&mut self) {
        self.handle_max_fps_change();
        _ = self.handle_size_change();
        self.update_texture();

        do_message_loop_work();

        self.request_external_begin_frame();
        self.update_cursor();
        self.process_message_queue();
        self.process_url_change_queue();
        self.process_title_change_queue();
        self.process_loading_state_queue();
        self.process_console_message_queue();
        self.process_drag_event_queue();
        self.process_ime_enable_queue();
        self.process_ime_composition_queue();
        self.process_ime_position();
    }

    fn handle_input_event(&mut self, event: Gd<InputEvent>) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
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
        } else if let Ok(key_event) = event.try_cast::<InputEventKey>() {
            input::handle_key_event(
                &host,
                browser.main_frame().as_ref(),
                &key_event,
                self.ime_active,
            );
        }
    }

    #[func]
    pub fn eval(&mut self, code: GString) {
        let Some(browser) = self.app.browser.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot execute JS: no browser");
            return;
        };
        let Some(frame) = browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot execute JS: no main frame");
            return;
        };

        let code_str: cef::CefStringUtf16 = code.to_string().as_str().into();
        frame.execute_java_script(Some(&code_str), None, 0);
    }

    #[func]
    fn set_url_property(&mut self, url: GString) {
        self.url = url.clone();

        if let Some(browser) = self.app.browser.as_ref()
            && let Some(frame) = browser.main_frame()
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
    /// helper:
    /// - automatically escapes the string payload for safe JS embedding, and
    /// - enforces a consistent IPC pattern (`window.onIpcMessage(message)`).
    ///
    /// Use this when you want structured IPC into the page, and `eval` when
    /// you truly need arbitrary JavaScript execution.
    pub fn send_ipc_message(&mut self, message: GString) {
        let Some(browser) = self.app.browser.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC message: no browser");
            return;
        };
        let Some(frame) = browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC message: no main frame");
            return;
        };

        // Use serde_json for proper JSON encoding which handles all edge cases:
        // - Unicode line terminators (U+2028, U+2029) that can break JS strings
        // - Backticks, single quotes, and all control characters
        // - Proper backslash and quote escaping
        // The result includes surrounding quotes, so we use it directly.
        let msg_str = message.to_string();
        let json_msg = serde_json::to_string(&msg_str).unwrap_or_else(|_| "\"\"".to_string());

        let js_code = format!(
            r#"if (typeof window.onIpcMessage === 'function') {{ window.onIpcMessage({}); }}"#,
            json_msg
        );
        let js_code_str: cef::CefStringUtf16 = js_code.as_str().into();
        frame.execute_java_script(Some(&js_code_str), None, 0);
    }

    #[func]
    pub fn go_back(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.go_back();
        }
    }

    #[func]
    pub fn go_forward(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.go_forward();
        }
    }

    #[func]
    pub fn can_go_back(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .map(|b| b.can_go_back() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn can_go_forward(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .map(|b| b.can_go_forward() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn reload(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.reload();
        }
    }

    #[func]
    pub fn reload_ignore_cache(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.reload_ignore_cache();
        }
    }

    #[func]
    pub fn stop_loading(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.stop_load();
        }
    }

    #[func]
    pub fn is_loading(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .map(|b| b.is_loading() != 0)
            .unwrap_or(false)
    }

    #[func]
    fn get_url_property(&self) -> GString {
        if let Some(browser) = self.app.browser.as_ref()
            && let Some(frame) = browser.main_frame()
        {
            let frame_url = frame.url();
            let url_string = cef::CefStringUtf16::from(&frame_url).to_string();
            return GString::from(url_string.as_str());
        }
        self.url.clone()
    }

    #[func]
    pub fn set_zoom_level(&mut self, level: f64) {
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_zoom_level(level);
        }
    }

    #[func]
    pub fn get_zoom_level(&self) -> f64 {
        self.app
            .browser
            .as_ref()
            .and_then(|b| b.host())
            .map(|h| h.zoom_level())
            .unwrap_or(0.0)
    }

    #[func]
    pub fn set_audio_muted(&mut self, muted: bool) {
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_audio_muted(muted as i32);
        }
    }

    #[func]
    pub fn is_audio_muted(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .and_then(|b| b.host())
            .map(|h| h.is_audio_muted() != 0)
            .unwrap_or(false)
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

    fn on_focus_enter(&mut self) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };

        host.set_focus(true as _);
    }

    fn get_pixel_scale_factor(&self) -> f32 {
        self.base()
            .get_viewport()
            .unwrap()
            .get_stretch_transform()
            .a
            .x
    }

    fn get_device_scale_factor(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    #[func]
    pub fn drag_enter(&mut self, file_paths: Array<GString>, position: Vector2, allowed_ops: i32) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
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
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
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
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };

        host.drag_target_drag_leave();

        self.app.drag_state.is_drag_over = false;
    }

    #[func]
    pub fn drag_drop(&mut self, position: Vector2) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
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
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
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
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };

        host.drag_source_system_drag_ended();
    }

    #[func]
    pub fn is_dragging_from_browser(&self) -> bool {
        self.app.drag_state.is_dragging_from_browser
    }

    #[func]
    pub fn is_drag_over(&self) -> bool {
        self.app.drag_state.is_drag_over
    }
}
