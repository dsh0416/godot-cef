use godot::classes::InputEvent;
use godot::prelude::*;

use super::FrameUpdate;

#[derive(Debug, Default)]
pub struct AndroidWebViewBridge;

impl AndroidWebViewBridge {
    pub fn new() -> Self {
        Self
    }

    pub fn create(
        &mut self,
        _instance_id: i64,
        _size: Vector2i,
        _url: String,
        _javascript_enabled: bool,
    ) -> Result<(), String> {
        Err("Android WebView texture backend is only available on Android".to_string())
    }

    pub fn shutdown(&mut self, _instance_id: i64) -> Result<(), String> {
        Ok(())
    }

    pub fn acquire_latest_frame(
        &mut self,
        _instance_id: i64,
    ) -> Result<Option<FrameUpdate>, String> {
        Ok(None)
    }

    pub fn load_url(&mut self, _instance_id: i64, _url: String) -> Result<(), String> {
        Ok(())
    }

    pub fn eval(&mut self, _instance_id: i64, _code: String) -> Result<(), String> {
        Ok(())
    }

    pub fn reload(&mut self, _instance_id: i64) -> Result<(), String> {
        Ok(())
    }

    pub fn go_back(&mut self, _instance_id: i64) -> Result<(), String> {
        Ok(())
    }

    pub fn go_forward(&mut self, _instance_id: i64) -> Result<(), String> {
        Ok(())
    }

    pub fn focus(&mut self, _instance_id: i64) -> Result<(), String> {
        Ok(())
    }

    pub fn clear_focus(&mut self, _instance_id: i64) -> Result<(), String> {
        Ok(())
    }

    pub fn resize(&mut self, _instance_id: i64, _size: Vector2i) -> Result<(), String> {
        Ok(())
    }

    pub fn set_javascript_enabled(
        &mut self,
        _instance_id: i64,
        _enabled: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    pub fn forward_input(
        &mut self,
        _instance_id: i64,
        _event: Gd<InputEvent>,
    ) -> Result<(), String> {
        Ok(())
    }
}
