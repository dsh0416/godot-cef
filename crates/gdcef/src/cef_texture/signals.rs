//! Signal processing for CefTexture.
//!
//! This module handles draining event queues and emitting Godot signals.

use super::CefTexture;
use godot::prelude::*;

use crate::browser::{DragEvent, LoadingStateEvent};
use crate::drag::DragDataInfo;
use crate::queue_processing::drain_queue;

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

impl CefTexture {
    pub(super) fn process_message_queue(&mut self) {
        let Some(queue) = &self.app.message_queue else {
            return;
        };

        for message in drain_queue(queue) {
            self.base_mut()
                .emit_signal("ipc_message", &[GString::from(&message).to_variant()]);
        }
    }

    pub(super) fn process_url_change_queue(&mut self) {
        let Some(queue) = &self.app.url_change_queue else {
            return;
        };

        for url in drain_queue(queue) {
            self.base_mut()
                .emit_signal("url_changed", &[GString::from(&url).to_variant()]);
        }
    }

    pub(super) fn process_title_change_queue(&mut self) {
        let Some(queue) = &self.app.title_change_queue else {
            return;
        };

        for title in drain_queue(queue) {
            self.base_mut()
                .emit_signal("title_changed", &[GString::from(&title).to_variant()]);
        }
    }

    pub(super) fn process_loading_state_queue(&mut self) {
        let Some(queue) = &self.app.loading_state_queue else {
            return;
        };

        for event in drain_queue(queue) {
            match event {
                LoadingStateEvent::Started { url } => {
                    self.base_mut()
                        .emit_signal("load_started", &[GString::from(&url).to_variant()]);
                }
                LoadingStateEvent::Finished {
                    url,
                    http_status_code,
                } => {
                    self.base_mut().emit_signal(
                        "load_finished",
                        &[
                            GString::from(&url).to_variant(),
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
                            GString::from(&url).to_variant(),
                            error_code.to_variant(),
                            GString::from(&error_text).to_variant(),
                        ],
                    );
                }
            }
        }
    }

    pub(super) fn process_console_message_queue(&mut self) {
        let Some(queue) = &self.app.console_message_queue else {
            return;
        };

        for event in drain_queue(queue) {
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

    pub(super) fn process_drag_event_queue(&mut self) {
        let Some(queue) = &self.app.drag_event_queue else {
            return;
        };

        for event in drain_queue(queue) {
            match event {
                DragEvent::Started {
                    drag_data,
                    x,
                    y,
                    allowed_ops,
                } => {
                    let drag_info = DragDataInfo::from_internal(&drag_data);
                    let position = Vector2::new(x as f32, y as f32);
                    self.base_mut().emit_signal(
                        "drag_started",
                        &[
                            drag_info.to_variant(),
                            position.to_variant(),
                            (allowed_ops as i32).to_variant(),
                        ],
                    );
                    self.app.drag_state.is_dragging_from_browser = true;
                    self.app.drag_state.allowed_ops = allowed_ops;
                }
                DragEvent::UpdateCursor { operation } => {
                    self.base_mut()
                        .emit_signal("drag_cursor_updated", &[(operation as i32).to_variant()]);
                }
                DragEvent::Entered { drag_data, mask } => {
                    let drag_info = DragDataInfo::from_internal(&drag_data);
                    self.base_mut().emit_signal(
                        "drag_entered",
                        &[drag_info.to_variant(), (mask as i32).to_variant()],
                    );
                    self.app.drag_state.is_drag_over = true;
                }
            }
        }
    }

    pub(super) fn process_download_request_queue(&mut self) {
        let Some(queue) = &self.app.download_request_queue else {
            return;
        };

        for event in drain_queue(queue) {
            let download_info = DownloadRequestInfo::from_event(&event);
            self.base_mut()
                .emit_signal("download_requested", &[download_info.to_variant()]);
        }
    }

    pub(super) fn process_download_update_queue(&mut self) {
        let Some(queue) = &self.app.download_update_queue else {
            return;
        };

        for event in drain_queue(queue) {
            let download_info = DownloadUpdateInfo::from_event(&event);
            self.base_mut()
                .emit_signal("download_updated", &[download_info.to_variant()]);
        }
    }
}
