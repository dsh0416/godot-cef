use super::CefTexture;
use crate::dialogs::DialogController;
use godot::prelude::*;

impl CefTexture {
    pub(super) fn dialog_controller(&self) -> Option<DialogController> {
        self.with_app(|app| app.state.as_ref().map(|state| state.dialogs.clone()))
    }
}

#[godot_api(secondary)]
impl CefTexture {
    #[func]
    pub fn continue_file_dialog(&self, request_id: i64, file_paths: PackedStringArray) -> bool {
        self.dialog_controller().is_some_and(|controller| {
            controller.resolve_file(
                request_id,
                file_paths
                    .as_slice()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            )
        })
    }

    #[func]
    pub fn cancel_file_dialog(&self, request_id: i64) -> bool {
        self.dialog_controller()
            .is_some_and(|controller| controller.cancel_file(request_id))
    }

    #[func]
    pub fn continue_js_dialog(&self, request_id: i64, accepted: bool, user_input: GString) -> bool {
        self.dialog_controller().is_some_and(|controller| {
            controller.resolve_js(request_id, accepted, user_input.to_string())
        })
    }

    #[func]
    pub fn cancel_js_dialog(&self, request_id: i64) -> bool {
        self.dialog_controller()
            .is_some_and(|controller| controller.cancel_js(request_id))
    }

    #[func]
    pub fn is_dialog_pending(&self, request_id: i64) -> bool {
        self.dialog_controller()
            .is_some_and(|controller| controller.is_pending(request_id))
    }
}
