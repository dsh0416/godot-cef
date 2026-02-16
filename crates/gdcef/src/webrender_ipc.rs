use cef::{CefStringUtf16, ImplBinaryValue, ImplListValue, ImplProcessMessage, ProcessMessage};
use cef_app::ipc_contract::{
    ROUTE_IME_CARET_POSITION, ROUTE_IPC_BINARY_RENDERER_TO_GODOT, ROUTE_IPC_DATA_RENDERER_TO_GODOT,
    ROUTE_IPC_RENDERER_TO_GODOT, ROUTE_TRIGGER_IME,
};

use crate::browser::ImeCompositionRange;
use crate::webrender::ClientIpcQueues;

pub(crate) fn on_process_message_received(
    message: Option<&mut ProcessMessage>,
    ipc: &ClientIpcQueues,
) -> i32 {
    let Some(message) = message else { return 0 };
    let route = CefStringUtf16::from(&message.name()).to_string();

    match route.as_str() {
        ROUTE_IPC_RENDERER_TO_GODOT => {
            if let Some(args) = message.argument_list() {
                let arg = args.string(0);
                let msg_str = CefStringUtf16::from(&arg).to_string();
                if let Ok(mut queues) = ipc.event_queues.lock() {
                    queues.messages.push_back(msg_str);
                }
            }
        }
        ROUTE_IPC_BINARY_RENDERER_TO_GODOT => {
            if let Some(args) = message.argument_list()
                && let Some(binary_value) = args.binary(0)
            {
                let size = binary_value.size();
                if size > 0 {
                    let mut buffer = vec![0u8; size];
                    let copied = binary_value.data(Some(&mut buffer), 0);
                    if copied > 0 {
                        buffer.truncate(copied);
                        if let Ok(mut queues) = ipc.event_queues.lock() {
                            queues.binary_messages.push_back(buffer);
                        }
                    }
                }
            }
        }
        ROUTE_IPC_DATA_RENDERER_TO_GODOT => {
            if let Some(args) = message.argument_list()
                && let Some(binary_value) = args.binary(0)
            {
                let size = binary_value.size();
                if size > crate::ipc_data::max_ipc_data_bytes() {
                    godot::global::godot_warn!(
                        "[CefTexture] Dropping IPC data message larger than limit: {} bytes",
                        size
                    );
                    return 0;
                }

                if size > 0 {
                    let mut buffer = vec![0u8; size];
                    let copied = binary_value.data(Some(&mut buffer), 0);
                    if copied > 0 {
                        buffer.truncate(copied);
                        if let Ok(mut queues) = ipc.event_queues.lock() {
                            queues.data_messages.push_back(buffer);
                        }
                    }
                }
            }
        }
        ROUTE_TRIGGER_IME => {
            if let Some(args) = message.argument_list() {
                let arg = args.bool(0);
                let enabled = arg != 0;
                if let Ok(mut queues) = ipc.event_queues.lock() {
                    queues.ime_enables.push_back(enabled);
                }
            }
        }
        ROUTE_IME_CARET_POSITION => {
            if let Some(args) = message.argument_list() {
                let x = args.int(0);
                let y = args.int(1);
                let height = args.int(2);
                if let Ok(mut queues) = ipc.event_queues.lock() {
                    queues.ime_composition_range = Some(ImeCompositionRange {
                        caret_x: x,
                        caret_y: y,
                        caret_height: height,
                    });
                }
            }
        }
        _ => {}
    }

    0
}
