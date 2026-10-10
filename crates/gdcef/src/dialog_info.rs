//! Godot snapshots of dialog metadata; CEF callback objects never cross the API.

use godot::prelude::*;

use crate::browser::{EventQueues, FileDialogRequestEvent, JsDialogRequestEvent};
use crate::dialogs::DialogController;

macro_rules! dialog_dto {
    ($name:ident { $($field:ident: $ty:ty = $default:expr),* $(,)? }) => {
        #[derive(GodotClass)]
        #[class(base=RefCounted)]
        pub struct $name {
            base: Base<RefCounted>,
            $(#[var] pub $field: $ty,)*
        }

        #[godot_api]
        impl IRefCounted for $name {
            fn init(base: Base<RefCounted>) -> Self {
                Self { base, $($field: $default,)* }
            }
        }
    };
}

dialog_dto!(FileDialogRequestInfo {
    request_id: i64 = 0,
    mode: i32 = 0,
    title: GString = GString::new(),
    default_file_path: GString = GString::new(),
    accept_filters: PackedStringArray = PackedStringArray::new(),
    accept_extensions: PackedStringArray = PackedStringArray::new(),
    accept_descriptions: PackedStringArray = PackedStringArray::new(),
});

dialog_dto!(JsDialogRequestInfo {
    request_id: i64 = 0,
    kind: GString = GString::new(),
    origin_url: GString = GString::new(),
    message_text: GString = GString::new(),
    default_prompt_text: GString = GString::new(),
    is_reload: bool = false,
});

fn strings(values: &[String]) -> PackedStringArray {
    values.iter().map(GString::from).collect()
}

impl FileDialogRequestInfo {
    fn from_event(event: &FileDialogRequestEvent) -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            request_id: event.request_id,
            mode: event.mode,
            title: GString::from(&event.title),
            default_file_path: GString::from(&event.default_file_path),
            accept_filters: strings(&event.accept_filters),
            accept_extensions: strings(&event.accept_extensions),
            accept_descriptions: strings(&event.accept_descriptions),
        })
    }
}

impl JsDialogRequestInfo {
    fn from_event(event: &JsDialogRequestEvent) -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            request_id: event.request_id,
            kind: GString::from(&event.kind),
            origin_url: GString::from(&event.origin_url),
            message_text: GString::from(&event.message_text),
            default_prompt_text: GString::from(&event.default_prompt_text),
            is_reload: event.is_reload,
        })
    }
}

/// Called by either browser owner after releasing the event queue lock.
/// `emit` uses the owner's BaseMut guard so signal handlers can respond immediately.
pub(crate) fn emit_events(
    controller: Option<&DialogController>,
    events: &EventQueues,
    mut emit: impl FnMut(&str, &[Variant]) -> bool,
) {
    for (request_id, result) in &events.dialog_request_finished {
        let _ = emit(
            "dialog_request_finished",
            &[request_id.to_variant(), GString::from(result).to_variant()],
        );
    }
    let Some(controller) = controller else {
        return;
    };
    for event in &events.file_dialog_requests {
        if controller.is_pending(event.request_id)
            && !emit(
                "file_dialog_requested",
                &[FileDialogRequestInfo::from_event(event).to_variant()],
            )
        {
            controller.dismiss_request(event.request_id, "unhandled");
        }
    }
    for event in &events.js_dialog_requests {
        if controller.is_pending(event.request_id)
            && !emit(
                "js_dialog_requested",
                &[JsDialogRequestInfo::from_event(event).to_variant()],
            )
        {
            controller.dismiss_request(event.request_id, "unhandled");
        }
    }
}
