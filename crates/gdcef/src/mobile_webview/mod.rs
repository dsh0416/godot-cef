mod platform;

#[cfg(target_os = "android")]
use godot::classes::ExternalTexture;
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ObjectNotification;
use godot::classes::{ITexture2D, Image, ImageTexture, InputEvent, RenderingServer, Texture2D};
use godot::prelude::*;
use std::sync::atomic::{AtomicI64, Ordering};

use platform::{AndroidWebViewBridge, FrameUpdate};

static NEXT_WEBVIEW_ID: AtomicI64 = AtomicI64::new(1);

#[derive(GodotClass)]
#[class(base=Texture2D, tool)]
pub struct AndroidWebViewTexture {
    base: Base<Texture2D>,

    #[export]
    #[var(get = get_url_property, set = set_url_property)]
    url: GString,

    #[export]
    #[var(get = get_texture_size_property, set = set_texture_size_property)]
    texture_size: Vector2i,

    #[export]
    #[var(get = get_javascript_enabled, set = set_javascript_enabled)]
    javascript_enabled: bool,

    fallback_texture: Gd<ImageTexture>,
    #[cfg(target_os = "android")]
    external_texture: Gd<ExternalTexture>,
    bridge: AndroidWebViewBridge,
    instance_id: i64,
    started: bool,
    last_external_buffer_id: u64,
    frame_hook_callable: Option<Callable>,
    frame_hook_connected: bool,
}

#[godot_api]
impl ITexture2D for AndroidWebViewTexture {
    fn init(base: Base<Texture2D>) -> Self {
        let texture_size = Vector2i::new(1024, 1024);
        let fallback_texture = Self::make_placeholder_texture(texture_size);
        let frame_hook_callable = base.to_init_gd().callable("_on_frame_pre_draw");
        RenderingServer::singleton().connect("frame_pre_draw", &frame_hook_callable);

        #[cfg(target_os = "android")]
        let external_texture = {
            let mut texture = ExternalTexture::new_gd();
            texture.set_size(Vector2::new(texture_size.x as f32, texture_size.y as f32));
            texture
        };

        Self {
            base,
            url: "https://example.com".into(),
            texture_size,
            javascript_enabled: true,
            fallback_texture,
            #[cfg(target_os = "android")]
            external_texture,
            bridge: AndroidWebViewBridge::new(),
            instance_id: NEXT_WEBVIEW_ID.fetch_add(1, Ordering::Relaxed),
            started: false,
            last_external_buffer_id: 0,
            frame_hook_callable: Some(frame_hook_callable),
            frame_hook_connected: true,
        }
    }

    fn on_notification(&mut self, what: ObjectNotification) {
        if what == ObjectNotification::PREDELETE {
            self.cleanup_instance();
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
        #[cfg(target_os = "android")]
        if self.started {
            return self.external_texture.get_rid();
        }

        self.fallback_texture.get_rid()
    }
}

#[godot_api]
impl AndroidWebViewTexture {
    #[signal]
    fn load_started(url: GString);

    #[signal]
    fn load_finished(url: GString);

    #[signal]
    fn load_error(url: GString, error_text: GString);

    #[signal]
    fn url_changed(url: GString);

    #[signal]
    fn title_changed(title: GString);

    #[func]
    fn _on_frame_pre_draw(&mut self) {
        self.tick();
    }

    #[func]
    pub fn start(&mut self) {
        if self.started {
            return;
        }

        match self.bridge.create(
            self.instance_id,
            self.texture_size,
            self.url.to_string(),
            self.javascript_enabled,
        ) {
            Ok(()) => {
                self.started = true;
                self.base_mut().emit_changed();
            }
            Err(error) => {
                godot::global::godot_warn!(
                    "[AndroidWebViewTexture] Failed to start WebView backend: {}",
                    error
                );
            }
        }
    }

    #[func]
    pub fn shutdown(&mut self) {
        if !self.started {
            return;
        }
        if let Err(error) = self.bridge.shutdown(self.instance_id) {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to shutdown WebView backend: {}",
                error
            );
        }
        self.started = false;
        self.last_external_buffer_id = 0;
        self.base_mut().emit_changed();
    }

    #[func]
    pub fn update_frame(&mut self) -> bool {
        if !self.started {
            return false;
        }

        let update = match self.bridge.acquire_latest_frame(self.instance_id) {
            Ok(Some(update)) => update,
            Ok(None) => return false,
            Err(error) => {
                godot::global::godot_warn!(
                    "[AndroidWebViewTexture] Failed to acquire WebView frame: {}",
                    error
                );
                return false;
            }
        };

        self.apply_frame_update(update)
    }

    #[func]
    pub fn eval(&mut self, code: GString) {
        if let Err(error) = self.bridge.eval(self.instance_id, code.to_string()) {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to evaluate JavaScript: {}",
                error
            );
        }
    }

    #[func]
    pub fn reload(&mut self) {
        if let Err(error) = self.bridge.reload(self.instance_id) {
            godot::global::godot_warn!("[AndroidWebViewTexture] Failed to reload: {}", error);
        }
    }

    #[func]
    pub fn go_back(&mut self) {
        if let Err(error) = self.bridge.go_back(self.instance_id) {
            godot::global::godot_warn!("[AndroidWebViewTexture] Failed to go back: {}", error);
        }
    }

    #[func]
    pub fn go_forward(&mut self) {
        if let Err(error) = self.bridge.go_forward(self.instance_id) {
            godot::global::godot_warn!("[AndroidWebViewTexture] Failed to go forward: {}", error);
        }
    }

    #[func]
    pub fn focus_webview(&mut self) {
        if let Err(error) = self.bridge.focus(self.instance_id) {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to focus WebView: {}",
                error
            );
        }
    }

    #[func]
    pub fn clear_webview_focus(&mut self) {
        if let Err(error) = self.bridge.clear_focus(self.instance_id) {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to clear WebView focus: {}",
                error
            );
        }
    }

    #[func]
    pub fn forward_input_event(&mut self, event: Gd<InputEvent>) {
        if let Err(error) = self.bridge.forward_input(self.instance_id, event) {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to forward input event: {}",
                error
            );
        }
    }

    #[func]
    pub(crate) fn get_url_property(&self) -> GString {
        self.url.clone()
    }

    #[func]
    pub(crate) fn set_url_property(&mut self, url: GString) {
        self.url = url.clone();
        if self.started
            && let Err(error) = self.bridge.load_url(self.instance_id, url.to_string())
        {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to load URL '{}': {}",
                url,
                error
            );
        }
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

        #[cfg(target_os = "android")]
        self.external_texture
            .set_size(Vector2::new(clamped.x as f32, clamped.y as f32));

        if self.started
            && let Err(error) = self.bridge.resize(self.instance_id, clamped)
        {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to resize WebView backend: {}",
                error
            );
        }

        self.base_mut().emit_changed();
    }

    #[func]
    pub(crate) fn get_javascript_enabled(&self) -> bool {
        self.javascript_enabled
    }

    #[func]
    pub(crate) fn set_javascript_enabled(&mut self, enabled: bool) {
        self.javascript_enabled = enabled;
        if self.started
            && let Err(error) = self
                .bridge
                .set_javascript_enabled(self.instance_id, enabled)
        {
            godot::global::godot_warn!(
                "[AndroidWebViewTexture] Failed to update JavaScript setting: {}",
                error
            );
        }
    }

    fn apply_frame_update(&mut self, update: FrameUpdate) -> bool {
        if update.width > 0 && update.height > 0 {
            let new_size = Vector2i::new(update.width, update.height);
            if new_size != self.texture_size {
                self.texture_size = new_size;
                self.refresh_fallback_texture();
                #[cfg(target_os = "android")]
                self.external_texture.set_size(Vector2::new(
                    self.texture_size.x as f32,
                    self.texture_size.y as f32,
                ));
            }
        }

        #[cfg(target_os = "android")]
        {
            if update.external_buffer_id != 0
                && update.external_buffer_id != self.last_external_buffer_id
            {
                self.external_texture
                    .set_external_buffer_id(update.external_buffer_id);
                self.last_external_buffer_id = update.external_buffer_id;
            }
        }

        self.base_mut().emit_changed();
        true
    }

    fn make_placeholder_texture(size: Vector2i) -> Gd<ImageTexture> {
        let bytes = vec![0u8; (size.x.max(1) * size.y.max(1) * 4) as usize];
        let byte_array = PackedByteArray::from(bytes.as_slice());
        let mut texture = ImageTexture::new_gd();
        if let Some(image) = Image::create_from_data(
            size.x.max(1),
            size.y.max(1),
            false,
            ImageFormat::RGBA8,
            &byte_array,
        ) {
            texture.set_image(&image);
        }
        texture
    }

    fn refresh_fallback_texture(&mut self) {
        self.fallback_texture = Self::make_placeholder_texture(self.texture_size);
    }

    fn tick(&mut self) {
        #[cfg(target_os = "android")]
        {
            if !self.started {
                self.start();
            }
            let _ = self.update_frame();
        }
    }

    fn cleanup_instance(&mut self) {
        self.shutdown();
        self.disconnect_frame_hook();
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
}

impl Drop for AndroidWebViewTexture {
    fn drop(&mut self) {
        self.cleanup_instance();
    }
}
