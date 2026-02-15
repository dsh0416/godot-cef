use adblock::lists::{FilterSet, ParseOptions};
use cef::{BrowserSettings, ImplBrowser, ImplBrowserHost, RequestContextSettings, WindowInfo};
use cef_app::PhysicalSize;
use godot::classes::Image;
use godot::classes::image::Format as ImageFormat;
use godot::classes::{AudioServer, DisplayServer, Engine, ImageTexture, Texture2Drd};
use godot::prelude::*;
use software_render::{DestBuffer, PopupBuffer, composite_popup};
use std::collections::HashMap;
use std::fs;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::accelerated_osr::{
    self, AcceleratedRenderState, GodotTextureImporter, PlatformAcceleratedRenderHandler,
};
use crate::browser::{App, BrowserState, PopupPolicyFlag, PopupStateQueue, RenderMode};
use crate::error::CefError;
use crate::utils::get_display_scale_factor;
use crate::{godot_protocol, render, webrender};

/// Shared browser creation inputs used by both `CefTexture` and `CefTexture2D`.
pub(crate) struct BackendCreateParams {
    pub logical_size: Vector2,
    pub dpi: f32,
    pub max_fps: i32,
    pub url: String,
    pub enable_accelerated_osr: bool,
    pub background_color: Color,
    pub popup_policy: i32,
    pub log_prefix: &'static str,
}

/// Bundles the pixel-level parameters shared by browser creation paths.
struct BrowserCreateParams {
    dpi: f32,
    pixel_width: i32,
    pixel_height: i32,
    popup_policy: PopupPolicyFlag,
    permission_policy: crate::browser::PermissionPolicyFlag,
    permission_request_counter: crate::browser::PermissionRequestIdCounter,
    pending_permission_requests: crate::browser::PendingPermissionRequests,
    pending_permission_aggregates: crate::browser::PendingPermissionAggregates,
}

fn color_to_cef_color(color: Color) -> u32 {
    let f = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
    u32::from_be_bytes([f(color.a), f(color.r), f(color.g), f(color.b)])
}

fn build_adblock_engine(log_prefix: &str) -> Option<Rc<adblock::Engine>> {
    if !crate::settings::is_adblock_enabled() {
        return None;
    }

    let Some(rules_path) = crate::settings::get_adblock_rules_path() else {
        godot::global::godot_warn!(
            "[{}] Adblock is enabled, but adblock rules path setting is empty. Request filtering will be disabled.",
            log_prefix
        );
        return None;
    };

    let rules = match fs::read_to_string(&rules_path) {
        Ok(content) => content,
        Err(error) => {
            godot::global::godot_warn!(
                "[{}] Failed to read adblock rules file '{}': {}. Request filtering will be disabled.",
                log_prefix,
                rules_path.display().to_string(),
                error
            );
            return None;
        }
    };

    let mut filter_set = FilterSet::new(true);
    let _metadata = filter_set.add_filter_list(&rules, ParseOptions::default());
    godot::global::godot_print!("[{}] Adblock filter list loaded.", log_prefix);
    Some(Rc::new(adblock::Engine::from_filter_set(filter_set, true)))
}

pub(crate) fn should_use_accelerated_osr(enable_accelerated_osr: bool, log_prefix: &str) -> bool {
    if !enable_accelerated_osr {
        godot::global::godot_print!(
            "[{}] Accelerated OSR disabled by `enable_accelerated_osr = false`; using software rendering",
            log_prefix
        );
        return false;
    }

    let (supported, reason) = accelerated_osr::accelerated_osr_support_diagnostic();
    if !supported {
        godot::global::godot_warn!(
            "[{}] Accelerated OSR unavailable: {}. Falling back to software rendering.",
            log_prefix,
            reason
        );
    }
    supported
}

pub(crate) fn get_max_fps() -> i32 {
    let setting_fps = crate::settings::get_max_frame_rate();
    if setting_fps > 0 {
        return setting_fps;
    }

    let engine_cap_fps = Engine::singleton().get_max_fps();
    let screen_cap_fps = DisplayServer::singleton().screen_get_refresh_rate().round() as i32;
    if engine_cap_fps > 0 {
        engine_cap_fps
    } else if screen_cap_fps > 0 {
        screen_cap_fps
    } else {
        60
    }
}

pub(crate) fn handle_max_fps_change(app: &App, last_max_fps: &mut i32, max_fps: i32) {
    if max_fps == *last_max_fps {
        return;
    }
    *last_max_fps = max_fps;
    if let Some(host) = app.host() {
        host.set_windowless_frame_rate(max_fps);
    }
}

pub(crate) fn handle_size_change(
    app: &App,
    last_size: &mut Vector2,
    last_dpi: &mut f32,
    logical_size: Vector2,
    current_dpi: f32,
) -> bool {
    if logical_size.x <= 0.0 || logical_size.y <= 0.0 {
        return false;
    }

    let size_diff = (logical_size - *last_size).abs();
    let dpi_diff = (current_dpi - *last_dpi).abs();
    if size_diff.x < 1e-6 && size_diff.y < 1e-6 && dpi_diff < 1e-6 {
        return false;
    }

    let pixel_width = logical_size.x * current_dpi;
    let pixel_height = logical_size.y * current_dpi;

    if let Some(state) = app.state.as_ref() {
        if let Ok(mut size) = state.render_size.lock() {
            size.width = pixel_width;
            size.height = pixel_height;
        }

        if let Ok(mut dpi) = state.device_scale_factor.lock() {
            *dpi = current_dpi;
        }
    }

    if let Some(host) = app.host() {
        host.notify_screen_info_changed();
        host.was_resized();
    }

    *last_size = logical_size;
    *last_dpi = current_dpi;
    true
}

pub(crate) fn request_external_begin_frame(app: &App) {
    if let Some(host) = app.host() {
        host.send_external_begin_frame();
    }
}

pub(crate) fn apply_popup_policy(app: &App, policy: i32) {
    if let Some(state) = app.state.as_ref() {
        state.popup_policy.store(policy, Ordering::Relaxed);
    }
}

/// Updates the primary browser texture for both software and accelerated modes.
///
/// Returns a replacement accelerated texture when a resize occurs. `CefTexture`
/// uses this to update its `TextureRect` texture binding immediately.
pub(crate) fn update_primary_texture(
    state: &mut BrowserState,
    log_prefix: &str,
) -> Option<Gd<Texture2Drd>> {
    if let RenderMode::Software {
        frame_buffer,
        texture,
    } = &mut state.render_mode
    {
        let Ok(mut fb) = frame_buffer.lock() else {
            return None;
        };

        let popup_metadata = state.popup_state.lock().ok().and_then(|popup| {
            if popup.visible && !popup.buffer.is_empty() {
                Some((
                    popup.width,
                    popup.height,
                    popup.rect.x,
                    popup.rect.y,
                    popup.dirty,
                ))
            } else {
                None
            }
        });

        let popup_dirty = popup_metadata
            .as_ref()
            .is_some_and(|(_, _, _, _, dirty)| *dirty);

        if !fb.dirty && !popup_dirty {
            return None;
        }

        if fb.data.is_empty() {
            return None;
        }

        let width = fb.width as i32;
        let height = fb.height as i32;
        let display_scale = get_display_scale_factor();

        let final_data =
            if let Some((popup_width, popup_height, popup_x, popup_y, _)) = popup_metadata {
                let popup_buffer = state
                    .popup_state
                    .lock()
                    .ok()
                    .map(|popup| popup.buffer.clone());

                if let Some(popup_buffer) = popup_buffer {
                    let mut composited = fb.data.clone();
                    let scaled_x = (popup_x as f32 * display_scale) as i32;
                    let scaled_y = (popup_y as f32 * display_scale) as i32;
                    composite_popup(
                        &mut DestBuffer {
                            data: &mut composited,
                            width: fb.width,
                            height: fb.height,
                        },
                        &PopupBuffer {
                            data: &popup_buffer,
                            width: popup_width,
                            height: popup_height,
                            x: scaled_x,
                            y: scaled_y,
                        },
                    );
                    if let Ok(mut popup) = state.popup_state.lock() {
                        popup.mark_clean();
                    }
                    composited
                } else {
                    fb.data.clone()
                }
            } else {
                fb.data.clone()
            };

        let byte_array = PackedByteArray::from(final_data.as_slice());
        let image: Option<Gd<Image>> =
            Image::create_from_data(width, height, false, ImageFormat::RGBA8, &byte_array);
        if let Some(image) = image {
            texture.set_image(&image);
        }

        fb.mark_clean();
        return None;
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if let RenderMode::Accelerated {
        render_state,
        texture_2d_rd,
    } = &mut state.render_mode
    {
        let Ok(mut accel_state) = render_state.lock() else {
            return None;
        };

        let texture_to_set = if let Some((new_w, new_h)) = accel_state.needs_resize.take()
            && new_w > 0
            && new_h > 0
        {
            render::free_rd_texture(accel_state.dst_rd_rid);

            let (new_rd_rid, new_texture_2d_rd) =
                match render::create_rd_texture(new_w as i32, new_h as i32) {
                    Ok(result) => result,
                    Err(e) => {
                        godot::global::godot_error!("[{}] {}", log_prefix, e);
                        return None;
                    }
                };

            accel_state.dst_rd_rid = new_rd_rid;
            accel_state.dst_width = new_w;
            accel_state.dst_height = new_h;

            *texture_2d_rd = new_texture_2d_rd.clone();
            Some(new_texture_2d_rd)
        } else {
            None
        };

        if accel_state.has_pending_copy
            && let Err(e) = accel_state.process_pending_copy()
        {
            godot::global::godot_error!("[{}] Failed to process pending copy: {}", log_prefix, e);
        }

        return texture_to_set;
    }

    None
}

pub(crate) fn try_create_browser(
    app: &mut App,
    params: &BackendCreateParams,
) -> Result<(), CefError> {
    if app.state.is_some() {
        return Ok(());
    }

    if params.logical_size.x <= 0.0 || params.logical_size.y <= 0.0 {
        return Err(CefError::InvalidSize {
            width: params.logical_size.x,
            height: params.logical_size.y,
        });
    }

    let pixel_width = (params.logical_size.x * params.dpi) as i32;
    let pixel_height = (params.logical_size.y * params.dpi) as i32;
    let use_accelerated =
        should_use_accelerated_osr(params.enable_accelerated_osr, params.log_prefix);

    let popup_policy: PopupPolicyFlag = Arc::new(AtomicI32::new(params.popup_policy));
    let default_permission_policy = crate::settings::get_default_permission_policy();
    let permission_policy: crate::browser::PermissionPolicyFlag =
        Arc::new(AtomicI32::new(default_permission_policy));
    let permission_request_counter: crate::browser::PermissionRequestIdCounter =
        Arc::new(AtomicI64::new(0));
    let pending_permission_requests: crate::browser::PendingPermissionRequests =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_permission_aggregates: crate::browser::PendingPermissionAggregates =
        Arc::new(Mutex::new(HashMap::new()));

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
        windowless_frame_rate: params.max_fps,
        background_color: color_to_cef_color(params.background_color),
        ..Default::default()
    };

    let adblock_engine = build_adblock_engine(params.log_prefix);
    let mut context = cef::request_context_create_context(
        Some(&RequestContextSettings::default()),
        Some(&mut webrender::RequestContextHandlerImpl::build(
            webrender::OsrRequestContextHandler::new(adblock_engine),
        )),
    );
    if let Some(ctx) = context.as_mut() {
        godot_protocol::register_res_scheme_handler_on_context(ctx);
        godot_protocol::register_user_scheme_handler_on_context(ctx);
    }

    let create_params = BrowserCreateParams {
        dpi: params.dpi,
        pixel_width,
        pixel_height,
        popup_policy,
        permission_policy,
        permission_request_counter,
        pending_permission_requests,
        pending_permission_aggregates,
    };

    if use_accelerated {
        create_accelerated_browser(
            app,
            &window_info,
            &browser_settings,
            context.as_mut(),
            create_params,
            &params.url,
            params.log_prefix,
        )?;
    } else {
        create_software_browser(
            app,
            &window_info,
            &browser_settings,
            context.as_mut(),
            create_params,
            &params.url,
            params.log_prefix,
        )?;
    }

    Ok(())
}

pub(crate) fn cleanup_runtime(app: &mut App, popup_texture_2d_rd: Option<&mut Gd<Texture2Drd>>) {
    if app.state.is_none() {
        app.release_cef_if_retained();
        return;
    }

    if let Some(state) = &app.state
        && let Ok(mut pending) = state.pending_permission_requests.lock()
    {
        pending.clear();
    }
    if let Some(state) = &app.state
        && let Ok(mut pending) = state.pending_permission_aggregates.lock()
    {
        pending.clear();
    }

    if let Some(state) = &app.state
        && let Some(audio) = &state.audio
    {
        audio.shutdown_flag.store(true, Ordering::Relaxed);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if let Some(state) = &mut app.state
        && let RenderMode::Accelerated {
            render_state,
            texture_2d_rd,
        } = &mut state.render_mode
    {
        texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
        if let Some(popup_texture_2d_rd) = popup_texture_2d_rd {
            popup_texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
        }
        if let Ok(mut rs) = render_state.lock() {
            render::free_rd_texture(rs.dst_rd_rid);
            if let Some(popup_rid) = rs.popup_rd_rid.take() {
                render::free_rd_texture(popup_rid);
            }
        }
    }

    if let Some(state) = app.state.take()
        && let Some(host) = state.browser.host()
    {
        host.close_browser(true as _);
    }

    app.clear_runtime_state();
    app.release_cef_if_retained();
}

fn create_software_browser(
    app: &mut App,
    _window_info: &WindowInfo,
    browser_settings: &BrowserSettings,
    context: Option<&mut cef::RequestContext>,
    params: BrowserCreateParams,
    url: &str,
    log_prefix: &str,
) -> Result<(), CefError> {
    let BrowserCreateParams {
        dpi,
        pixel_width,
        pixel_height,
        popup_policy,
        permission_policy,
        permission_request_counter,
        pending_permission_requests,
        pending_permission_aggregates,
    } = params;
    godot::global::godot_print!(
        "[{}] Creating browser in software rendering mode",
        log_prefix
    );
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
    let queues = webrender::ClientQueues::new(
        sample_rate,
        enable_audio_capture,
        permission_policy.clone(),
        permission_request_counter.clone(),
        pending_permission_requests.clone(),
        pending_permission_aggregates.clone(),
    );

    let texture = ImageTexture::new_gd();
    let cef_render_handler =
        webrender::SoftwareOsrHandler::build(render_handler, queues.event_queues.clone());
    let mut client = webrender::CefClientImpl::build(
        cef_render_handler,
        cursor_type.clone(),
        queues.clone(),
        popup_policy.clone(),
    );

    let browser = cef::browser_host_create_browser_sync(
        Some(&window_info),
        Some(&mut client),
        Some(&url.into()),
        Some(browser_settings),
        None,
        context,
    )
    .ok_or_else(|| {
        CefError::BrowserCreationFailed("browser_host_create_browser_sync returned None".into())
    })?;

    let event_queues = queues.event_queues.clone();
    app.state = Some(BrowserState {
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
        popup_policy,
        pending_permission_requests,
        pending_permission_aggregates,
    });

    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn create_accelerated_browser(
    app: &mut App,
    window_info: &WindowInfo,
    browser_settings: &BrowserSettings,
    context: Option<&mut cef::RequestContext>,
    params: BrowserCreateParams,
    url: &str,
    log_prefix: &str,
) -> Result<(), CefError> {
    godot::global::godot_print!(
        "[{}] Creating browser in accelerated rendering mode",
        log_prefix
    );
    let importer = match GodotTextureImporter::new() {
        Some(imp) => imp,
        None => {
            godot::global::godot_warn!(
                "[{}] Failed to create GPU texture importer, falling back to software rendering",
                log_prefix
            );
            return create_software_browser(
                app,
                window_info,
                browser_settings,
                context,
                params,
                url,
                log_prefix,
            );
        }
    };
    let BrowserCreateParams {
        dpi,
        pixel_width,
        pixel_height,
        popup_policy,
        permission_policy,
        permission_request_counter,
        pending_permission_requests,
        pending_permission_aggregates,
    } = params;

    let (rd_texture_rid, texture_2d_rd) = render::create_rd_texture(pixel_width, pixel_height)?;
    let render_state = Arc::new(Mutex::new(AcceleratedRenderState::new(
        importer,
        rd_texture_rid,
        pixel_width as u32,
        pixel_height as u32,
    )));

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
    let queues = webrender::ClientQueues::new(
        sample_rate,
        enable_audio_capture,
        permission_policy.clone(),
        permission_request_counter.clone(),
        pending_permission_requests.clone(),
        pending_permission_aggregates.clone(),
    );

    let cef_render_handler =
        webrender::AcceleratedOsrHandler::build(render_handler, queues.event_queues.clone());
    let mut client = webrender::CefClientImpl::build(
        cef_render_handler,
        cursor_type.clone(),
        queues.clone(),
        popup_policy.clone(),
    );

    let browser = match cef::browser_host_create_browser_sync(
        Some(window_info),
        Some(&mut client),
        Some(&url.into()),
        Some(browser_settings),
        None,
        context,
    ) {
        Some(browser) => browser,
        None => {
            render::free_rd_texture(rd_texture_rid);
            return Err(CefError::BrowserCreationFailed(
                "browser_host_create_browser_sync returned None (accelerated)".into(),
            ));
        }
    };

    let event_queues = queues.event_queues.clone();
    app.state = Some(BrowserState {
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
        popup_policy,
        pending_permission_requests,
        pending_permission_aggregates,
    });
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn create_accelerated_browser(
    app: &mut App,
    window_info: &WindowInfo,
    browser_settings: &BrowserSettings,
    context: Option<&mut cef::RequestContext>,
    params: BrowserCreateParams,
    url: &str,
    log_prefix: &str,
) -> Result<(), CefError> {
    create_software_browser(
        app,
        window_info,
        browser_settings,
        context,
        params,
        url,
        log_prefix,
    )
}

#[cfg(test)]
mod tests {
    use super::color_to_cef_color;
    use godot::prelude::Color;

    #[test]
    fn color_to_cef_color_packs_argb_in_big_endian_order() {
        let color = Color::from_rgba(0.1, 0.2, 0.3, 0.4);

        // 0.1,0.2,0.3,0.4 map to 25,51,76,102 with truncating conversion.
        assert_eq!(
            color_to_cef_color(color),
            u32::from_be_bytes([102, 25, 51, 76])
        );
    }

    #[test]
    fn color_to_cef_color_clamps_components_to_zero_through_one() {
        let color = Color::from_rgba(-1.0, 2.0, 0.25, 1.5);

        assert_eq!(
            color_to_cef_color(color),
            u32::from_be_bytes([255, 0, 255, 63])
        );
    }
}
