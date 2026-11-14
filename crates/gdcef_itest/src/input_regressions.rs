//! Browser results for inactive-window input and frame-aware text commits.

use super::*;
use godot::classes::{
    InputEventKey, InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture,
    InputEventScreenTouch, LineEdit, Window,
};
use godot::global::{Key, MouseButton};

#[derive(Default)]
pub(super) struct InputRegressionState {
    pub(super) stage: usize,
    phase: usize,
    frame: usize,
    stage_at: Option<Instant>,
    scroll_top: f64,
}

impl Runner {
    pub(super) fn attach_input_regression_node(&mut self, node: &Gd<Node>) -> Result<(), String> {
        if self.case != "inactive_pointer" {
            return self.attach_dialog_node(node);
        }
        let Some(mut root) = scene_tree().and_then(|tree| tree.get_root()) else {
            return Err("SceneTree root disappeared".into());
        };
        root.set_embedding_subwindows(true);
        let mut window = Window::new_alloc();
        window.set_visible(false);
        window.set_force_native(false);
        window.set("unfocusable", &true.to_variant());
        window.set_size(Vector2i::new(640, 360));
        window.set_position(Vector2i::ZERO);
        root.add_child(&window);
        window.add_child(node);
        window.show();
        window.notify_mouse_entered();
        self.check(
            window.is_embedded(),
            "Inactive input window must be embedded",
        );
        self.check(!window.has_focus(), "Inactive input window must lack focus");
        self.dialogs.window = Some(window);
        Ok(())
    }

    fn input_web(&self, kind: &str) -> Option<Value> {
        self.web_events
            .iter()
            .find(|event| event["type"] == kind && event["phase"] == self.input_regressions.phase)
            .cloned()
    }

    fn input_next(&mut self) {
        self.input_regressions.stage += 1;
        self.input_regressions.frame = self.process_frames;
        self.input_regressions.stage_at = Some(Instant::now());
    }

    fn input_key(&mut self, key: Key, unicode: u32) {
        for pressed in [true, false] {
            let mut event = InputEventKey::new_gd();
            event.set_keycode(key);
            event.set_physical_keycode(key);
            event.set_unicode(if pressed { unicode } else { 0 });
            event.set_pressed(pressed);
            if let Some(window) = &mut self.dialogs.window {
                window.push_input_ex(&event).in_local_coords(true).done();
            }
        }
    }

    pub(super) fn tick_inactive_pointer(&mut self) {
        match self.input_regressions.stage {
            0 if self.input_web("pointer_ready").is_some() => {
                let mut motion = InputEventMouseMotion::new_gd();
                motion.set_position(Vector2::new(50.0, 34.0));
                motion.set_global_position(Vector2::new(50.0, 34.0));
                if let Some(window) = &mut self.dialogs.window {
                    window.push_input_ex(&motion).in_local_coords(true).done();
                }
                self.input_next();
            }
            // The original bug sends mouse-leave from every internal process
            // frame. Sampling after several frames must preserve DOM :hover.
            1 if self.process_frames >= self.input_regressions.frame + 12 => {
                self.eval("window.pointerSnapshot('hover')");
                self.input_next();
            }
            2 => {
                let Some(hover) = self.input_web("pointer_hover") else {
                    return;
                };
                self.check(
                    hover["moves"].as_u64().is_some_and(|n| n > 0) && hover["hover"] == true,
                    format!(
                        "Inactive pointer motion must retain hover across process frames: {hover}"
                    ),
                );
                let mut event = InputEventMouseButton::new_gd();
                event.set_button_index(MouseButton::WHEEL_DOWN);
                event.set_pressed(true);
                event.set_position(Vector2::new(50.0, 34.0));
                event.set_global_position(Vector2::new(50.0, 34.0));
                if let Some(window) = &mut self.dialogs.window {
                    window.push_input_ex(&event).in_local_coords(true).done();
                }
                self.eval("window.pointerSnapshot('result')");
                self.input_next();
            }
            3 => {
                let Some(result) = self.input_web("pointer_result") else {
                    return;
                };
                self.check(result["wheels"] == 1 && result["scrollTop"].as_f64().is_some_and(|n| n > 0.0),
                    format!("Inactive wheel must deliver one trusted event and scroll the page: {result}"));
                self.check(
                    result["trusted"] == true && result["hover"] == true,
                    "Inactive pointer input must be trusted and retain hover after scrolling",
                );
                self.check(
                    self.dialogs
                        .window
                        .as_ref()
                        .is_some_and(|window| !window.has_focus()),
                    "Pointer input must not focus the inactive window",
                );
                self.input_regressions.scroll_top =
                    result["scrollTop"].as_f64().unwrap_or_default();
                let mut event = InputEventMouseButton::new_gd();
                event.set_button_index(MouseButton::WHEEL_RIGHT);
                event.set_pressed(true);
                event.set_position(Vector2::new(50.0, 34.0));
                event.set_global_position(Vector2::new(50.0, 34.0));
                if let Some(window) = &mut self.dialogs.window {
                    window.push_input_ex(&event).in_local_coords(true).done();
                }
                self.eval("window.pointerSnapshot('horizontal')");
                self.input_next();
            }
            4 => {
                let Some(result) = self.input_web("pointer_horizontal") else {
                    return;
                };
                self.check(
                    result["wheels"] == 2
                        && result["scrollLeft"].as_f64().is_some_and(|n| n != 500.0),
                    format!("Inactive horizontal wheel must scroll horizontally: {result}"),
                );
                let mut event = InputEventPanGesture::new_gd();
                event.set_position(Vector2::new(50.0, 34.0));
                event.set_delta(Vector2::new(0.0, 1.0));
                if let Some(window) = &mut self.dialogs.window {
                    window.push_input_ex(&event).in_local_coords(true).done();
                }
                self.eval("window.pointerSnapshot('pan')");
                self.input_next();
            }
            5 => {
                let Some(result) = self.input_web("pointer_pan") else {
                    return;
                };
                self.check(
                    result["wheels"] == 3
                        && result["scrollTop"]
                            .as_f64()
                            .is_some_and(|n| n > self.input_regressions.scroll_top),
                    format!("Inactive pan gesture must keep scrolling: {result}"),
                );
                self.check(
                    result["trusted"] == true && result["hover"] == true,
                    "Inactive horizontal scrolling and pan must preserve trusted hover",
                );
                // Godot touch emulation may change pointer hover independently
                // of CEF. Check activation rejection after passive hover samples.
                for pressed in [true, false] {
                    let mut event = InputEventMouseButton::new_gd();
                    event.set_button_index(MouseButton::LEFT);
                    event.set_pressed(pressed);
                    event.set_position(Vector2::new(50.0, 34.0));
                    event.set_global_position(Vector2::new(50.0, 34.0));
                    if let Some(window) = &mut self.dialogs.window {
                        window.push_input_ex(&event).in_local_coords(true).done();
                    }
                }
                for pressed in [true, false] {
                    let mut event = InputEventScreenTouch::new_gd();
                    event.set_position(Vector2::new(50.0, 34.0));
                    event.set_index(0);
                    event.set_pressed(pressed);
                    if let Some(window) = &mut self.dialogs.window {
                        window.push_input_ex(&event).in_local_coords(true).done();
                    }
                }
                self.input_key(Key::A, 'a' as u32);
                self.eval("window.pointerSnapshot('blocked')");
                self.input_next();
            }
            6 => {
                let Some(result) = self.input_web("pointer_blocked") else {
                    return;
                };
                self.check(
                    result["clicks"] == 0 && result["keys"] == 0 && result["touches"] == 0,
                    format!("Inactive clicks, touches and keys must remain blocked: {result}"),
                );
                self.check(
                    self.dialogs
                        .window
                        .as_ref()
                        .is_some_and(|window| !window.has_focus()),
                    "Blocked activation must not focus the inactive window",
                );
                self.stop();
            }
            _ => {}
        }
    }

    pub(super) fn tick_iframe_ime(&mut self) {
        const TARGETS: &[&str] = &[
            "root", "same", "cross", "sandbox", "cross", "root", "blank", "root",
        ];
        let phase = self.input_regressions.phase;
        let target = TARGETS[phase];
        match self.input_regressions.stage {
            0 if self
                .web_events
                .iter()
                .any(|event| event["type"] == "ime_ready") =>
            {
                self.eval(&format!(
                    "window.prepareIme({phase}, '{}')",
                    if target == "blank" { "cross" } else { target }
                ));
                self.input_next();
            }
            1 if self.input_web("ime_prepared").is_some() => {
                self.dialog_click(if target == "blank" {
                    Vector2::new(500.0, 280.0)
                } else {
                    Vector2::new(50.0, if target == "root" { 34.0 } else { 134.0 })
                });
                self.input_next();
            }
            2 => {
                if self.process_frames < self.input_regressions.frame + 12 {
                    return;
                }
                let proxy = self
                    .dialogs
                    .window
                    .as_ref()
                    .and_then(|window| window.gui_get_focus_owner())
                    .and_then(|control| control.try_cast::<LineEdit>().ok());
                if target == "blank" {
                    if let Some(result) = self.input_web("ime_noneditable")
                        && proxy.is_none()
                    {
                        self.check(result["editableActive"] == false,
                            "Clicking a plain cross-site frame must leave its editable node unfocused");
                        self.check(
                            true,
                            "Plain cross-site frame focus must release the old root IME proxy",
                        );
                        self.input_regressions = InputRegressionState {
                            phase: phase + 1,
                            ..Default::default()
                        };
                    } else if self
                        .input_regressions
                        .stage_at
                        .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
                    {
                        self.fail(format!("Plain cross-site frame must release the root IME proxy; DOM report: {:?}, proxy focused: {}", self.input_web("ime_noneditable"), proxy.is_some()));
                    }
                    return;
                }
                if proxy.is_none() || self.input_web("ime_focus").is_none() {
                    if self
                        .input_regressions
                        .stage_at
                        .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
                    {
                        self.fail(format!("Frame {target} phase {phase} failed to focus its editable node and IME proxy; DOM focus report: {:?}, proxy focused: {}", self.input_web("ime_focus"), proxy.is_some()));
                    }
                    return;
                }
                self.check(
                    true,
                    format!("Frame {target} must activate the native IME proxy"),
                );
                // Ordinary typing takes the complete Godot key -> LineEdit ->
                // production text_changed path, including duplicate detection.
                self.input_key(Key::A, 'a' as u32);
                self.eval(&format!("window.imeSnapshot({phase}, 'ascii')"));
                self.input_next();
            }
            3 => {
                let Some(result) = self.input_web("ime_ascii") else {
                    return;
                };
                self.check(
                    result["value"] == "a",
                    format!("Frame {target} must insert ASCII exactly once: {result}"),
                );
                let proxy = self
                    .dialogs
                    .window
                    .as_ref()
                    .and_then(|window| window.gui_get_focus_owner())
                    .and_then(|control| control.try_cast::<LineEdit>().ok());
                let Some(mut proxy) = proxy else {
                    self.fail("IME proxy lost focus before the committed text event");
                    return;
                };
                // Headless Godot has no OS candidate window. Deliver the same
                // LineEdit signal used for a committed composition, exercising
                // production CEF text routing with BMP and surrogate-pair text.
                proxy.emit_signal("text_changed", &["雪😀".to_variant()]);
                self.eval(&format!("window.imeSnapshot({phase}, 'unicode')"));
                self.input_next();
            }
            4 => {
                let Some(result) = self.input_web("ime_unicode") else {
                    return;
                };
                self.check(
                    result["value"] == "a雪😀" && result["focused"] == true,
                    format!("Frame {target} must preserve a Unicode commit exactly once: {result}"),
                );
                self.check(
                    result["inputs"].as_u64().is_some_and(|n| n >= 2) && result["trusted"] == true,
                    format!("Frame {target} must receive native trusted input events: {result}"),
                );
                self.input_key(Key::LEFT, 0);
                self.input_key(Key::BACKSPACE, 0);
                self.eval(&format!("window.imeSnapshot({phase}, 'editing')"));
                self.input_next();
            }
            5 => {
                let Some(result) = self.input_web("ime_editing") else {
                    return;
                };
                self.check(
                    result["value"] == "a😀",
                    format!("Frame {target} must retain arrow and Backspace editing: {result}"),
                );
                if !self.failures.is_empty() || phase + 1 == TARGETS.len() {
                    self.stop();
                } else {
                    self.input_regressions = InputRegressionState {
                        phase: phase + 1,
                        ..Default::default()
                    };
                }
            }
            _ => {}
        }
    }
}
