use std::sync::{Arc, Mutex};

use cef::{
    self, CefStringUtf16, Frame, ImplFrame, ImplListValue, ImplProcessMessage, ImplV8Handler,
    ImplV8Value, ProcessId, V8Handler, V8Value, WrapV8Handler, process_message_create, rc::Rc,
    v8_value_create_bool, wrap_v8_handler,
};

#[derive(Clone)]
pub(crate) struct OsrIpcHandler {
    frame: Option<Arc<Mutex<Frame>>>,
}

impl OsrIpcHandler {
    pub fn new(frame: Option<Arc<Mutex<Frame>>>) -> Self {
        Self { frame }
    }
}

impl OsrIpcHandlerBuilder {
    pub(crate) fn build(handler: OsrIpcHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrIpcHandlerBuilder {
        handler: OsrIpcHandler,
    }

    impl V8Handler {
        fn execute(
            &self,
            _name: Option<&CefStringUtf16>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<cef::V8Value>>,
            _exception: Option<&mut CefStringUtf16>
        ) -> i32 {
            if let Some(arguments) = arguments
                && let Some(arg) = arguments.first()
                    && let Some(arg) = arg {
                        if arg.is_string() != 1 {
                            if let Some(retval) = retval {
                                *retval = v8_value_create_bool(false as _);
                            }

                            return 0;
                        }

                        let route = CefStringUtf16::from("ipcRendererToGodot");
                        let msg_str = CefStringUtf16::from(&arg.string_value());
                        if let Some(frame) = self.handler.frame.as_ref() {
                            let frame = frame.lock().unwrap();

                            let process_message = process_message_create(Some(&route));
                            if let Some(mut process_message) = process_message {
                                if let Some(argument_list) = process_message.argument_list() {
                                    argument_list.set_string(0, Some(&msg_str));
                                }

                                frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));

                                if let Some(retval) = retval {
                                    *retval = v8_value_create_bool(true as _);
                                }

                                return 1;
                            }
                        }
                    }

            if let Some(retval) = retval {
                *retval = v8_value_create_bool(false as _);
            }

            return 0;
        }
    }
}

#[derive(Clone)]
pub(crate) struct OsrImeCaretHandler {
    frame: Option<Arc<Mutex<Frame>>>,
}

impl OsrImeCaretHandler {
    pub fn new(frame: Option<Arc<Mutex<Frame>>>) -> Self {
        Self { frame }
    }
}

impl OsrImeCaretHandlerBuilder {
    pub(crate) fn build(handler: OsrImeCaretHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrImeCaretHandlerBuilder {
        handler: OsrImeCaretHandler,
    }

    impl V8Handler {
        fn execute(
            &self,
            _name: Option<&CefStringUtf16>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<cef::V8Value>>,
            _exception: Option<&mut CefStringUtf16>
        ) -> i32 {
            if let Some(arguments) = arguments
                && arguments.len() >= 3
                && let Some(Some(x_arg)) = arguments.first()
                && let Some(Some(y_arg)) = arguments.get(1)
                && let Some(Some(height_arg)) = arguments.get(2)
            {
                let x = x_arg.int_value();
                let y = y_arg.int_value();
                let height = height_arg.int_value();

                if let Some(frame) = self.handler.frame.as_ref() {
                    match frame.lock() {
                        Ok(frame) => {
                            let route = CefStringUtf16::from("imeCaretPosition");
                            let process_message = process_message_create(Some(&route));
                            if let Some(mut process_message) = process_message {
                                if let Some(argument_list) = process_message.argument_list() {
                                    argument_list.set_int(0, x);
                                    argument_list.set_int(1, y);
                                    argument_list.set_int(2, height);
                                }

                                frame.send_process_message(ProcessId::BROWSER, Some(&mut process_message));

                                if let Some(retval) = retval {
                                    *retval = v8_value_create_bool(true as _);
                                }

                                return 1;
                            }
                        }
                        Err(_) => {
                            if let Some(retval) = retval {
                                *retval = v8_value_create_bool(false as _);
                            }
                            return 0;
                        }
                    }
                }
            }

            if let Some(retval) = retval {
                *retval = v8_value_create_bool(false as _);
            }

            0
        }
    }
}
