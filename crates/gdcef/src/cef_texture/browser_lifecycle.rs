use super::CefTexture;
use cef::{BrowserSettings, ImplBrowser, ImplBrowserHost, RequestContextSettings, WindowInfo};
use cef_app::PhysicalSize;
use godot::classes::{AudioServer, ImageTexture};
use godot::prelude::*;
use std::sync::{Arc, Mutex};

use crate::accelerated_osr::{
    self, AcceleratedRenderState, GodotTextureImporter, PlatformAcceleratedRenderHandler,
};
use crate::browser::{BrowserState, PopupStateQueue, RenderMode};
use crate::error::CefError;
use crate::{godot_protocol, render, webrender};

fn color_to_cef_color(color: Color) -> u32 {
    let f = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
    u32::from_be_bytes([f(color.a), f(color.r), f(color.g), f(color.b)])
}

impl CefTexture {
    fn log_cleanup_state_violations(&self) {
        if self.app.state.is_some() {
            godot::global::godot_warn!(
                "[CefTexture] Cleanup invariant violation: runtime state not fully cleared"
            );
        }
    }

    pub(super) fn cleanup_instance(&mut self) {
        if self.app.state.is_none() {
            crate::cef_init::cef_release();
            return;
        }

        // Signal audio handler that we're shutting down to suppress "socket closed" errors
        if let Some(state) = &self.app.state
            && let Some(audio) = &state.audio
        {
            use std::sync::atomic::Ordering;
            audio.shutdown_flag.store(true, Ordering::Relaxed);
        }

        // Hide the TextureRect and clear its texture BEFORE freeing resources.
        // This prevents Godot from trying to render with an invalid texture during shutdown.
        self.base_mut().set_visible(false);

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(state) = &mut self.app.state
            && let RenderMode::Accelerated {
                render_state,
                texture_2d_rd,
            } = &mut state.render_mode
        {
            // Clear the RD texture RID from the Texture2Drd to break the reference
            // before we free the underlying RD texture.
            texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
            if let Some(popup_texture_2d_rd) = &mut self.popup_texture_2d_rd {
                popup_texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
            }
            if let Ok(mut rs) = render_state.lock() {
                render::free_rd_texture(rs.dst_rd_rid);
                // Also free popup texture RID if it exists
                if let Some(popup_rid) = rs.popup_rd_rid.take() {
                    render::free_rd_texture(popup_rid);
                }
            }
        }

        if let Some(state) = self.app.state.take()
            && let Some(host) = state.browser.host()
        {
            host.close_browser(true as _);
        }

        self.app.clear_runtime_state();

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
        crate::cef_init::cef_release();
    }

    pub(super) fn create_browser(&mut self) {
        if let Err(e) = self.try_create_browser() {
            godot::global::godot_error!("[CefTexture] {}", e);
        }
    }

    pub(super) fn try_create_browser(&mut self) -> Result<(), CefError> {
        // Prevent double-initialization: if browser already exists, do nothing.
        // This avoids resource leaks (unclosed browser handles, leaked textures, etc.).
        if self.app.state.is_some() {
            return Ok(());
        }

        let logical_size = self.base().get_size();

        // Validate size before attempting to create browser.
        // A zero or negative size will crash CEF subprocess.
        if logical_size.x <= 0.0 || logical_size.y <= 0.0 {
            return Err(CefError::InvalidSize {
                width: logical_size.x,
                height: logical_size.y,
            });
        }

        let dpi = self.get_pixel_scale_factor();
        let pixel_width = (logical_size.x * dpi) as i32;
        let pixel_height = (logical_size.y * dpi) as i32;

        let use_accelerated = self.should_use_accelerated_osr();

        let window_info = WindowInfo {
            bounds: cef::Rect {
                x: 0,
                y: 0,
                width: pixel_width,
                height: pixel_height,
            },
            windowless_rendering_enabled: true as _,
            shared_texture_enabled: use_accelerated as _,
            external_begin_frame_enabled: true as _,
            ..Default::default()
        };

        let browser_settings = BrowserSettings {
            windowless_frame_rate: self.get_max_fps(),
            background_color: color_to_cef_color(self.background_color),
            ..Default::default()
        };

        let mut context = cef::request_context_create_context(
            Some(&RequestContextSettings::default()),
            Some(&mut webrender::RequestContextHandlerImpl::build(
                webrender::OsrRequestContextHandler {},
            )),
        );

        // Register the res:// and user:// scheme handlers on this specific request context
        if let Some(ctx) = context.as_mut() {
            godot_protocol::register_res_scheme_handler_on_context(ctx);
            godot_protocol::register_user_scheme_handler_on_context(ctx);
        }

        if use_accelerated {
            self.create_accelerated_browser(
                &window_info,
                &browser_settings,
                context.as_mut(),
                dpi,
                pixel_width,
                pixel_height,
            )?;
        } else {
            self.create_software_browser(
                &window_info,
                &browser_settings,
                context.as_mut(),
                dpi,
                pixel_width,
                pixel_height,
            )?;
        }

        self.last_size = logical_size;
        self.last_dpi = dpi;
        Ok(())
    }

    fn should_use_accelerated_osr(&self) -> bool {
        if !self.enable_accelerated_osr {
            godot::global::godot_print!(
                "[CefTexture] Accelerated OSR disabled by `enable_accelerated_osr = false`; using software rendering"
            );
            return false;
        }

        let (supported, reason) = accelerated_osr::accelerated_osr_support_diagnostic();
        if !supported {
            godot::global::godot_warn!(
                "[CefTexture] Accelerated OSR unavailable: {}. Falling back to software rendering.",
                reason
            );
        }
        supported
    }

    fn create_software_browser(
        &mut self,
        _window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Result<(), CefError> {
        godot::global::godot_print!("[CefTexture] Creating browser in software rendering mode");
        let window_info = WindowInfo {
            bounds: cef::Rect {
                x: 0,
                y: 0,
                width: pixel_width,
                height: pixel_height,
            },
            windowless_rendering_enabled: true as _,
            shared_texture_enabled: false as _,
            external_begin_frame_enabled: true as _,
            ..Default::default()
        };

        let render_handler = cef_app::OsrRenderHandler::new(
            dpi,
            PhysicalSize::new(pixel_width as f32, pixel_height as f32),
        );

        let frame_buffer = render_handler.get_frame_buffer();
        let render_size = render_handler.get_size();
        let device_scale_factor = render_handler.get_device_scale_factor();
        let cursor_type = render_handler.get_cursor_type();
        let popup_state: PopupStateQueue = render_handler.get_popup_state();
        let sample_rate = AudioServer::singleton().get_mix_rate();
        let enable_audio_capture = crate::settings::is_audio_capture_enabled();
        let queues = webrender::ClientQueues::new(sample_rate, enable_audio_capture);

        let texture = ImageTexture::new_gd();

        let cef_render_handler =
            webrender::SoftwareOsrHandler::build(render_handler, queues.event_queues.clone());
        let mut client = webrender::CefClientImpl::build(
            cef_render_handler,
            cursor_type.clone(),
            queues.clone(),
        );

        // Attempt browser creation first, before updating any app state
        let browser = cef::browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(browser_settings),
            None,
            context,
        )
        .ok_or_else(|| {
            CefError::BrowserCreationFailed("browser_host_create_browser_sync returned None".into())
        })?;

        // Browser created successfully - now update app state
        self.base_mut().set_texture(&texture);
        let event_queues = queues.event_queues.clone();
        self.app.state = Some(BrowserState {
            browser,
            render_mode: RenderMode::Software {
                frame_buffer,
                texture,
            },
            render_size,
            device_scale_factor,
            cursor_type,
            popup_state,
            event_queues,
            audio: queues.into_audio_state(),
        });

        Ok(())
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn create_accelerated_browser(
        &mut self,
        window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Result<(), CefError> {
        godot::global::godot_print!("[CefTexture] Creating browser in accelerated rendering mode");
        let importer = match GodotTextureImporter::new() {
            Some(imp) => imp,
            None => {
                godot::global::godot_warn!(
                    "Failed to create GPU texture importer, falling back to software rendering"
                );
                return self.create_software_browser(
                    window_info,
                    browser_settings,
                    context,
                    dpi,
                    pixel_width,
                    pixel_height,
                );
            }
        };

        // Create the RD texture first
        let (rd_texture_rid, texture_2d_rd) = render::create_rd_texture(pixel_width, pixel_height)?;

        // Create shared render state with the importer and destination texture
        let render_state = Arc::new(Mutex::new(AcceleratedRenderState::new(
            importer,
            rd_texture_rid,
            pixel_width as u32,
            pixel_height as u32,
        )));

        // Create render handler and give it the shared state
        let mut render_handler = PlatformAcceleratedRenderHandler::new(
            dpi,
            PhysicalSize::new(pixel_width as f32, pixel_height as f32),
        );
        render_handler.set_render_state(render_state.clone());

        let render_size = render_handler.get_size();
        let device_scale_factor = render_handler.get_device_scale_factor();
        let cursor_type = render_handler.get_cursor_type();
        let popup_state: PopupStateQueue = render_handler.get_popup_state();
        let sample_rate = AudioServer::singleton().get_mix_rate();
        let enable_audio_capture = crate::settings::is_audio_capture_enabled();
        let queues = webrender::ClientQueues::new(sample_rate, enable_audio_capture);

        let cef_render_handler =
            webrender::AcceleratedOsrHandler::build(render_handler, queues.event_queues.clone());
        let mut client = webrender::CefClientImpl::build(
            cef_render_handler,
            cursor_type.clone(),
            queues.clone(),
        );

        // Attempt browser creation first, before updating any app state
        let browser = match cef::browser_host_create_browser_sync(
            Some(window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(browser_settings),
            None,
            context,
        ) {
            Some(browser) => browser,
            None => {
                // Browser creation failed - clean up the RD texture to prevent leak
                render::free_rd_texture(rd_texture_rid);
                return Err(CefError::BrowserCreationFailed(
                    "browser_host_create_browser_sync returned None (accelerated)".into(),
                ));
            }
        };

        // Browser created successfully - now update app state
        self.base_mut().set_texture(&texture_2d_rd);
        let event_queues = queues.event_queues.clone();
        self.app.state = Some(BrowserState {
            browser,
            render_mode: RenderMode::Accelerated {
                render_state,
                texture_2d_rd,
            },
            render_size,
            device_scale_factor,
            cursor_type,
            popup_state,
            event_queues,
            audio: queues.into_audio_state(),
        });

        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn create_accelerated_browser(
        &mut self,
        window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Result<(), CefError> {
        self.create_software_browser(
            window_info,
            browser_settings,
            context,
            dpi,
            pixel_width,
            pixel_height,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_to_cef_color_opaque_red() {
        // Opaque red: (r=1, g=0, b=0, a=1) → ARGB bytes [255, 255, 0, 0]
        let color = Color::from_rgba(1.0, 0.0, 0.0, 1.0);
        assert_eq!(color_to_cef_color(color), 0xFF_FF_00_00);
    }

    #[test]
    fn test_color_to_cef_color_opaque_green() {
        let color = Color::from_rgba(0.0, 1.0, 0.0, 1.0);
        assert_eq!(color_to_cef_color(color), 0xFF_00_FF_00);
    }

    #[test]
    fn test_color_to_cef_color_opaque_blue() {
        let color = Color::from_rgba(0.0, 0.0, 1.0, 1.0);
        assert_eq!(color_to_cef_color(color), 0xFF_00_00_FF);
    }

    #[test]
    fn test_color_to_cef_color_transparent_white() {
        let color = Color::from_rgba(1.0, 1.0, 1.0, 0.0);
        assert_eq!(color_to_cef_color(color), 0x00_FF_FF_FF);
    }

    #[test]
    fn test_color_to_cef_color_half_alpha() {
        // ~50% alpha, black
        let color = Color::from_rgba(0.0, 0.0, 0.0, 0.5);
        let result = color_to_cef_color(color);
        // Alpha byte should be ~127
        let alpha = (result >> 24) & 0xFF;
        assert!((126..=128).contains(&alpha), "alpha was {}", alpha);
    }

    #[test]
    fn test_color_to_cef_color_clamps_out_of_range() {
        // Values >1 or <0 should be clamped
        let color = Color::from_rgba(2.0, -1.0, 0.5, 1.5);
        let result = color_to_cef_color(color);
        assert_eq!(result, 0xFF_FF_00_7F);
    }
}
