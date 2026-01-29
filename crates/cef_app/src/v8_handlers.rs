use std::sync::{Arc, Mutex};

use cef::{
    self, CefStringUtf16, Frame, ImplFrame, ImplListValue, ImplProcessMessage, ImplV8Handler,
    ImplV8Value, ProcessId, V8Handler, V8Value, WrapV8Handler, binary_value_create,
    process_message_create, rc::Rc, v8_value_create_bool, wrap_v8_handler,
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
pub(crate) struct OsrIpcBinaryHandler {
    frame: Option<Arc<Mutex<Frame>>>,
}

impl OsrIpcBinaryHandler {
    pub fn new(frame: Option<Arc<Mutex<Frame>>>) -> Self {
        Self { frame }
    }
}

impl OsrIpcBinaryHandlerBuilder {
    pub(crate) fn build(handler: OsrIpcBinaryHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrIpcBinaryHandlerBuilder {
        handler: OsrIpcBinaryHandler,
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
                && let Some(arg) = arg
            {
                if arg.is_array_buffer() != 1 {
                    if let Some(retval) = retval {
                        *retval = v8_value_create_bool(false as _);
                    }
                    return 0;
                }

                let data_ptr = arg.array_buffer_data();
                let data_len = arg.array_buffer_byte_length();

                if data_ptr.is_null() || data_len == 0 {
                    if let Some(retval) = retval {
                        *retval = v8_value_create_bool(false as _);
                    }
                    return 0;
                }

                let data: Vec<u8> = unsafe {
                    std::slice::from_raw_parts(data_ptr as *const u8, data_len).to_vec()
                };

                let Some(mut binary_value) = binary_value_create(Some(&data)) else {
                    if let Some(retval) = retval {
                        *retval = v8_value_create_bool(false as _);
                    }
                    return 0;
                };

                if let Some(frame) = self.handler.frame.as_ref() {
                    let frame = frame
                        .lock()
                        .expect("OsrIpcHandler: failed to lock frame mutex (poisoned)");

                    let route = CefStringUtf16::from("ipcBinaryRendererToGodot");
                    let process_message = process_message_create(Some(&route));
                    if let Some(mut process_message) = process_message {
                        if let Some(argument_list) = process_message.argument_list() {
                            argument_list.set_binary(0, Some(&mut binary_value));
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
