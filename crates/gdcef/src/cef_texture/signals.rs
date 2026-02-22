//! Signal processing for CefTexture.
//!
//! This module handles draining event queues and emitting Godot signals.

use super::CefTexture;
use godot::prelude::*;

use std::collections::VecDeque;

use crate::browser::{DragEvent, LoadingStateEvent};
use crate::drag::DragDataInfo;

macro_rules! emit_signal_variants {
    ($self:expr, $name:literal $(,)?) => {{
        $self.base_mut().emit_signal($name, &[]);
    }};
    ($self:expr, $name:literal, $($arg:expr),+ $(,)?) => {{
        let args = [$(($arg).to_variant()),+];
        $self.base_mut().emit_signal($name, &args);
    }};
}

macro_rules! godot_dto {
    ($name:ident { $($field:ident : $type:ty = $default:expr),* $(,)? }) => {
        #[derive(GodotClass)]
        #[class(base=RefCounted)]
        pub struct $name {
            base: Base<RefCounted>,
            $(
                #[var]
                pub $field: $type,
            )*
        }

        #[godot_api]
        impl IRefCounted for $name {
            fn init(base: Base<RefCounted>) -> Self {
                Self { base, $($field: $default,)* }
            }
        }
    };
}

godot_dto!(DownloadRequestInfo {
    id: u32 = 0,
    url: GString = GString::new(),
    original_url: GString = GString::new(),
    suggested_file_name: GString = GString::new(),
    mime_type: GString = GString::new(),
    total_bytes: i64 = -1,
});

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

godot_dto!(DownloadUpdateInfo {
    id: u32 = 0,
    url: GString = GString::new(),
    full_path: GString = GString::new(),
    received_bytes: i64 = 0,
    total_bytes: i64 = -1,
    current_speed: i64 = 0,
    percent_complete: i32 = -1,
    is_in_progress: bool = false,
    is_complete: bool = false,
    is_canceled: bool = false,
});

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

godot_dto!(CookieInfo {
    name: GString = GString::new(),
    value: GString = GString::new(),
    domain: GString = GString::new(),
    path: GString = GString::new(),
    secure: bool = false,
    httponly: bool = false,
    same_site: i32 = 0,
    has_expires: bool = false,
});

impl CookieInfo {
    fn from_data(data: &crate::cookie::CookieData) -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            name: GString::from(&data.name),
            value: GString::from(&data.value),
            domain: GString::from(&data.domain),
            path: GString::from(&data.path),
            secure: data.secure,
            httponly: data.httponly,
            same_site: crate::cef_raw_to_i32!(data.same_site.get_raw()),
            has_expires: data.has_expires,
        })
    }
}

impl CefTexture {
    /// Takes all queued events with a single lock and processes them.
    ///
    /// Uses `mem::take` to swap the entire `EventQueues` with an empty default,
    /// releasing the lock before any signal emission.
    pub(super) fn process_all_event_queues(&mut self) {
        let Some(event_queues) =
            self.with_app(|app| app.state.as_ref().map(|s| s.event_queues.clone()))
        else {
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
        self.emit_data_message_signals(&events.data_messages);
        self.emit_url_change_signals(&events.url_changes);
        self.emit_title_change_signals(&events.title_changes);
        self.emit_loading_state_signals(&events.loading_states);
        self.emit_console_message_signals(&events.console_messages);
        self.emit_drag_event_signals(&events.drag_events);
        self.emit_popup_request_signals(&events.popup_requests);
        self.emit_permission_request_signals(&events.permission_requests);
        self.emit_find_result_signals(&events.find_results);
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
            emit_signal_variants!(self, "ipc_message", GString::from(message));
        }
    }

    fn emit_binary_message_signals(&mut self, messages: &VecDeque<Vec<u8>>) {
        for data in messages {
            let byte_array = PackedByteArray::from(data.as_slice());
            emit_signal_variants!(self, "ipc_binary_message", byte_array);
        }
    }

    fn emit_data_message_signals(&mut self, messages: &VecDeque<Vec<u8>>) {
        for data in messages {
            match crate::ipc_data::decode_cbor_bytes_to_variant(data) {
                Ok(variant) => {
                    self.base_mut().emit_signal("ipc_data_message", &[variant]);
                }
                Err(err) => {
                    godot::global::godot_warn!(
                        "[CefTexture] Failed to decode IPC data message: {}",
                        err
                    );
                }
            }
        }
    }

    fn emit_url_change_signals(&mut self, urls: &VecDeque<String>) {
        for url in urls {
            emit_signal_variants!(self, "url_changed", GString::from(url));
        }
    }

    fn emit_title_change_signals(&mut self, titles: &VecDeque<String>) {
        for title in titles {
            emit_signal_variants!(self, "title_changed", GString::from(title));
        }
    }

    fn emit_loading_state_signals(&mut self, events: &VecDeque<LoadingStateEvent>) {
        for event in events {
            match event {
                LoadingStateEvent::Started { url } => {
                    emit_signal_variants!(self, "load_started", GString::from(url));
                }
                LoadingStateEvent::Finished {
                    url,
                    http_status_code,
                } => {
                    emit_signal_variants!(
                        self,
                        "load_finished",
                        GString::from(url),
                        http_status_code
                    );
                }
                LoadingStateEvent::Error {
                    url,
                    error_code,
                    error_text,
                } => {
                    emit_signal_variants!(
                        self,
                        "load_error",
                        GString::from(url),
                        error_code,
                        GString::from(error_text)
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
            emit_signal_variants!(
                self,
                "console_message",
                event.level,
                GString::from(&event.message),
                GString::from(&event.source),
                event.line
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
                    emit_signal_variants!(
                        self,
                        "drag_started",
                        drag_info,
                        position,
                        *allowed_ops as i32
                    );
                    self.with_app_mut(|app| {
                        app.drag_state.is_dragging_from_browser = true;
                        app.drag_state.allowed_ops = *allowed_ops;
                    });
                }
                DragEvent::UpdateCursor { operation } => {
                    emit_signal_variants!(self, "drag_cursor_updated", *operation as i32);
                }
                DragEvent::Entered { drag_data, mask } => {
                    let drag_info = DragDataInfo::from_internal(drag_data);
                    emit_signal_variants!(self, "drag_entered", drag_info, *mask as i32);
                    self.with_app_mut(|app| {
                        app.drag_state.is_drag_over = true;
                    });
                }
            }
        }
    }

    fn emit_popup_request_signals(&mut self, events: &VecDeque<crate::browser::PopupRequestEvent>) {
        for event in events {
            emit_signal_variants!(
                self,
                "popup_requested",
                GString::from(&event.target_url),
                event.disposition.get_raw(),
                event.user_gesture
            );
        }
    }

    fn emit_permission_request_signals(
        &mut self,
        events: &VecDeque<crate::browser::PermissionRequestEvent>,
    ) {
        for event in events {
            emit_signal_variants!(
                self,
                "permission_requested",
                GString::from(&event.permission_type),
                GString::from(&event.url),
                event.request_id
            );
        }
    }

    fn emit_find_result_signals(&mut self, events: &VecDeque<crate::browser::FindResultEvent>) {
        for event in events {
            emit_signal_variants!(
                self,
                "find_result",
                event.count,
                event.active_index,
                event.final_update
            );
        }
    }

    fn emit_cookie_event_signals(&mut self, events: &VecDeque<crate::cookie::CookieEvent>) {
        for event in events {
            match event {
                crate::cookie::CookieEvent::Received(cookies) => {
                    let array: Array<Gd<CookieInfo>> =
                        cookies.iter().map(CookieInfo::from_data).collect();
                    emit_signal_variants!(self, "cookies_received", array);
                }
                crate::cookie::CookieEvent::Set(success) => {
                    emit_signal_variants!(self, "cookie_set", success);
                }
                crate::cookie::CookieEvent::Deleted(count) => {
                    emit_signal_variants!(self, "cookies_deleted", count);
                }
                crate::cookie::CookieEvent::Flushed => {
                    emit_signal_variants!(self, "cookies_flushed");
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
            emit_signal_variants!(self, "download_requested", download_info);
        }
    }

    fn emit_download_update_signals(
        &mut self,
        events: &VecDeque<crate::browser::DownloadUpdateEvent>,
    ) {
        for event in events {
            let download_info = DownloadUpdateInfo::from_event(event);
            emit_signal_variants!(self, "download_updated", download_info);
        }
    }

    fn emit_render_process_terminated_signals(
        &mut self,
        events: &VecDeque<(String, cef::TerminationStatus)>,
    ) {
        for (reason, status) in events {
            emit_signal_variants!(
                self,
                "render_process_terminated",
                status.get_raw(),
                GString::from(reason)
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
