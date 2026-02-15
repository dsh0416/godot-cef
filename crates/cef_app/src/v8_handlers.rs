use ciborium::value::Value as CborValue;
use std::sync::{Arc, Mutex};
use std::{cell::RefCell, rc::Rc as StdRc};

use cef::{
    self, CefStringUtf16, Frame, ImplFrame, ImplListValue, ImplProcessMessage, ImplV8Handler,
    ImplV8Value, ProcessId, V8Handler, V8Value, WrapV8Handler, binary_value_create,
    process_message_create, rc::Rc, v8_value_create_bool, v8_value_create_function,
    v8_value_create_object, wrap_v8_handler,
};

// Keep this in sync with crates/gdcef/src/ipc_data.rs.
const MAX_IPC_DATA_BYTES: usize = 8 * 1024 * 1024;

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

#[derive(Clone)]
pub(crate) struct OsrIpcDataHandler {
    frame: Option<Arc<Mutex<Frame>>>,
}

impl OsrIpcDataHandler {
    pub fn new(frame: Option<Arc<Mutex<Frame>>>) -> Self {
        Self { frame }
    }
}

impl OsrIpcDataHandlerBuilder {
    pub(crate) fn build(handler: OsrIpcDataHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrIpcDataHandlerBuilder {
        handler: OsrIpcDataHandler,
    }

    impl V8Handler {
        fn execute(
            &self,
            _name: Option<&CefStringUtf16>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<cef::V8Value>>,
            exception: Option<&mut CefStringUtf16>
        ) -> i32 {
            if let Some(arguments) = arguments
                && let Some(Some(arg)) = arguments.first()
            {
                match v8_to_cbor_bytes(arg) {
                    Ok(encoded) => {
                        if encoded.len() > MAX_IPC_DATA_BYTES {
                            if let Some(retval) = retval {
                                *retval = v8_value_create_bool(false as _);
                            }
                            if let Some(exception) = exception {
                                let msg = format!(
                                    "IPC data payload exceeds maximum size of {} bytes",
                                    MAX_IPC_DATA_BYTES
                                );
                                *exception = CefStringUtf16::from(msg.as_str());
                            }
                            return 0;
                        }

                        if let Some(mut binary) = binary_value_create(Some(&encoded))
                            && let Some(frame) = self.handler.frame.as_ref()
                        {
                            let frame = frame
                                .lock()
                                .expect("OsrIpcDataHandler: failed to lock frame mutex (poisoned)");
                            let route = CefStringUtf16::from("ipcDataRendererToGodot");
                            if let Some(mut process_message) = process_message_create(Some(&route))
                                && let Some(argument_list) = process_message.argument_list()
                            {
                                argument_list.set_binary(0, Some(&mut binary));
                                frame.send_process_message(
                                    ProcessId::BROWSER,
                                    Some(&mut process_message),
                                );
                                if let Some(retval) = retval {
                                    *retval = v8_value_create_bool(true as _);
                                }
                                return 1;
                            }
                        }
                    }
                    Err(err) => {
                        if let Some(retval) = retval {
                            *retval = v8_value_create_bool(false as _);
                        }
                        if let Some(exception) = exception {
                            *exception = CefStringUtf16::from(err.as_str());
                        }
                        return 0;
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

type ListenerCallbacks = StdRc<RefCell<Vec<V8Value>>>;

#[derive(Clone)]
pub(crate) struct IpcListenerSet {
    callbacks: ListenerCallbacks,
}

impl IpcListenerSet {
    pub fn new() -> Self {
        Self {
            callbacks: StdRc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn emit(&self, value: &V8Value) {
        // Drop invalid/non-function callbacks first so stale V8 references
        // do not accumulate across context lifetimes.
        {
            let mut callbacks = self.callbacks.borrow_mut();
            callbacks.retain(|callback| callback.is_valid() != 0 && callback.is_function() != 0);
        }

        // Snapshot before invoking callbacks to avoid RefCell re-entrancy
        // if listeners are added/removed while a callback is running.
        let callbacks_snapshot = self.callbacks.borrow().clone();
        for callback in callbacks_snapshot {
            let _ = callback.execute_function(None, Some(&[Some(value.clone())]));
        }
    }

    pub fn clear(&self) {
        self.callbacks.borrow_mut().clear();
    }

    pub fn build_api_object(&self) -> Option<V8Value> {
        let object = v8_value_create_object(None, None)?;

        let mut add_handler = OsrListenerHandlerBuilder::build(OsrListenerHandler::new(
            self.callbacks.clone(),
            ListenerOperation::Add,
        ));
        let mut remove_handler = OsrListenerHandlerBuilder::build(OsrListenerHandler::new(
            self.callbacks.clone(),
            ListenerOperation::Remove,
        ));
        let mut has_handler = OsrListenerHandlerBuilder::build(OsrListenerHandler::new(
            self.callbacks.clone(),
            ListenerOperation::Has,
        ));

        let mut add_fn =
            v8_value_create_function(Some(&"addListener".into()), Some(&mut add_handler))?;
        let mut remove_fn =
            v8_value_create_function(Some(&"removeListener".into()), Some(&mut remove_handler))?;
        let mut has_fn =
            v8_value_create_function(Some(&"hasListener".into()), Some(&mut has_handler))?;

        object.set_value_bykey(
            Some(&"addListener".into()),
            Some(&mut add_fn),
            cef::V8Propertyattribute::from(cef::sys::cef_v8_propertyattribute_t(0)),
        );
        object.set_value_bykey(
            Some(&"removeListener".into()),
            Some(&mut remove_fn),
            cef::V8Propertyattribute::from(cef::sys::cef_v8_propertyattribute_t(0)),
        );
        object.set_value_bykey(
            Some(&"hasListener".into()),
            Some(&mut has_fn),
            cef::V8Propertyattribute::from(cef::sys::cef_v8_propertyattribute_t(0)),
        );

        Some(object)
    }
}

#[derive(Clone, Copy)]
enum ListenerOperation {
    Add,
    Remove,
    Has,
}

#[derive(Clone)]
pub(crate) struct OsrListenerHandler {
    callbacks: ListenerCallbacks,
    op: ListenerOperation,
}

impl OsrListenerHandler {
    fn new(callbacks: ListenerCallbacks, op: ListenerOperation) -> Self {
        Self { callbacks, op }
    }
}

impl OsrListenerHandlerBuilder {
    pub(crate) fn build(handler: OsrListenerHandler) -> V8Handler {
        Self::new(handler)
    }
}

wrap_v8_handler! {
    pub(crate) struct OsrListenerHandlerBuilder {
        handler: OsrListenerHandler,
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
            let mut result = false;
            if let Some(arguments) = arguments
                && let Some(Some(arg)) = arguments.first()
                && arg.is_function() != 0
            {
                let mut callbacks = self.handler.callbacks.borrow_mut();
                match self.handler.op {
                    ListenerOperation::Add => {
                        if !callbacks.iter().any(|existing| {
                            let mut cb = arg.clone();
                            existing.is_same(Some(&mut cb)) != 0
                        }) {
                            callbacks.push(arg.clone());
                        }
                        result = true;
                    }
                    ListenerOperation::Remove => {
                        callbacks.retain(|existing| {
                            let mut cb = arg.clone();
                            existing.is_same(Some(&mut cb)) == 0
                        });
                        result = true;
                    }
                    ListenerOperation::Has => {
                        result = callbacks.iter().any(|existing| {
                            let mut cb = arg.clone();
                            existing.is_same(Some(&mut cb)) != 0
                        });
                    }
                }
            }

            if let Some(retval) = retval {
                *retval = v8_value_create_bool(result as _);
            }
            1
        }
    }
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

fn v8_to_cbor_bytes(value: &V8Value) -> Result<Vec<u8>, String> {
    let cbor = v8_to_cbor_value(value)?;
    let mut out = Vec::new();
    ciborium::ser::into_writer(&cbor, &mut out).map_err(|e| format!("CBOR encode failed: {e}"))?;
    if out.len() > MAX_IPC_DATA_BYTES {
        return Err(format!(
            "CBOR payload exceeds maximum size of {} bytes",
            MAX_IPC_DATA_BYTES
        ));
    }
    Ok(out)
}

fn v8_to_cbor_value(value: &V8Value) -> Result<CborValue, String> {
    if value.is_undefined() != 0 || value.is_null() != 0 {
        return Ok(CborValue::Null);
    }
    if value.is_bool() != 0 {
        return Ok(CborValue::Bool(value.bool_value() != 0));
    }
    if value.is_int() != 0 {
        return Ok(CborValue::Integer((value.int_value() as i64).into()));
    }
    if value.is_uint() != 0 {
        return Ok(CborValue::Integer((value.uint_value() as u64).into()));
    }
    if value.is_double() != 0 {
        return Ok(CborValue::Float(value.double_value()));
    }
    if value.is_string() != 0 {
        return Ok(CborValue::Text(
            CefStringUtf16::from(&value.string_value()).to_string(),
        ));
    }
    if value.is_array_buffer() != 0 {
        let ptr = value.array_buffer_data();
        let len = value.array_buffer_byte_length();
        if len > MAX_IPC_DATA_BYTES {
            return Err(format!(
                "ArrayBuffer exceeds maximum IPC data size of {} bytes",
                MAX_IPC_DATA_BYTES
            ));
        }
        if ptr.is_null() || len == 0 {
            return Ok(CborValue::Bytes(Vec::new()));
        }
        let data = unsafe { std::slice::from_raw_parts(ptr as *const u8, len).to_vec() };
        return Ok(CborValue::Bytes(data));
    }
    if value.is_array() != 0 {
        let len = value.array_length();
        let mut out = Vec::with_capacity(len as usize);
        for i in 0..len {
            if let Some(element) = value.value_byindex(i) {
                out.push(v8_to_cbor_value(&element)?);
            } else {
                out.push(CborValue::Null);
            }
        }
        return Ok(CborValue::Array(out));
    }
    // Treat plain JS objects as CBOR maps, preserving string keys.
    if value.is_object() != 0 {
        // Retrieve the list of own enumerable property names via CEF.
        let mut keys_list = cef::CefStringList::new();
        if value.keys(Some(&mut keys_list)) != 0 {
            let mut entries = Vec::new();
            for key in keys_list {
                // Look up the corresponding property value on the object.
                let key_cef_for_lookup = CefStringUtf16::from(key.as_str());
                if let Some(prop) = value.value_bykey(Some(&key_cef_for_lookup)) {
                    let encoded = v8_to_cbor_value(&prop)?;
                    entries.push((CborValue::Text(key), encoded));
                }
            }
            return Ok(CborValue::Map(entries));
        }
    }
    Err("Unsupported JS value for CBOR IPC".to_string())
}

pub(crate) fn cbor_bytes_to_v8_value(bytes: &[u8]) -> Result<V8Value, String> {
    let cbor: CborValue =
        ciborium::de::from_reader(bytes).map_err(|e| format!("CBOR decode failed: {e}"))?;
    cbor_value_to_v8(&cbor).ok_or_else(|| "Failed to convert CBOR to V8".to_string())
}

fn cbor_value_to_v8(value: &CborValue) -> Option<V8Value> {
    match value {
        CborValue::Null => cef::v8_value_create_null(),
        CborValue::Bool(v) => v8_value_create_bool(*v as _),
        CborValue::Integer(v) => {
            let int_val = i128::from(*v);
            if int_val >= i32::MIN as i128 && int_val <= i32::MAX as i128 {
                cef::v8_value_create_int(int_val as i32)
            } else {
                cef::v8_value_create_double(int_val as f64)
            }
        }
        CborValue::Float(v) => cef::v8_value_create_double(*v),
        CborValue::Text(v) => {
            let s: CefStringUtf16 = v.as_str().into();
            cef::v8_value_create_string(Some(&s))
        }
        CborValue::Bytes(v) => {
            let mut copy = v.clone();
            cef::v8_value_create_array_buffer_with_copy(copy.as_mut_ptr(), copy.len())
        }
        CborValue::Array(v) => {
            let array = cef::v8_value_create_array(v.len() as i32)?;
            for (idx, item) in v.iter().enumerate() {
                if let Some(mut value) = cbor_value_to_v8(item) {
                    array.set_value_byindex(idx as i32, Some(&mut value));
                }
            }
            Some(array)
        }
        CborValue::Map(v) => {
            let object = v8_value_create_object(None, None)?;
            for (key, map_value) in v {
                let key = cbor_map_key_to_js_property_name(key);
                let key_cef = CefStringUtf16::from(key.as_str());

                // Preserve map shape even when a value type is unsupported.
                let mut js_value =
                    cbor_value_to_v8(map_value).or_else(cef::v8_value_create_null)?;
                object.set_value_bykey(
                    Some(&key_cef),
                    Some(&mut js_value),
                    cef::V8Propertyattribute::from(cef::sys::cef_v8_propertyattribute_t(0)),
                );
            }
            Some(object)
        }
        CborValue::Tag(_, inner) => cbor_value_to_v8(inner),
        _ => None,
    }
}

fn cbor_map_key_to_js_property_name(key: &CborValue) -> String {
    match key {
        CborValue::Text(v) => v.clone(),
        CborValue::Integer(v) => i128::from(*v).to_string(),
        CborValue::Float(v) => v.to_string(),
        CborValue::Bool(v) => v.to_string(),
        CborValue::Null => "null".to_string(),
        CborValue::Bytes(v) => {
            // Keep binary keys stable and ASCII-safe for JS object properties.
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut out = String::with_capacity(v.len() * 2);
            for byte in v {
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
            out
        }
        other => format!("{other:?}"),
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
