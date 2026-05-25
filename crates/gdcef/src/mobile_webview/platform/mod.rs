#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_os = "android"))]
mod stub;

#[cfg(target_os = "android")]
pub use android::AndroidWebViewBridge;
#[cfg(not(target_os = "android"))]
pub use stub::AndroidWebViewBridge;

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct FrameUpdate {
    pub external_buffer_id: u64,
    pub width: i32,
    pub height: i32,
}
