use std::sync::{Arc, Mutex};

use cef::sys::cef_v8_propertyattribute_t;
use cef::{
    Browser, CefStringUtf16, Domnode, Frame, ImplBinaryValue, ImplDomnode, ImplFrame,
    ImplListValue, ImplProcessMessage, ImplRenderProcessHandler, ImplV8Context, ImplV8Value,
    ProcessId, ProcessMessage, RenderProcessHandler, V8Context, V8Propertyattribute,
    WrapRenderProcessHandler, process_message_create, rc::Rc,
    v8_value_create_array_buffer_with_copy, v8_value_create_function, v8_value_create_string,
    wrap_render_process_handler,
};

use crate::v8_handlers::{
    IpcListenerSet, OsrImeCaretHandler, OsrImeCaretHandlerBuilder, OsrIpcBinaryHandler,
    OsrIpcBinaryHandlerBuilder, OsrIpcDataHandler, OsrIpcDataHandlerBuilder, OsrIpcHandler,
    OsrIpcHandlerBuilder, cbor_bytes_to_v8_value,
};

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

                        let key: cef::CefStringUtf16 = "sendIpcMessage".to_string().as_str().into();
                        let mut handler = OsrIpcHandlerBuilder::build(OsrIpcHandler::new(Some(frame_arc.clone())));
                        let mut func = v8_value_create_function(Some(&"sendIpcMessage".into()), Some(&mut handler)).unwrap();
                        global.set_value_bykey(Some(&key), Some(&mut func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

                        let binary_key: cef::CefStringUtf16 = "sendIpcBinaryMessage".into();
                        let mut binary_handler = OsrIpcBinaryHandlerBuilder::build(OsrIpcBinaryHandler::new(Some(frame_arc.clone())));
                        let mut binary_func = v8_value_create_function(Some(&"sendIpcBinaryMessage".into()), Some(&mut binary_handler)).unwrap();
                        global.set_value_bykey(Some(&binary_key), Some(&mut binary_func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

                        let data_key: cef::CefStringUtf16 = "sendIpcData".into();
                        let mut data_handler = OsrIpcDataHandlerBuilder::build(OsrIpcDataHandler::new(Some(frame_arc.clone())));
                        let mut data_func = v8_value_create_function(Some(&"sendIpcData".into()), Some(&mut data_handler)).unwrap();
                        global.set_value_bykey(Some(&data_key), Some(&mut data_func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

                        if let Some(mut listeners_obj) = self.handler.string_listeners.build_api_object() {
                            global.set_value_bykey(
                                Some(&"ipcMessage".into()),
                                Some(&mut listeners_obj),
                                V8Propertyattribute::from(cef_v8_propertyattribute_t(0)),
                            );
                        }

                        if let Some(mut listeners_obj) = self.handler.binary_listeners.build_api_object() {
                            global.set_value_bykey(
                                Some(&"ipcBinaryMessage".into()),
                                Some(&mut listeners_obj),
                                V8Propertyattribute::from(cef_v8_propertyattribute_t(0)),
                            );
                        }

                        if let Some(mut listeners_obj) = self.handler.data_listeners.build_api_object() {
                            global.set_value_bykey(
                                Some(&"ipcDataMessage".into()),
                                Some(&mut listeners_obj),
                                V8Propertyattribute::from(cef_v8_propertyattribute_t(0)),
                            );
                        }

                        let caret_key: cef::CefStringUtf16 = "__sendImeCaretPosition".into();
                        let mut caret_handler = OsrImeCaretHandlerBuilder::build(OsrImeCaretHandler::new(Some(frame_arc)));
                        let mut caret_func = v8_value_create_function(Some(&"__sendImeCaretPosition".into()), Some(&mut caret_handler)).unwrap();
                        global.set_value_bykey(Some(&caret_key), Some(&mut caret_func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

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
                    // send to the browser process to activate IME
                    let route = cef::CefStringUtf16::from("triggerIme");
                    let process_message = process_message_create(Some(&route));
                    if let Some(mut process_message) = process_message {
                        if let Some(argument_list) = process_message.argument_list() {
                            argument_list.set_bool(0, true as _);
                        }

                        if let Some(frame) = frame {
                            frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));
                            let report_script: cef::CefStringUtf16 = "if(window.__activateImeTracking)window.__activateImeTracking();".into();
                            frame.execute_java_script(Some(&report_script), None, 0);
                        }
                    }
                    return;
                }

            // send to the browser process to deactivate IME
            let route = cef::CefStringUtf16::from("triggerIme");
            let process_message = process_message_create(Some(&route));
            if let Some(mut process_message) = process_message {
                if let Some(argument_list) = process_message.argument_list() {
                    argument_list.set_bool(0, false as _);
                }

                if let Some(frame) = frame {
                    frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));
                    let deactivate_script: cef::CefStringUtf16 = "if(window.__deactivateImeTracking)window.__deactivateImeTracking();".into();
                    frame.execute_java_script(Some(&deactivate_script), None, 0);
                }
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
                "ipcGodotToRenderer" => {
                    if let Some(args) = message.argument_list() {
                        let msg_cef = args.string(0);
                        let msg_str = CefStringUtf16::from(&msg_cef);

                        if let Some(frame) = frame {
                            invoke_js_string_callback(
                                frame,
                                "onIpcMessage",
                                &msg_str,
                                Some(&self.handler.string_listeners),
                            );
                        }
                    }
                    return 1;
                }
                "ipcBinaryGodotToRenderer" => {
                    if let Some(args) = message.argument_list()
                        && let Some(binary_value) = args.binary(0) {
                            let size = binary_value.size();
                            if size > 0 {
                                let mut buffer = vec![0u8; size];
                                let copied = binary_value.data(Some(&mut buffer), 0);
                                if copied > 0 {
                                    buffer.truncate(copied);

                                    if let Some(frame) = frame {
                                        invoke_js_binary_callback(
                                            frame,
                                            "onIpcBinaryMessage",
                                            &buffer,
                                            Some(&self.handler.binary_listeners),
                                        );
                                    }
                                }
                            }
                        }
                    return 1;
                }
                "ipcDataGodotToRenderer" => {
                    if let Some(args) = message.argument_list()
                        && let Some(binary_value) = args.binary(0)
                    {
                        let size = binary_value.size();
                        if size > 0 {
                            let mut buffer = vec![0u8; size];
                            let copied = binary_value.data(Some(&mut buffer), 0);
                            if copied > 0 {
                                buffer.truncate(copied);
                                if let Some(frame) = frame {
                                    invoke_js_data_callback(
                                        frame,
                                        "onIpcDataMessage",
                                        &buffer,
                                        Some(&self.handler.data_listeners),
                                    );
                                }
                            }
                        }
                    }
                    return 1;
                }
                _ => {}
            }

            0
        }
    }
}

/// Invoke a JavaScript callback with a string argument.
fn invoke_js_string_callback(
    frame: &mut Frame,
    callback_name: &str,
    msg_str: &CefStringUtf16,
    listeners: Option<&IpcListenerSet>,
) {
    if let Some(context) = frame.v8_context()
        && context.enter() != 0
    {
        if let Some(mut global) = context.global()
            && let Some(str_value) = v8_value_create_string(Some(msg_str))
        {
            let callback_key: CefStringUtf16 = callback_name.into();
            if let Some(callback) = global.value_bykey(Some(&callback_key))
                && callback.is_function() != 0
            {
                let args = [Some(str_value.clone())];
                let _ = callback.execute_function(Some(&mut global), Some(&args));
            }
            if let Some(listeners) = listeners {
                listeners.emit(&str_value);
            }
        }
        context.exit();
    }
}

/// Invoke a JavaScript callback with an ArrayBuffer argument.
fn invoke_js_binary_callback(
    frame: &mut Frame,
    callback_name: &str,
    buffer: &[u8],
    listeners: Option<&IpcListenerSet>,
) {
    if let Some(context) = frame.v8_context()
        && context.enter() != 0
    {
        if let Some(mut global) = context.global() {
            let callback_key: CefStringUtf16 = callback_name.into();
            let mut buffer_copy = buffer.to_owned();
            if let Some(array_buffer) =
                v8_value_create_array_buffer_with_copy(buffer_copy.as_mut_ptr(), buffer_copy.len())
            {
                if let Some(callback) = global.value_bykey(Some(&callback_key))
                    && callback.is_function() != 0
                {
                    let args = [Some(array_buffer.clone())];
                    let _ = callback.execute_function(Some(&mut global), Some(&args));
                }
                if let Some(listeners) = listeners {
                    listeners.emit(&array_buffer);
                }
            }
        }
        context.exit();
    }
}

fn invoke_js_data_callback(
    frame: &mut Frame,
    callback_name: &str,
    cbor_payload: &[u8],
    listeners: Option<&IpcListenerSet>,
) {
    if let Some(context) = frame.v8_context()
        && context.enter() != 0
    {
        if let Some(mut global) = context.global()
            && let Ok(value) = cbor_bytes_to_v8_value(cbor_payload)
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
