use std::sync::{Arc, Mutex};

use cef::{
    Browser, CefStringUtf16, Domnode, Frame, ImplBinaryValue, ImplDomnode, ImplFrame,
    ImplListValue, ImplProcessMessage, ImplRenderProcessHandler, ImplV8Context, ImplV8Value,
    ProcessId, ProcessMessage, RenderProcessHandler, V8Context, V8Handler, V8Value,
    WrapRenderProcessHandler, process_message_create, rc::Rc,
    v8_value_create_array_buffer_with_copy, v8_value_create_function, v8_value_create_string,
    wrap_render_process_handler,
};

use crate::ipc_contract::{
    ROUTE_IPC_BINARY_GODOT_TO_RENDERER, ROUTE_IPC_DATA_GODOT_TO_RENDERER,
    ROUTE_IPC_GODOT_TO_RENDERER, ROUTE_TRIGGER_IME,
};
use crate::v8_handlers::{
    IpcListenerSet, OsrImeCaretHandler, OsrImeCaretHandlerBuilder, OsrIpcBinaryHandler,
    OsrIpcBinaryHandlerBuilder, OsrIpcDataHandler, OsrIpcDataHandlerBuilder, OsrIpcHandler,
    OsrIpcHandlerBuilder, cbor_bytes_to_v8_value, v8_prop_default,
};

fn send_browser_bool_message(frame: Option<&mut Frame>, route: &str, value: bool) {
    let Some(frame) = frame else {
        return;
    };
    let route = cef::CefStringUtf16::from(route);
    let Some(mut process_message) = process_message_create(Some(&route)) else {
        return;
    };
    if let Some(argument_list) = process_message.argument_list() {
        argument_list.set_bool(0, value as _);
    }
    frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));
}

#[derive(Clone)]
pub(crate) struct OsrRenderProcessHandler {
    string_listeners: IpcListenerSet,
    binary_listeners: IpcListenerSet,
    data_listeners: IpcListenerSet,
}

impl OsrRenderProcessHandler {
    pub fn new() -> Self {
        Self {
            string_listeners: IpcListenerSet::new(),
            binary_listeners: IpcListenerSet::new(),
            data_listeners: IpcListenerSet::new(),
        }
    }
}

wrap_render_process_handler! {
    pub(crate) struct RenderProcessHandlerBuilder {
        handler: OsrRenderProcessHandler,
    }

    impl RenderProcessHandler {
        fn on_context_created(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            if let Some(context) = context {
                let global = context.global();
                if let Some(global) = global
                    && let Some(frame) = frame {
                        let frame_arc = Arc::new(Mutex::new(frame.clone()));

                        register_v8_function(&global, "sendIpcMessage",
                            &mut OsrIpcHandlerBuilder::build(OsrIpcHandler::new(Some(frame_arc.clone()))));
                        register_v8_function(&global, "sendIpcBinaryMessage",
                            &mut OsrIpcBinaryHandlerBuilder::build(OsrIpcBinaryHandler::new(Some(frame_arc.clone()))));
                        register_v8_function(&global, "sendIpcData",
                            &mut OsrIpcDataHandlerBuilder::build(OsrIpcDataHandler::new(Some(frame_arc.clone()))));

                        for (name, listeners) in [
                            ("ipcMessage", &self.handler.string_listeners),
                            ("ipcBinaryMessage", &self.handler.binary_listeners),
                            ("ipcDataMessage", &self.handler.data_listeners),
                        ] {
                            if let Some(mut obj) = listeners.build_api_object() {
                                register_v8_value(&global, name, &mut obj);
                            }
                        }

                        register_v8_function(&global, "__sendImeCaretPosition",
                            &mut OsrImeCaretHandlerBuilder::build(OsrImeCaretHandler::new(Some(frame_arc))));

                        let helper_script: cef::CefStringUtf16 = include_str!("ime_helper.js").into();
                        frame.execute_java_script(Some(&helper_script), None, 0);
                    }
            }
        }

        fn on_context_released(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _context: Option<&mut V8Context>,
        ) {
            // Listener callbacks hold V8 function references. Clear them when
            // a V8 context is released so we don't retain stale callbacks.
            self.handler.string_listeners.clear();
            self.handler.binary_listeners.clear();
            self.handler.data_listeners.clear();
        }

        fn on_focused_node_changed(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, node: Option<&mut Domnode>) {
            if let Some(node) = node
                && node.is_editable() == 1 {
                    if let Some(frame) = frame {
                        // send to the browser process to activate IME
                        send_browser_bool_message(Some(frame), ROUTE_TRIGGER_IME, true);
                        let report_script: cef::CefStringUtf16 = "if(window.__activateImeTracking)window.__activateImeTracking();".into();
                        frame.execute_java_script(Some(&report_script), None, 0);
                    }
                    return;
                }

            if let Some(frame) = frame {
                // send to the browser process to deactivate IME
                send_browser_bool_message(Some(frame), ROUTE_TRIGGER_IME, false);
                let deactivate_script: cef::CefStringUtf16 = "if(window.__deactivateImeTracking)window.__deactivateImeTracking();".into();
                frame.execute_java_script(Some(&deactivate_script), None, 0);
            }
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let Some(message) = message else { return 0 };
            let route = CefStringUtf16::from(&message.name()).to_string();

            match route.as_str() {
                ROUTE_IPC_GODOT_TO_RENDERER => {
                    if let Some(args) = message.argument_list()
                        && let Some(frame) = frame
                    {
                        let msg_cef = args.string(0);
                        let msg_str = CefStringUtf16::from(&msg_cef);
                        invoke_js_callback(frame, "onIpcMessage", Some(&self.handler.string_listeners), |_| {
                            v8_value_create_string(Some(&msg_str))
                        });
                    }
                    return 1;
                }
                ROUTE_IPC_BINARY_GODOT_TO_RENDERER => {
                    if let Some(buffer) = extract_binary_payload(message)
                        && let Some(frame) = frame
                    {
                        invoke_js_callback(frame, "onIpcBinaryMessage", Some(&self.handler.binary_listeners), |_| {
                            let mut copy = buffer.clone();
                            v8_value_create_array_buffer_with_copy(copy.as_mut_ptr(), copy.len())
                        });
                    }
                    return 1;
                }
                ROUTE_IPC_DATA_GODOT_TO_RENDERER => {
                    if let Some(buffer) = extract_binary_payload(message)
                        && let Some(frame) = frame
                    {
                        invoke_js_callback(frame, "onIpcDataMessage", Some(&self.handler.data_listeners), |_| {
                            cbor_bytes_to_v8_value(&buffer).ok()
                        });
                    }
                    return 1;
                }
                _ => {}
            }

            0
        }
    }
}

fn register_v8_function(global: &V8Value, name: &str, handler: &mut V8Handler) {
    let key: CefStringUtf16 = name.into();
    if let Some(mut func) = v8_value_create_function(Some(&key), Some(handler)) {
        global.set_value_bykey(Some(&key), Some(&mut func), v8_prop_default());
    }
}

fn register_v8_value(global: &V8Value, name: &str, value: &mut V8Value) {
    let key: CefStringUtf16 = name.into();
    global.set_value_bykey(Some(&key), Some(value), v8_prop_default());
}

fn extract_binary_payload(message: &mut ProcessMessage) -> Option<Vec<u8>> {
    let args = message.argument_list()?;
    let binary_value = args.binary(0)?;
    let size = binary_value.size();
    if size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size];
    let copied = binary_value.data(Some(&mut buffer), 0);
    if copied == 0 {
        return None;
    }
    buffer.truncate(copied);
    Some(buffer)
}

fn invoke_js_callback(
    frame: &mut Frame,
    callback_name: &str,
    listeners: Option<&IpcListenerSet>,
    create_value: impl FnOnce(&mut V8Value) -> Option<V8Value>,
) {
    if let Some(context) = frame.v8_context()
        && context.enter() != 0
    {
        if let Some(mut global) = context.global()
            && let Some(value) = create_value(&mut global)
        {
            let callback_key: CefStringUtf16 = callback_name.into();
            if let Some(callback) = global.value_bykey(Some(&callback_key))
                && callback.is_function() != 0
            {
                let args = [Some(value.clone())];
                let _ = callback.execute_function(Some(&mut global), Some(&args));
            }
            if let Some(listeners) = listeners {
                listeners.emit(&value);
            }
        }
        context.exit();
    }
}

impl RenderProcessHandlerBuilder {
    pub(crate) fn build(handler: OsrRenderProcessHandler) -> RenderProcessHandler {
        Self::new(handler)
    }
}
