//! Signal processing for CefTexture.
//!
//! This module handles draining event queues and emitting Godot signals.

use super::CefTexture;
use godot::prelude::*;

use crate::browser::{DragEvent, EventQueues, LoadingStateEvent};
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

/// Drained events from the consolidated event queue.
/// This allows us to release the lock before emitting signals.
#[derive(Default)]
pub(super) struct DrainedEvents {
    pub messages: Vec<String>,
    pub binary_messages: Vec<Vec<u8>>,
    pub url_changes: Vec<String>,
    pub title_changes: Vec<String>,
    pub loading_states: Vec<LoadingStateEvent>,
    pub ime_enables: Vec<bool>,
    pub ime_composition_range: Option<crate::browser::ImeCompositionRange>,
    pub console_messages: Vec<crate::browser::ConsoleMessageEvent>,
    pub drag_events: Vec<DragEvent>,
    pub download_requests: Vec<crate::browser::DownloadRequestEvent>,
    pub download_updates: Vec<crate::browser::DownloadUpdateEvent>,
}

impl DrainedEvents {
    /// Drains all events from the consolidated event queue in a single lock.
    pub fn drain_from(queues: &mut EventQueues) -> Self {
        Self {
            messages: queues.messages.drain(..).collect(),
            binary_messages: queues.binary_messages.drain(..).collect(),
            url_changes: queues.url_changes.drain(..).collect(),
            title_changes: queues.title_changes.drain(..).collect(),
            loading_states: queues.loading_states.drain(..).collect(),
            ime_enables: queues.ime_enables.drain(..).collect(),
            ime_composition_range: queues.ime_composition_range.take(),
            console_messages: queues.console_messages.drain(..).collect(),
            drag_events: queues.drag_events.drain(..).collect(),
            download_requests: queues.download_requests.drain(..).collect(),
            download_updates: queues.download_updates.drain(..).collect(),
        }
    }
}

impl CefTexture {
    /// Drains all event queues with a single lock and processes them.
    /// This is more efficient than locking each queue separately.
    pub(super) fn process_all_event_queues(&mut self) {
        let Some(event_queues) = &self.app.event_queues else {
            return;
        };

        // Drain all events with a single lock
        let events = {
            let Ok(mut queues) = event_queues.lock() else {
                return;
            };
            DrainedEvents::drain_from(&mut queues)
        };

        // Now process events without holding the lock
        self.emit_message_signals(&events.messages);
        self.emit_binary_message_signals(&events.binary_messages);
        self.emit_url_change_signals(&events.url_changes);
        self.emit_title_change_signals(&events.title_changes);
        self.emit_loading_state_signals(&events.loading_states);
        self.emit_console_message_signals(&events.console_messages);
        self.emit_drag_event_signals(&events.drag_events);
        self.emit_download_request_signals(&events.download_requests);
        self.emit_download_update_signals(&events.download_updates);

        // Handle IME events (these may modify self state)
        self.process_ime_enable_events(&events.ime_enables);
        if let Some(range) = events.ime_composition_range {
            self.process_ime_composition_event(range);
        }
    }

    fn emit_message_signals(&mut self, messages: &[String]) {
        for message in messages {
            self.base_mut()
                .emit_signal("ipc_message", &[GString::from(message).to_variant()]);
        }
    }

    fn emit_binary_message_signals(&mut self, messages: &[Vec<u8>]) {
        for data in messages {
            let byte_array = PackedByteArray::from(data.as_slice());
            self.base_mut()
                .emit_signal("ipc_binary_message", &[byte_array.to_variant()]);
        }
    }

    fn emit_url_change_signals(&mut self, urls: &[String]) {
        for url in urls {
            self.base_mut()
                .emit_signal("url_changed", &[GString::from(url).to_variant()]);
        }
    }

    fn emit_title_change_signals(&mut self, titles: &[String]) {
        for title in titles {
            self.base_mut()
                .emit_signal("title_changed", &[GString::from(title).to_variant()]);
        }
    }

    fn emit_loading_state_signals(&mut self, events: &[LoadingStateEvent]) {
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

    fn emit_console_message_signals(&mut self, events: &[crate::browser::ConsoleMessageEvent]) {
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

    fn emit_drag_event_signals(&mut self, events: &[DragEvent]) {
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

    fn emit_download_request_signals(&mut self, events: &[crate::browser::DownloadRequestEvent]) {
        for event in events {
            let download_info = DownloadRequestInfo::from_event(event);
            self.base_mut()
                .emit_signal("download_requested", &[download_info.to_variant()]);
        }
    }

    fn emit_download_update_signals(&mut self, events: &[crate::browser::DownloadUpdateEvent]) {
        for event in events {
            let download_info = DownloadUpdateInfo::from_event(event);
            self.base_mut()
                .emit_signal("download_updated", &[download_info.to_variant()]);
        }
    }

    fn process_ime_enable_events(&mut self, events: &[bool]) {
        // Take the last event (latest wins)
        if let Some(&enable) = events.last() {
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
