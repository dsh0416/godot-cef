//! Wheel default actions in same-site and out-of-process child frames.

use super::*;
use godot::classes::{InputEventMouseButton, InputEventMouseMotion};
use godot::global::MouseButton;

// Do not script-scroll after hide/show: that itself repairs the reported CEF bug.
const ACTIONS: &[(bool, &str)] = &[
    (false, "baseline"),
    (false, "hide_show"),
    (true, "baseline"),
    (true, "hide_show"),
    (true, "hide_show"),
    (true, "cancelled"),
];

#[derive(Default)]
pub(super) struct ScrollState {
    phase: usize,
    prepared: bool,
    sent: bool,
}

impl Runner {
    fn scroll_web(&self, kind: &str) -> Option<Value> {
        self.web_events
            .iter()
            .find(|event| event["type"] == kind && event["phase"] == self.scroll.phase)
            .cloned()
    }

    pub(super) fn tick_iframe_scroll(&mut self) {
        if !self
            .web_events
            .iter()
            .any(|event| event["type"] == "iframe_scroll_ready")
        {
            return;
        }
        let (cross_site, action) = ACTIONS[self.scroll.phase];
        if !self.scroll.prepared {
            self.scroll.prepared = true;
            self.eval(&format!(
                "window.prepareIframeScroll({}, {cross_site}, '{action}')",
                self.scroll.phase
            ));
            return;
        }
        let Some(ready) = self.scroll_web("iframe_scroll_prepared") else {
            return;
        };
        if !self.scroll.sent {
            self.check(
                ready["crossSite"] == cross_site && ready["scrollTop"] == 0,
                format!(
                    "Iframe wheel phase must start at the expected site and scroll offset: {ready}"
                ),
            );
            let position = Vector2::new(80.0, 80.0);
            let mut motion = InputEventMouseMotion::new_gd();
            motion.set_position(position);
            motion.set_global_position(position);
            let mut wheel = InputEventMouseButton::new_gd();
            wheel.set_position(position);
            wheel.set_global_position(position);
            wheel.set_button_index(MouseButton::WHEEL_DOWN);
            wheel.set_factor(1.0);
            wheel.set_pressed(true);
            if self.class == "CefTexture2D" {
                self.call(
                    "forward_mouse_motion_event",
                    &[
                        motion.to_variant(),
                        1.0_f32.to_variant(),
                        1.0_f32.to_variant(),
                    ],
                );
                self.call(
                    "forward_mouse_button_event",
                    &[
                        wheel.to_variant(),
                        1.0_f32.to_variant(),
                        1.0_f32.to_variant(),
                    ],
                );
            } else if let Some(window) = &mut self.dialogs.window {
                window.push_input_ex(&motion).in_local_coords(true).done();
                window.push_input_ex(&wheel).in_local_coords(true).done();
            }
            self.scroll.sent = true;
            return;
        }
        let Some(result) = self.scroll_web("iframe_scroll_result") else {
            return;
        };
        self.check(
            result["wheels"] == 1 && result["trusted"] == true && result["target"] == "content",
            format!(
                "Iframe {action} must receive exactly one trusted wheel on its scroller: {result}"
            ),
        );
        let offset = result["scrollTop"].as_f64().unwrap_or_default();
        self.check(
            if action == "cancelled" {
                offset == 0.0 && result["prevented"] == true
            } else {
                offset > 0.0 && result["prevented"] == false
            },
            format!("Iframe {action} (cross-site={cross_site}) must retain the native wheel default action: {result}"),
        );
        if self.scroll.phase + 1 == ACTIONS.len() {
            self.stop();
        } else {
            self.scroll = ScrollState {
                phase: self.scroll.phase + 1,
                ..Default::default()
            };
        }
    }
}
