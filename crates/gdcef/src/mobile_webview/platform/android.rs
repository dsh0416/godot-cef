use godot::classes::{
    InputEvent, InputEventMouseButton, InputEventMouseMotion, InputEventScreenDrag,
    InputEventScreenTouch,
};
use godot::prelude::*;
use jni::objects::{JObject, JValue, JValueOwned};
use jni::signature::{MethodSignature, RuntimeMethodSignature};
use jni::strings::JNIString;
use jni::sys::{JNI_VERSION_1_6, jint, jobject};
use jni::{Env, JavaVM};
use std::ffi::c_void;
use std::sync::OnceLock;

use super::FrameUpdate;

const BRIDGE_CLASS: &str = "io/github/dsh0416/godotcef/AndroidWebViewBridge";

static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

#[repr(C)]
struct AHardwareBuffer {
    _private: [u8; 0],
}

#[link(name = "android")]
unsafe extern "C" {
    fn AHardwareBuffer_fromHardwareBuffer(
        env: *mut jni::sys::JNIEnv,
        hardware_buffer: jobject,
    ) -> *mut AHardwareBuffer;
    fn AHardwareBuffer_acquire(buffer: *mut AHardwareBuffer);
    fn AHardwareBuffer_release(buffer: *mut AHardwareBuffer);
}

#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: *mut jni::sys::JavaVM, _reserved: *mut c_void) -> jint {
    let vm = unsafe { JavaVM::from_raw(vm) };
    let _ = JAVA_VM.set(vm);
    JNI_VERSION_1_6
}

#[derive(Debug, Default)]
pub struct AndroidWebViewBridge {
    retained_buffer: Option<*mut AHardwareBuffer>,
}

impl AndroidWebViewBridge {
    pub fn new() -> Self {
        Self {
            retained_buffer: None,
        }
    }

    pub fn create(
        &mut self,
        instance_id: i64,
        size: Vector2i,
        url: String,
        javascript_enabled: bool,
    ) -> Result<(), String> {
        with_env(|env| {
            let url = env.new_string(url).map_err(jni_error)?;
            let url_obj: JObject = url.into();
            call_bridge_static(
                env,
                "create",
                "(JIILjava/lang/String;Z)V",
                &[
                    JValue::Long(instance_id),
                    JValue::Int(size.x),
                    JValue::Int(size.y),
                    JValue::Object(&url_obj),
                    JValue::Bool(javascript_enabled),
                ],
            )?;
            Ok(())
        })
    }

    pub fn shutdown(&mut self, instance_id: i64) -> Result<(), String> {
        let result = call_void("shutdown", "(J)V", &[JValue::Long(instance_id)]);
        self.release_retained_buffer();
        result
    }

    pub fn acquire_latest_frame(
        &mut self,
        instance_id: i64,
    ) -> Result<Option<FrameUpdate>, String> {
        with_env(|env| {
            let hardware_buffer = call_bridge_static(
                env,
                "acquireLatestFrame",
                "(J)Landroid/hardware/HardwareBuffer;",
                &[JValue::Long(instance_id)],
            )?
            .l()
            .map_err(jni_error)?;
            if hardware_buffer.is_null() {
                return Ok(None);
            }
            let hardware_buffer_ptr = hardware_buffer_ptr(env, &hardware_buffer)?;
            self.retain_buffer(hardware_buffer_ptr);

            let width =
                call_bridge_static(env, "getFrameWidth", "(J)I", &[JValue::Long(instance_id)])?
                    .i()
                    .map_err(jni_error)?;
            let height =
                call_bridge_static(env, "getFrameHeight", "(J)I", &[JValue::Long(instance_id)])?
                    .i()
                    .map_err(jni_error)?;

            Ok(Some(FrameUpdate {
                external_buffer_id: hardware_buffer_ptr as u64,
                width,
                height,
            }))
        })
    }

    pub fn load_url(&mut self, instance_id: i64, url: String) -> Result<(), String> {
        with_env(|env| {
            let url = env.new_string(url).map_err(jni_error)?;
            let url_obj: JObject = url.into();
            call_bridge_static(
                env,
                "loadUrl",
                "(JLjava/lang/String;)V",
                &[JValue::Long(instance_id), JValue::Object(&url_obj)],
            )?;
            Ok(())
        })
    }

    pub fn eval(&mut self, instance_id: i64, code: String) -> Result<(), String> {
        with_env(|env| {
            let code = env.new_string(code).map_err(jni_error)?;
            let code_obj: JObject = code.into();
            call_bridge_static(
                env,
                "eval",
                "(JLjava/lang/String;)V",
                &[JValue::Long(instance_id), JValue::Object(&code_obj)],
            )?;
            Ok(())
        })
    }

    pub fn reload(&mut self, instance_id: i64) -> Result<(), String> {
        call_void("reload", "(J)V", &[JValue::Long(instance_id)])
    }

    pub fn go_back(&mut self, instance_id: i64) -> Result<(), String> {
        call_void("goBack", "(J)V", &[JValue::Long(instance_id)])
    }

    pub fn go_forward(&mut self, instance_id: i64) -> Result<(), String> {
        call_void("goForward", "(J)V", &[JValue::Long(instance_id)])
    }

    pub fn focus(&mut self, instance_id: i64) -> Result<(), String> {
        call_void("focus", "(J)V", &[JValue::Long(instance_id)])
    }

    pub fn clear_focus(&mut self, instance_id: i64) -> Result<(), String> {
        call_void("clearFocus", "(J)V", &[JValue::Long(instance_id)])
    }

    pub fn resize(&mut self, instance_id: i64, size: Vector2i) -> Result<(), String> {
        call_void(
            "resize",
            "(JII)V",
            &[
                JValue::Long(instance_id),
                JValue::Int(size.x),
                JValue::Int(size.y),
            ],
        )
    }

    pub fn set_javascript_enabled(
        &mut self,
        instance_id: i64,
        enabled: bool,
    ) -> Result<(), String> {
        call_void(
            "setJavaScriptEnabled",
            "(JZ)V",
            &[JValue::Long(instance_id), JValue::Bool(enabled)],
        )
    }

    pub fn forward_input(&mut self, instance_id: i64, event: Gd<InputEvent>) -> Result<(), String> {
        if let Ok(touch) = event.clone().try_cast::<InputEventScreenTouch>() {
            let position = touch.get_position();
            return call_void(
                "touch",
                "(JIFFZ)V",
                &[
                    JValue::Long(instance_id),
                    JValue::Int(touch.get_index()),
                    JValue::Float(position.x),
                    JValue::Float(position.y),
                    JValue::Bool(touch.is_pressed()),
                ],
            );
        }

        if let Ok(drag) = event.clone().try_cast::<InputEventScreenDrag>() {
            let position = drag.get_position();
            return call_void(
                "drag",
                "(JIFF)V",
                &[
                    JValue::Long(instance_id),
                    JValue::Int(drag.get_index()),
                    JValue::Float(position.x),
                    JValue::Float(position.y),
                ],
            );
        }

        if let Ok(button) = event.clone().try_cast::<InputEventMouseButton>() {
            let position = button.get_position();
            return call_void(
                "mouseButton",
                "(JFFZ)V",
                &[
                    JValue::Long(instance_id),
                    JValue::Float(position.x),
                    JValue::Float(position.y),
                    JValue::Bool(button.is_pressed()),
                ],
            );
        }

        if let Ok(motion) = event.try_cast::<InputEventMouseMotion>() {
            let position = motion.get_position();
            return call_void(
                "mouseMove",
                "(JFF)V",
                &[
                    JValue::Long(instance_id),
                    JValue::Float(position.x),
                    JValue::Float(position.y),
                ],
            );
        }

        Ok(())
    }

    fn retain_buffer(&mut self, buffer: *mut AHardwareBuffer) {
        if self.retained_buffer == Some(buffer) {
            return;
        }
        unsafe {
            AHardwareBuffer_acquire(buffer);
        }
        self.release_retained_buffer();
        self.retained_buffer = Some(buffer);
    }

    fn release_retained_buffer(&mut self) {
        if let Some(buffer) = self.retained_buffer.take() {
            unsafe {
                AHardwareBuffer_release(buffer);
            }
        }
    }
}

impl Drop for AndroidWebViewBridge {
    fn drop(&mut self) {
        self.release_retained_buffer();
    }
}

fn hardware_buffer_ptr(
    env: &mut Env,
    hardware_buffer: &JObject,
) -> Result<*mut AHardwareBuffer, String> {
    let ptr =
        unsafe { AHardwareBuffer_fromHardwareBuffer(env.get_raw(), hardware_buffer.as_raw()) };
    if ptr.is_null() {
        return Err("AHardwareBuffer_fromHardwareBuffer returned null".to_string());
    }
    Ok(ptr)
}

fn with_env<R>(f: impl FnOnce(&mut Env) -> Result<R, String>) -> Result<R, String> {
    let Some(vm) = JAVA_VM.get() else {
        return Err("JNI_OnLoad has not provided a JavaVM".to_string());
    };
    vm.attach_current_thread(|env| f(env).map_err(BridgeError::from))
        .map_err(|error| error.0)
}

fn call_void(name: &str, sig: &str, args: &[JValue]) -> Result<(), String> {
    with_env(|env| {
        call_bridge_static(env, name, sig, args)?;
        Ok(())
    })
}

fn call_bridge_static<'local>(
    env: &mut Env<'local>,
    name: &str,
    sig: &str,
    args: &[JValue],
) -> Result<JValueOwned<'local>, String> {
    let class = JNIString::new(BRIDGE_CLASS);
    let name = JNIString::new(name);
    let sig = RuntimeMethodSignature::from_str(sig).map_err(jni_error)?;
    let sig_ref = MethodSignature::from(&sig);
    env.call_static_method(class, name, sig_ref, args)
        .map_err(jni_error)
}

struct BridgeError(String);

impl From<String> for BridgeError {
    fn from(error: String) -> Self {
        Self(error)
    }
}

impl From<jni::errors::Error> for BridgeError {
    fn from(error: jni::errors::Error) -> Self {
        Self(error.to_string())
    }
}

fn jni_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
