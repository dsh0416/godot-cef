use std::sync::{Arc, Mutex};

use cef::sys::cef_v8_propertyattribute_t;
use cef::{
    Browser, Domnode, Frame, ImplDomnode, ImplFrame, ImplListValue, ImplProcessMessage,
    ImplRenderProcessHandler, ImplV8Context, ImplV8Value, ProcessId, RenderProcessHandler,
    V8Context, V8Propertyattribute, WrapRenderProcessHandler, process_message_create, rc::Rc,
    v8_value_create_function, wrap_render_process_handler,
};

use crate::v8_handlers::{
    OsrImeCaretHandler, OsrImeCaretHandlerBuilder, OsrIpcHandler, OsrIpcHandlerBuilder,
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
    }
}

impl RenderProcessHandlerBuilder {
    pub(crate) fn build(handler: OsrRenderProcessHandler) -> RenderProcessHandler {
        Self::new(handler)
    }
}
