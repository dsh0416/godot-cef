//! Signal conversion shared by the Control and Texture2D browser owners.

use std::collections::VecDeque;

use godot::prelude::*;

use crate::browser::DisplayEvent;

/// Called after releasing the CEF event queue lock on the Godot thread.
pub(crate) fn emit_events(events: &VecDeque<DisplayEvent>, mut emit: impl FnMut(&str, &[Variant])) {
    for event in events {
        match event {
            DisplayEvent::Tooltip(text) => {
                emit("tooltip_changed", &[GString::from(text).to_variant()]);
            }
            DisplayEvent::Fullscreen(fullscreen) => {
                emit("fullscreen_changed", &[fullscreen.to_variant()]);
            }
            DisplayEvent::LoadingProgress(progress) => {
                emit("loading_progress_changed", &[progress.to_variant()]);
            }
            DisplayEvent::StatusMessage(message) => {
                emit(
                    "status_message_changed",
                    &[GString::from(message).to_variant()],
                );
            }
            DisplayEvent::FaviconUrls(urls) => {
                let urls: PackedStringArray = urls.iter().map(GString::from).collect();
                emit("favicon_urls_changed", &[urls.to_variant()]);
            }
        }
    }
}
