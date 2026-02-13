//! Signal processing for CefTexture.
//!
//! This module handles draining event queues and emitting Godot signals.

use super::CefTexture;
use godot::prelude::*;

use std::collections::VecDeque;

use crate::browser::{DragEvent, LoadingStateEvent};
use crate::drag::DragDataInfo;

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct DownloadRequestInfo {
    base: Base<RefCounted>,

    #[var]
    pub id: u32,

    #[var]
    pub url: GString,

    #[var]
    pub original_url: GString,

    #[var]
    pub suggested_file_name: GString,

    #[var]
    pub mime_type: GString,

    #[var]
    pub total_bytes: i64,
}

#[godot_api]
impl IRefCounted for DownloadRequestInfo {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            id: 0,
            url: GString::new(),
            original_url: GString::new(),
            suggested_file_name: GString::new(),
            mime_type: GString::new(),
            total_bytes: -1,
        }
    }
}

impl DownloadRequestInfo {
    fn from_event(event: &crate::browser::DownloadRequestEvent) -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            id: event.id,
            url: GString::from(&event.url),
            original_url: GString::from(&event.original_url),
            suggested_file_name: GString::from(&event.suggested_file_name),
            mime_type: GString::from(&event.mime_type),
            total_bytes: event.total_bytes,
        })
    }
}

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct DownloadUpdateInfo {
    base: Base<RefCounted>,

    #[var]
    pub id: u32,

    #[var]
    pub url: GString,

    #[var]
    pub full_path: GString,

    #[var]
    pub received_bytes: i64,

    #[var]
    pub total_bytes: i64,

    #[var]
    pub current_speed: i64,

    #[var]
    pub percent_complete: i32,

    #[var]
    pub is_in_progress: bool,

    #[var]
    pub is_complete: bool,

    #[var]
    pub is_canceled: bool,
}

#[godot_api]
impl IRefCounted for DownloadUpdateInfo {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            id: 0,
            url: GString::new(),
            full_path: GString::new(),
            received_bytes: 0,
            total_bytes: -1,
            current_speed: 0,
            percent_complete: -1,
            is_in_progress: false,
            is_complete: false,
            is_canceled: false,
        }
    }
}

impl DownloadUpdateInfo {
    fn from_event(event: &crate::browser::DownloadUpdateEvent) -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            id: event.id,
            url: GString::from(&event.url),
            full_path: GString::from(&event.full_path),
            received_bytes: event.received_bytes,
            total_bytes: event.total_bytes,
            current_speed: event.current_speed,
            percent_complete: event.percent_complete,
            is_in_progress: event.is_in_progress,
            is_complete: event.is_complete,
            is_canceled: event.is_canceled,
        })
    }
}

/// Cookie information exposed to GDScript as a `RefCounted` object.
///
/// Properties: `name`, `value`, `domain`, `path`, `secure`, `httponly`,
/// `same_site`, `has_expires`.
#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct CookieInfo {
    base: Base<RefCounted>,

    #[var]
    pub name: GString,

    #[var]
    pub value: GString,

    #[var]
    pub domain: GString,

    #[var]
    pub path: GString,

    #[var]
    pub secure: bool,

    #[var]
    pub httponly: bool,

    /// SameSite policy (0 = Unspecified, 1 = None, 2 = Lax, 3 = Strict).
    #[var]
    pub same_site: i32,

    #[var]
    pub has_expires: bool,
}

#[godot_api]
impl IRefCounted for CookieInfo {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            name: GString::new(),
            value: GString::new(),
            domain: GString::new(),
            path: GString::new(),
            secure: false,
            httponly: false,
            same_site: 0,
            has_expires: false,
        }
    }
}

impl CookieInfo {
    fn from_data(data: &crate::cookie::CookieData) -> Gd<Self> {
        #[cfg(target_os = "windows")]
        return Gd::from_init_fn(|base| Self {
            base,
            name: GString::from(&data.name),
            value: GString::from(&data.value),
            domain: GString::from(&data.domain),
            path: GString::from(&data.path),
            secure: data.secure,
            httponly: data.httponly,
            same_site: data.same_site.get_raw(),
            has_expires: data.has_expires,
        });
        #[cfg(not(target_os = "windows"))]
        return Gd::from_init_fn(|base| Self {
            base,
            name: GString::from(&data.name),
            value: GString::from(&data.value),
            domain: GString::from(&data.domain),
            path: GString::from(&data.path),
            secure: data.secure,
            httponly: data.httponly,
            same_site: data.same_site.get_raw() as i32,
            has_expires: data.has_expires,
        });
    }
}

impl CefTexture {
    /// Takes all queued events with a single lock and processes them.
    ///
    /// Uses `mem::take` to swap the entire `EventQueues` with an empty default,
    /// releasing the lock before any signal emission.
    pub(super) fn process_all_event_queues(&mut self) {
        let Some(event_queues) = self.app.state.as_ref().map(|s| &s.event_queues) else {
            return;
        };

        // Take all events with a single lock, replacing with empty queues
        let events = {
            let Ok(mut queues) = event_queues.lock() else {
                godot::global::godot_warn!(
                    "[CefTexture] Failed to lock event queues while draining signals"
                );
                return;
            };
            std::mem::take(&mut *queues)
        };

        // Now process events without holding the lock
        self.emit_message_signals(&events.messages);
        self.emit_binary_message_signals(&events.binary_messages);
        self.emit_url_change_signals(&events.url_changes);
        self.emit_title_change_signals(&events.title_changes);
        self.emit_loading_state_signals(&events.loading_states);
        self.emit_console_message_signals(&events.console_messages);
        self.emit_drag_event_signals(&events.drag_events);
        self.emit_popup_request_signals(&events.popup_requests);
        self.emit_cookie_event_signals(&events.cookie_events);
        self.emit_download_request_signals(&events.download_requests);
        self.emit_download_update_signals(&events.download_updates);
        self.emit_render_process_terminated_signals(&events.render_process_terminated);

        // Handle IME events (these may modify self state)
        self.process_ime_enable_events(&events.ime_enables);
        if let Some(range) = events.ime_composition_range {
            self.process_ime_composition_event(range);
        }
    }

    fn emit_message_signals(&mut self, messages: &VecDeque<String>) {
        for message in messages {
            self.base_mut()
                .emit_signal("ipc_message", &[GString::from(message).to_variant()]);
        }
    }

    fn emit_binary_message_signals(&mut self, messages: &VecDeque<Vec<u8>>) {
        for data in messages {
            let byte_array = PackedByteArray::from(data.as_slice());
            self.base_mut()
                .emit_signal("ipc_binary_message", &[byte_array.to_variant()]);
        }
    }

    fn emit_url_change_signals(&mut self, urls: &VecDeque<String>) {
        for url in urls {
            self.base_mut()
                .emit_signal("url_changed", &[GString::from(url).to_variant()]);
        }
    }

    fn emit_title_change_signals(&mut self, titles: &VecDeque<String>) {
        for title in titles {
            self.base_mut()
                .emit_signal("title_changed", &[GString::from(title).to_variant()]);
        }
    }

    fn emit_loading_state_signals(&mut self, events: &VecDeque<LoadingStateEvent>) {
        for event in events {
            match event {
                LoadingStateEvent::Started { url } => {
                    self.base_mut()
                        .emit_signal("load_started", &[GString::from(url).to_variant()]);
                }
                LoadingStateEvent::Finished {
                    url,
                    http_status_code,
                } => {
                    self.base_mut().emit_signal(
                        "load_finished",
                        &[
                            GString::from(url).to_variant(),
                            http_status_code.to_variant(),
                        ],
                    );
                }
                LoadingStateEvent::Error {
                    url,
                    error_code,
                    error_text,
                } => {
                    self.base_mut().emit_signal(
                        "load_error",
                        &[
                            GString::from(url).to_variant(),
                            error_code.to_variant(),
                            GString::from(error_text).to_variant(),
                        ],
                    );
                }
            }
        }
    }

    fn emit_console_message_signals(
        &mut self,
        events: &VecDeque<crate::browser::ConsoleMessageEvent>,
    ) {
        for event in events {
            self.base_mut().emit_signal(
                "console_message",
                &[
                    event.level.to_variant(),
                    GString::from(&event.message).to_variant(),
                    GString::from(&event.source).to_variant(),
                    event.line.to_variant(),
                ],
            );
        }
    }

    fn emit_drag_event_signals(&mut self, events: &VecDeque<DragEvent>) {
        for event in events {
            match event {
                DragEvent::Started {
                    drag_data,
                    x,
                    y,
                    allowed_ops,
                } => {
                    let drag_info = DragDataInfo::from_internal(drag_data);
                    let position = Vector2::new(*x as f32, *y as f32);
                    self.base_mut().emit_signal(
                        "drag_started",
                        &[
                            drag_info.to_variant(),
                            position.to_variant(),
                            (*allowed_ops as i32).to_variant(),
                        ],
                    );
                    self.app.drag_state.is_dragging_from_browser = true;
                    self.app.drag_state.allowed_ops = *allowed_ops;
                }
                DragEvent::UpdateCursor { operation } => {
                    self.base_mut()
                        .emit_signal("drag_cursor_updated", &[(*operation as i32).to_variant()]);
                }
                DragEvent::Entered { drag_data, mask } => {
                    let drag_info = DragDataInfo::from_internal(drag_data);
                    self.base_mut().emit_signal(
                        "drag_entered",
                        &[drag_info.to_variant(), (*mask as i32).to_variant()],
                    );
                    self.app.drag_state.is_drag_over = true;
                }
            }
        }
    }

    fn emit_popup_request_signals(&mut self, events: &VecDeque<crate::browser::PopupRequestEvent>) {
        for event in events {
            self.base_mut().emit_signal(
                "popup_requested",
                &[
                    GString::from(&event.target_url).to_variant(),
                    event.disposition.get_raw().to_variant(),
                    event.user_gesture.to_variant(),
                ],
            );
        }
    }

    fn emit_cookie_event_signals(&mut self, events: &VecDeque<crate::cookie::CookieEvent>) {
        for event in events {
            match event {
                crate::cookie::CookieEvent::Received(cookies) => {
                    let array: Array<Gd<CookieInfo>> =
                        cookies.iter().map(CookieInfo::from_data).collect();
                    self.base_mut()
                        .emit_signal("cookies_received", &[array.to_variant()]);
                }
                crate::cookie::CookieEvent::Set(success) => {
                    self.base_mut()
                        .emit_signal("cookie_set", &[success.to_variant()]);
                }
                crate::cookie::CookieEvent::Deleted(count) => {
                    self.base_mut()
                        .emit_signal("cookies_deleted", &[count.to_variant()]);
                }
                crate::cookie::CookieEvent::Flushed => {
                    self.base_mut().emit_signal("cookies_flushed", &[]);
                }
            }
        }
    }

    fn emit_download_request_signals(
        &mut self,
        events: &VecDeque<crate::browser::DownloadRequestEvent>,
    ) {
        for event in events {
            let download_info = DownloadRequestInfo::from_event(event);
            self.base_mut()
                .emit_signal("download_requested", &[download_info.to_variant()]);
        }
    }

    fn emit_download_update_signals(
        &mut self,
        events: &VecDeque<crate::browser::DownloadUpdateEvent>,
    ) {
        for event in events {
            let download_info = DownloadUpdateInfo::from_event(event);
            self.base_mut()
                .emit_signal("download_updated", &[download_info.to_variant()]);
        }
    }

    fn emit_render_process_terminated_signals(
        &mut self,
        events: &VecDeque<(String, cef::TerminationStatus)>,
    ) {
        for (reason, status) in events {
            self.base_mut().emit_signal(
                "render_process_terminated",
                &[
                    status.get_raw().to_variant(),
                    GString::from(reason).to_variant(),
                ],
            );
        }
    }

    fn process_ime_enable_events(&mut self, events: &VecDeque<bool>) {
        // Take the last event (latest wins)
        if let Some(&enable) = events.back() {
            if enable && !self.ime_active {
                self.activate_ime();
            } else if !enable && self.ime_active {
                self.deactivate_ime();
            }
        }
    }

    fn process_ime_composition_event(&mut self, range: crate::browser::ImeCompositionRange) {
        if self.ime_active {
            // Directly assign to ime_position field instead of using setter
            // to avoid conflict with GodotClass-generated setter
            self.ime_position = Vector2i::new(range.caret_x, range.caret_y + range.caret_height);
            self.process_ime_position();
        }
    }
}
