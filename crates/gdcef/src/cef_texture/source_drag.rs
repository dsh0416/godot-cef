use cef::{ImplBrowser, ImplBrowserHost};
use godot::classes::Input;
use godot::global::MouseButton;

use super::CefTexture;
use crate::browser::BrowserState;
use godot::prelude::*;

impl BrowserState {
    pub(crate) fn finish_source_drag(
        &self,
        session_id: Option<i64>,
        position: Option<(i32, i32)>,
        operation: i32,
    ) {
        let session = self
            .source_drag
            .lock()
            .ok()
            .and_then(|mut state| state.finish(session_id));
        // Never call CEF while holding the shared drag lock.
        if let Some(session) = session
            && let Some(host) = self.browser.host()
        {
            let (x, y) = position.unwrap_or(session.position);
            let operation = operation & session.allowed_ops as i32;
            let op = cef::DragOperationsMask::from(cef::sys::cef_drag_operations_mask_t(
                crate::cef_i32_to_raw!(operation),
            ));
            host.drag_source_ended_at(x, y, op);
            host.drag_source_system_drag_ended();
        }
    }
}

impl CefTexture {
    pub(super) fn source_drag_session_id(&self) -> Option<i64> {
        self.with_app(|app| {
            app.state.as_ref().and_then(|state| {
                state
                    .source_drag
                    .lock()
                    .ok()
                    .and_then(|drag| drag.active_id())
            })
        })
    }

    pub(super) fn finish_browser_drag(
        &mut self,
        session_id: Option<i64>,
        position: Option<Vector2>,
        operation: i32,
    ) {
        self.with_app(|app| {
            if let Some(state) = &app.state {
                state.finish_source_drag(
                    session_id,
                    position.map(|position| (position.x as i32, position.y as i32)),
                    operation,
                );
            }
        });
    }

    pub(super) fn cancel_browser_drag(&mut self) {
        self.with_app(|app| {
            if let Some(state) = &app.state {
                if let Ok(mut drag) = state.source_drag.lock() {
                    drag.enabled = false;
                }
                state.finish_source_drag(None, None, 0);
            }
        });
    }

    pub(super) fn refresh_browser_drag(&mut self) {
        let enabled = self.base().is_inside_tree()
            && self.base().is_visible_in_tree()
            && self.base().can_process()
            && self.base().is_processing()
            && !self
                .base()
                .get_signal_connection_list("drag_started")
                .is_empty();
        let released = !Input::singleton().is_mouse_button_pressed(MouseButton::LEFT);
        self.with_app(|app| {
            if let Some(state) = &app.state {
                if let Ok(mut drag) = state.source_drag.lock() {
                    drag.enabled = enabled;
                }
                // Runs after input dispatch, allowing a drop handler to finish first.
                if !enabled || released {
                    state.finish_source_drag(None, None, 0);
                }
            }
        });
    }
}
