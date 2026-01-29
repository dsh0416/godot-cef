use super::CefTexture;
use cef::{BrowserSettings, ImplBrowser, ImplBrowserHost, RequestContextSettings, WindowInfo};
use cef_app::PhysicalSize;
use godot::classes::{AudioServer, ImageTexture};
use godot::prelude::*;
use std::sync::{Arc, Mutex};

use crate::accelerated_osr::{
    self, AcceleratedRenderState, GodotTextureImporter, PlatformAcceleratedRenderHandler,
};
use crate::browser::{PopupStateQueue, RenderMode};
use crate::error::CefError;
use crate::{godot_protocol, render, webrender};

fn get_godot_audio_sample_rate() -> i32 {
    AudioServer::singleton().get_mix_rate() as i32
}

impl CefTexture {
    pub(super) fn cleanup_instance(&mut self) {
        if self.app.browser.is_none() {
            crate::cef_init::cef_release();
            return;
        }

        // Signal audio handler that we're shutting down to suppress "socket closed" errors
        if let Some(ref shutdown_flag) = self.app.audio_shutdown_flag {
            use std::sync::atomic::Ordering;
            shutdown_flag.store(true, Ordering::Relaxed);
        }

        // Hide the TextureRect and clear its texture BEFORE freeing resources.
        // This prevents Godot from trying to render with an invalid texture during shutdown.
        self.base_mut().set_visible(false);

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(RenderMode::Accelerated {
            render_state,
            texture_2d_rd,
        }) = &mut self.app.render_mode
        {
            // Clear the RD texture RID from the Texture2Drd to break the reference
            // before we free the underlying RD texture.
            texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
            if let Some(popup_texture_2d_rd) = &mut self.popup_texture_2d_rd {
                popup_texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
            }
            if let Ok(mut state) = render_state.lock() {
                render::free_rd_texture(state.dst_rd_rid);
                // Also free popup texture RID if it exists
                if let Some(popup_rid) = state.popup_rd_rid.take() {
                    render::free_rd_texture(popup_rid);
                }
            }
        }

        if let Some(browser) = self.app.browser.take()
            && let Some(host) = browser.host()
        {
            host.close_browser(true as _);
        }

        self.app.render_mode = None;
        self.app.render_size = None;
        self.app.device_scale_factor = None;
        self.app.cursor_type = None;
        self.app.popup_state = None;
        self.app.message_queue = None;
        self.app.url_change_queue = None;
        self.app.title_change_queue = None;
        self.app.loading_state_queue = None;
        self.app.ime_enable_queue = None;
        self.app.ime_composition_range = None;
        self.app.console_message_queue = None;
        self.app.drag_event_queue = None;
        self.app.drag_state = Default::default();
        self.app.audio_packet_queue = None;
        self.app.audio_params = None;
        self.app.audio_sample_rate = None;
        self.app.download_request_queue = None;
        self.app.download_update_queue = None;
        self.app.audio_shutdown_flag = None;

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
        if self.app.browser.is_some() {
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

        let browser = if use_accelerated {
            self.create_accelerated_browser(
                &window_info,
                &browser_settings,
                context.as_mut(),
                dpi,
                pixel_width,
                pixel_height,
            )?
        } else {
            self.create_software_browser(
                &window_info,
                &browser_settings,
                context.as_mut(),
                dpi,
                pixel_width,
                pixel_height,
            )?
        };

        self.app.browser = Some(browser);
        self.last_size = logical_size;
        self.last_dpi = dpi;
        Ok(())
    }

    fn should_use_accelerated_osr(&self) -> bool {
        self.enable_accelerated_osr && accelerated_osr::is_accelerated_osr_supported()
    }

    fn create_software_browser(
        &mut self,
        _window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Result<cef::Browser, CefError> {
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
        let sample_rate = get_godot_audio_sample_rate();
        let enable_audio_capture = crate::settings::is_audio_capture_enabled();
        let queues = webrender::ClientQueues::new(sample_rate, enable_audio_capture);

        let texture = ImageTexture::new_gd();

        let mut client = webrender::SoftwareClientImpl::build(
            render_handler,
            webrender::ClientQueues {
                message_queue: queues.message_queue.clone(),
                binary_message_queue: queues.binary_message_queue.clone(),
                url_change_queue: queues.url_change_queue.clone(),
                title_change_queue: queues.title_change_queue.clone(),
                loading_state_queue: queues.loading_state_queue.clone(),
                ime_enable_queue: queues.ime_enable_queue.clone(),
                ime_composition_queue: queues.ime_composition_queue.clone(),
                console_message_queue: queues.console_message_queue.clone(),
                drag_event_queue: queues.drag_event_queue.clone(),
                audio_packet_queue: queues.audio_packet_queue.clone(),
                audio_params: queues.audio_params.clone(),
                audio_sample_rate: queues.audio_sample_rate.clone(),
                audio_shutdown_flag: queues.audio_shutdown_flag.clone(),
                enable_audio_capture,
                download_request_queue: queues.download_request_queue.clone(),
                download_update_queue: queues.download_update_queue.clone(),
            },
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
        self.app.render_mode = Some(RenderMode::Software {
            frame_buffer,
            texture,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);
        self.app.popup_state = Some(popup_state);
        self.app.message_queue = Some(queues.message_queue);
        self.app.binary_message_queue = Some(queues.binary_message_queue);
        self.app.url_change_queue = Some(queues.url_change_queue);
        self.app.title_change_queue = Some(queues.title_change_queue);
        self.app.loading_state_queue = Some(queues.loading_state_queue);
        self.app.ime_enable_queue = Some(queues.ime_enable_queue);
        self.app.ime_composition_range = Some(queues.ime_composition_queue);
        self.app.console_message_queue = Some(queues.console_message_queue);
        self.app.drag_event_queue = Some(queues.drag_event_queue);
        self.app.audio_packet_queue = Some(queues.audio_packet_queue);
        self.app.audio_params = Some(queues.audio_params);
        self.app.audio_sample_rate = Some(queues.audio_sample_rate);
        self.app.download_request_queue = Some(queues.download_request_queue);
        self.app.download_update_queue = Some(queues.download_update_queue);
        self.app.audio_shutdown_flag = Some(queues.audio_shutdown_flag);

        Ok(browser)
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
    ) -> Result<cef::Browser, CefError> {
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
        let sample_rate = get_godot_audio_sample_rate();
        let enable_audio_capture = crate::settings::is_audio_capture_enabled();
        let queues = webrender::ClientQueues::new(sample_rate, enable_audio_capture);

        let mut client = webrender::AcceleratedClientImpl::build(
            render_handler,
            cursor_type.clone(),
            webrender::ClientQueues {
                message_queue: queues.message_queue.clone(),
                binary_message_queue: queues.binary_message_queue.clone(),
                url_change_queue: queues.url_change_queue.clone(),
                title_change_queue: queues.title_change_queue.clone(),
                loading_state_queue: queues.loading_state_queue.clone(),
                ime_enable_queue: queues.ime_enable_queue.clone(),
                ime_composition_queue: queues.ime_composition_queue.clone(),
                console_message_queue: queues.console_message_queue.clone(),
                drag_event_queue: queues.drag_event_queue.clone(),
                audio_packet_queue: queues.audio_packet_queue.clone(),
                audio_params: queues.audio_params.clone(),
                audio_sample_rate: queues.audio_sample_rate.clone(),
                audio_shutdown_flag: queues.audio_shutdown_flag.clone(),
                enable_audio_capture,
                download_request_queue: queues.download_request_queue.clone(),
                download_update_queue: queues.download_update_queue.clone(),
            },
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
        self.app.render_mode = Some(RenderMode::Accelerated {
            render_state,
            texture_2d_rd,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);
        self.app.popup_state = Some(popup_state);
        self.app.message_queue = Some(queues.message_queue);
        self.app.binary_message_queue = Some(queues.binary_message_queue);
        self.app.url_change_queue = Some(queues.url_change_queue);
        self.app.title_change_queue = Some(queues.title_change_queue);
        self.app.loading_state_queue = Some(queues.loading_state_queue);
        self.app.ime_enable_queue = Some(queues.ime_enable_queue);
        self.app.ime_composition_range = Some(queues.ime_composition_queue);
        self.app.console_message_queue = Some(queues.console_message_queue);
        self.app.drag_event_queue = Some(queues.drag_event_queue);
        self.app.audio_packet_queue = Some(queues.audio_packet_queue);
        self.app.audio_params = Some(queues.audio_params);
        self.app.audio_sample_rate = Some(queues.audio_sample_rate);
        self.app.download_request_queue = Some(queues.download_request_queue);
        self.app.download_update_queue = Some(queues.download_update_queue);
        self.app.audio_shutdown_flag = Some(queues.audio_shutdown_flag);

        Ok(browser)
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
    ) -> Result<cef::Browser, CefError> {
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
