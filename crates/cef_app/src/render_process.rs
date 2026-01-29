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
    OsrImeCaretHandler, OsrImeCaretHandlerBuilder, OsrIpcBinaryHandler, OsrIpcBinaryHandlerBuilder,
    OsrIpcHandler, OsrIpcHandlerBuilder,
};

#[derive(Clone)]
pub(crate) struct OsrRenderProcessHandler {}

impl OsrRenderProcessHandler {
    pub fn new() -> Self {
        Self {}
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

                        let caret_key: cef::CefStringUtf16 = "__sendImeCaretPosition".into();
                        let mut caret_handler = OsrImeCaretHandlerBuilder::build(OsrImeCaretHandler::new(Some(frame_arc)));
                        let mut caret_func = v8_value_create_function(Some(&"__sendImeCaretPosition".into()), Some(&mut caret_handler)).unwrap();
                        global.set_value_bykey(Some(&caret_key), Some(&mut caret_func), V8Propertyattribute::from(cef_v8_propertyattribute_t(0)));

                        let helper_script: cef::CefStringUtf16 = include_str!("ime_helper.js").into();
                        frame.execute_java_script(Some(&helper_script), None, 0);
                    }
            }
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
                            invoke_js_string_callback(frame, "onIpcMessage", &msg_str);
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
                                        invoke_js_binary_callback(frame, "onIpcBinaryMessage", &buffer);
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
fn invoke_js_string_callback(frame: &mut Frame, callback_name: &str, msg_str: &CefStringUtf16) {
    if let Some(context) = frame.v8_context()
        && context.enter() != 0
    {
        if let Some(mut global) = context.global() {
            let callback_key: CefStringUtf16 = callback_name.into();
            if let Some(callback) = global.value_bykey(Some(&callback_key))
                && callback.is_function() != 0
                && let Some(str_value) = v8_value_create_string(Some(msg_str))
            {
                let args = [Some(str_value)];
                let _ = callback.execute_function(Some(&mut global), Some(&args));
            }
        }
        context.exit();
    }
}

/// Invoke a JavaScript callback with an ArrayBuffer argument.
fn invoke_js_binary_callback(frame: &mut Frame, callback_name: &str, buffer: &[u8]) {
    if let Some(context) = frame.v8_context()
        && context.enter() != 0
    {
        if let Some(mut global) = context.global() {
            let callback_key: CefStringUtf16 = callback_name.into();
            let mut buffer_copy = buffer.to_owned();
            if let Some(callback) = global.value_bykey(Some(&callback_key))
                && callback.is_function() != 0
                && let Some(array_buffer) = v8_value_create_array_buffer_with_copy(
                    buffer_copy.as_mut_ptr(),
                    buffer_copy.len(),
                )
            {
                let args = [Some(array_buffer)];
                let _ = callback.execute_function(Some(&mut global), Some(&args));
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
