//! Display callbacks and CEF visibility behavior through both public browser types.

use super::*;
use godot::classes::{Control, InputEventMouseMotion, Window};

const SETTLE: Duration = Duration::from_millis(500);
const HIDDEN_SAMPLE: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub(super) struct DisplayState {
    pub(super) signals: Vec<Value>,
    parent: Option<Gd<Control>>,
    pub(super) stage: usize,
    pub(super) detached: bool,
    stage_at: Option<Instant>,
    marker: usize,
}

impl Runner {
    pub(super) fn setup_display_signals(&mut self) -> Result<(), String> {
        for method in [
            "get_browser_hidden",
            "set_browser_hidden",
            "is_browser_hidden",
        ] {
            if !self
                .browser
                .as_ref()
                .is_some_and(|browser| browser.has_method(method))
            {
                return Err(format!("Missing public visibility method: {method}"));
            }
        }
        let manual = self
            .browser
            .as_ref()
            .and_then(|browser| browser.get("browser_hidden").try_to::<bool>().ok());
        self.check(
            manual == Some(false),
            "Browsers must default to rendering enabled",
        );
        if self.class == "CefTexture" {
            let automatic = self.browser.as_ref().and_then(|browser| {
                browser
                    .get("pause_rendering_when_hidden")
                    .try_to::<bool>()
                    .ok()
            });
            self.check(
                automatic == Some(false),
                "Node visibility rendering control must be opt-in",
            );
            for method in [
                "get_pause_rendering_when_hidden",
                "set_pause_rendering_when_hidden",
            ] {
                if !self
                    .browser
                    .as_ref()
                    .is_some_and(|browser| browser.has_method(method))
                {
                    return Err(format!("Missing public Node visibility method: {method}"));
                }
            }
        }
        for signal in [
            "tooltip_changed",
            "fullscreen_changed",
            "loading_progress_changed",
            "status_message_changed",
            "favicon_urls_changed",
        ] {
            if !self
                .browser
                .as_ref()
                .is_some_and(|browser| browser.has_signal(signal))
            {
                return Err(format!("Missing public display signal: {signal}"));
            }
            self.connect(signal)?;
        }
        Ok(())
    }

    pub(super) fn attach_display_node(&mut self, node: &Gd<Node>) -> Result<(), String> {
        let Some(mut root) = scene_tree().and_then(|tree| tree.get_root()) else {
            return Err("SceneTree root disappeared".into());
        };
        root.set_embedding_subwindows(true);
        let mut window = Window::new_alloc();
        window.set_visible(false);
        window.set_force_native(false);
        window.set_size(Vector2i::new(640, 360));
        window.set_position(Vector2i::ZERO);
        root.add_child(&window);
        self.dialogs.window = Some(window.clone());
        window.show();
        window.grab_focus();
        self.check(
            window.is_embedded(),
            "Display input window must remain embedded in headless Godot",
        );
        self.check(
            window.has_focus(),
            "Embedded display window must acquire logical focus",
        );
        let mut parent = Control::new_alloc();
        parent.set_size(Vector2::new(640.0, 360.0));
        window.add_child(&parent);
        parent.add_child(node);
        self.display.parent = Some(parent);
        window.notify_mouse_entered();
        self.call("grab_focus", &[]);
        Ok(())
    }

    pub(super) fn display_signal(&mut self, name: &str, args: &[Variant]) {
        self.check(
            args.len() == 1,
            format!("{name} must have exactly one argument"),
        );
        let Some(value) = args.first() else {
            return;
        };
        let value = match name {
            "tooltip_changed" | "status_message_changed" => {
                let text = value.try_to::<GString>();
                self.check(text.is_ok(), format!("{name} must carry a Godot string"));
                json!(text.ok().map(|text| text.to_string()))
            }
            "fullscreen_changed" => {
                let state = value.try_to::<bool>();
                self.check(state.is_ok(), "fullscreen_changed must carry a bool");
                json!(state.ok())
            }
            "loading_progress_changed" => {
                let progress = value.try_to::<f64>();
                self.check(
                    progress.as_ref().is_ok_and(|progress| {
                        progress.is_finite() && (0.0..=1.0).contains(progress)
                    }),
                    "Loading progress must be a finite number between zero and one",
                );
                json!(progress.ok())
            }
            "favicon_urls_changed" => {
                let urls = value.try_to::<PackedStringArray>();
                self.check(
                    urls.is_ok(),
                    "Favicon callback must carry PackedStringArray",
                );
                json!(urls.ok().map(|urls| {
                    urls.as_slice()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                }))
            }
            _ => {
                self.fail(format!("Unexpected display signal: {name}"));
                return;
            }
        };
        self.display
            .signals
            .push(json!({"name":name, "value":value}));
    }

    fn display_web(&self, kind: &str, phase: Option<&str>) -> Option<Value> {
        self.web_events
            .iter()
            .find(|event| {
                event["type"] == kind && phase.is_none_or(|phase| event["phase"] == phase)
            })
            .cloned()
    }

    fn display_has_signal(&self, name: &str, value: &Value) -> bool {
        self.display
            .signals
            .iter()
            .skip(self.display.marker)
            .any(|event| event["name"] == name && event["value"] == *value)
    }

    fn display_has_icon(&self, phase: &str) -> bool {
        self.display.signals.iter().any(|event| {
            event["name"] == "favicon_urls_changed"
                && event["value"].as_array().is_some_and(|urls| {
                    urls.iter().any(|url| {
                        url.as_str().is_some_and(|url| {
                            url.contains("/display-icon.svg?")
                                && url.contains(&format!("phase={phase}"))
                        })
                    })
                })
        })
    }

    fn display_next(&mut self) {
        self.display.stage += 1;
        self.display.stage_at = Some(Instant::now());
    }

    fn display_elapsed(&self, delay: Duration) -> bool {
        self.display
            .stage_at
            .is_some_and(|at| at.elapsed() >= delay)
    }

    fn display_hover(&mut self, position: Vector2) {
        let mut motion = InputEventMouseMotion::new_gd();
        motion.set_position(position);
        motion.set_global_position(position);
        if self.class == "CefTexture2D" {
            self.call(
                "forward_mouse_motion_event",
                &[
                    motion.to_variant(),
                    1.0_f32.to_variant(),
                    1.0_f32.to_variant(),
                ],
            );
        } else if let Some(window) = &mut self.dialogs.window {
            window.grab_focus();
            window.push_input_ex(&motion).in_local_coords(true).done();
        }
    }

    pub(super) fn tick_display_events(&mut self) {
        let Some(ready) = self.display_web("display_ready", None) else {
            return;
        };
        match self.display.stage {
            0 => {
                if !self.display_has_icon("initial")
                    || !self.display_has_signal("loading_progress_changed", &json!(1.0))
                {
                    return;
                }
                self.check(
                    self.display_web("display_loaded", None)
                        .is_some_and(|event| event["preload"] == true),
                    "Display fixture must execute preload before page script",
                );
                self.check(
                    ready["link_hit"] == "link" && ready["fullscreen_hit"] == "fullscreen",
                    "CEF must finish laying out the input targets",
                );
                self.check(
                    self.display.signals.iter().any(|event| {
                        event["name"] == "loading_progress_changed"
                            && event["value"].as_f64().is_some_and(|value| value < 1.0)
                    }),
                    "Real page loading must report incomplete progress before completion",
                );
                self.display.marker = self.display.signals.len();
                self.display_hover(Vector2::new(50.0, 34.0));
                self.display_next();
            }
            1 => {
                if !self.display_has_signal("tooltip_changed", &ready["tooltip"])
                    || !self.display_has_signal("status_message_changed", &ready["link_href"])
                {
                    return;
                }
                self.check(
                    ready["tooltip"] == "悬停提示: 雪 ✓",
                    "Tooltip callback must preserve the exact Unicode title",
                );
                self.display.marker = self.display.signals.len();
                self.display_hover(Vector2::new(600.0, 340.0));
                self.display_next();
            }
            2 => {
                if !self.display_has_signal("tooltip_changed", &json!(""))
                    || !self.display_has_signal("status_message_changed", &json!(""))
                {
                    return;
                }
                self.check(
                    true,
                    "Leaving the link must clear both tooltip and status text",
                );
                self.eval("window.changeIcon()");
                self.display_next();
            }
            3 => {
                if !self.display_has_icon("changed") {
                    return;
                }
                self.check(
                    true,
                    "Replacing rel=icon must deliver its updated absolute URL",
                );
                self.display.marker = self.display.signals.len();
                self.dialog_click(Vector2::new(50.0, 100.0));
                self.display_next();
            }
            4 => {
                if !self.display_has_signal("fullscreen_changed", &json!(true))
                    || !self.web_events.iter().any(|event| {
                        event["type"] == "display_fullscreen" && event["fullscreen"] == true
                    })
                {
                    return;
                }
                self.check(
                    self.display_web("display_click", None)
                        .is_some_and(|event| {
                            event["trusted"] == true && event["activated"] == true
                        }),
                    "Fullscreen must originate from a trusted browser mouse click",
                );
                self.eval("document.exitFullscreen().catch(error=>window.report({type:'fixture_error',message:String(error)}))");
                self.display_next();
            }
            5 => {
                if !self.display_has_signal("fullscreen_changed", &json!(false))
                    || !self.web_events.iter().any(|event| {
                        event["type"] == "display_fullscreen" && event["fullscreen"] == false
                    })
                {
                    return;
                }
                self.check(true, "CEF and the DOM must both report exiting fullscreen");
                self.stop();
            }
            _ => self.fail("Unexpected display-event stage"),
        }
    }

    fn effective_hidden(&mut self) -> bool {
        let result = self.call("is_browser_hidden", &[]).try_to::<bool>();
        self.check(result.is_ok(), "is_browser_hidden must return bool");
        result.unwrap_or(false)
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.call("set_browser_hidden", &[hidden.to_variant()]);
        let getter = self.call("get_browser_hidden", &[]).try_to::<bool>();
        let property = self
            .browser
            .as_ref()
            .and_then(|browser| browser.get("browser_hidden").try_to::<bool>().ok());
        self.check(
            getter.is_ok_and(|value| value == hidden) && property == Some(hidden),
            "Manual browser visibility must round-trip through methods and property",
        );
    }

    fn snapshot(&mut self, phase: &str, after_frame: bool) {
        self.eval(&format!(
            "window.{}('{phase}')",
            if after_frame {
                "afterFrame"
            } else {
                "snapshot"
            }
        ));
    }

    fn check_hidden_snapshot(&mut self, snapshot: &Value, expected: bool) {
        let effective = self.effective_hidden();
        self.check(
            effective == expected,
            "Effective CEF visibility must match manual and Node visibility controls",
        );
        self.check(
            snapshot["hidden"] == expected
                && snapshot["visibility"] == if expected { "hidden" } else { "visible" },
            "Real DOM visibility must match the browser's effective hidden state",
        );
    }

    fn check_paused_frames(&mut self, before_phase: &str, after: &Value) {
        let Some(before) = self.display_web("display_snapshot", Some(before_phase)) else {
            self.fail("Hidden rendering sample omitted its baseline");
            return;
        };
        self.check_hidden_snapshot(&before, true);
        self.check_hidden_snapshot(after, true);
        self.check(
            before["frames"] == after["frames"],
            "CEF animation frames must stop after hiding and draining in-flight frames",
        );
        self.check(
            before["ticks"]
                .as_u64()
                .zip(after["ticks"].as_u64())
                .is_some_and(|(before, after)| after > before),
            "JavaScript timers must keep progressing while browser rendering is hidden",
        );
    }

    pub(super) fn tick_browser_hidden(&mut self) {
        match self.display.stage {
            0 => {
                let Some(loaded) = self.display_web("display_loaded", None) else {
                    return;
                };
                self.check(
                    loaded["preload"] == true,
                    "An initially hidden browser must load and run preload",
                );
                self.display_next();
            }
            1 if self.display_elapsed(SETTLE) => {
                self.snapshot("initial_before", false);
                self.display_next();
            }
            2 if self
                .display_web("display_snapshot", Some("initial_before"))
                .is_some()
                && self.display_elapsed(HIDDEN_SAMPLE) =>
            {
                self.snapshot("initial_after", false);
                self.display_next();
            }
            3 => {
                let Some(after) = self.display_web("display_snapshot", Some("initial_after"))
                else {
                    return;
                };
                self.check_paused_frames("initial_before", &after);
                self.check(
                    self.display_has_signal("loading_progress_changed", &json!(1.0)),
                    "Initially hidden browser must still deliver loading progress",
                );
                self.set_hidden(false);
                self.snapshot("first_shown", true);
                self.display_next();
            }
            4 => {
                let Some(shown) = self.display_web("display_snapshot", Some("first_shown")) else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                self.check(
                    shown["frames"].as_u64().is_some_and(|frames| {
                        frames
                            > self
                                .display_web("display_snapshot", Some("initial_after"))
                                .and_then(|event| event["frames"].as_u64())
                                .unwrap_or(u64::MAX)
                    }),
                    "Showing an initially hidden browser must resume animation frames",
                );
                self.set_hidden(true);
                self.eval("window.hiddenEvaluation=6*7");
                self.display_next();
            }
            5 if self.display_elapsed(SETTLE) => {
                self.snapshot("manual_before", false);
                self.display_next();
            }
            6 if self
                .display_web("display_snapshot", Some("manual_before"))
                .is_some()
                && self.display_elapsed(HIDDEN_SAMPLE) =>
            {
                self.snapshot("manual_after", false);
                self.display_next();
            }
            7 => {
                let Some(after) = self.display_web("display_snapshot", Some("manual_after")) else {
                    return;
                };
                self.check_paused_frames("manual_before", &after);
                self.check(
                    after["evaluated"] == true,
                    "eval must still execute while rendering is hidden",
                );
                self.set_hidden(false);
                self.snapshot("manual_shown", true);
                self.display_next();
            }
            8 => {
                let Some(shown) = self.display_web("display_snapshot", Some("manual_shown")) else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                if self.class == "CefTexture2D" {
                    self.stop();
                    return;
                }
                self.call("hide", &[]);
                self.display_next();
            }
            9 if self.display_elapsed(SETTLE) => {
                self.snapshot("opt_out_hidden", true);
                self.display_next();
            }
            10 => {
                let Some(shown) = self.display_web("display_snapshot", Some("opt_out_hidden"))
                else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                let visible = self.call("is_visible_in_tree", &[]).try_to::<bool>();
                self.check(
                    visible.is_ok_and(|visible| !visible),
                    "Node must actually be invisible when checking the default opt-out behavior",
                );
                self.call("show", &[]);
                self.call("set_pause_rendering_when_hidden", &[true.to_variant()]);
                let enabled = self
                    .call("get_pause_rendering_when_hidden", &[])
                    .try_to::<bool>();
                self.check(
                    enabled.is_ok_and(|enabled| enabled),
                    "Automatic Node visibility control must round-trip through its public methods",
                );
                self.call("hide", &[]);
                self.display_next();
            }
            11 if self.display_elapsed(SETTLE) => {
                self.snapshot("auto_before", false);
                self.display_next();
            }
            12 if self
                .display_web("display_snapshot", Some("auto_before"))
                .is_some()
                && self.display_elapsed(HIDDEN_SAMPLE) =>
            {
                self.snapshot("auto_after", false);
                self.display_next();
            }
            13 => {
                let Some(after) = self.display_web("display_snapshot", Some("auto_after")) else {
                    return;
                };
                self.check_paused_frames("auto_before", &after);
                self.call("show", &[]);
                self.snapshot("auto_shown", true);
                self.display_next();
            }
            14 => {
                let Some(shown) = self.display_web("display_snapshot", Some("auto_shown")) else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                if let Some(parent) = &mut self.display.parent {
                    parent.hide();
                }
                self.display_next();
            }
            15 if self.display_elapsed(SETTLE) => {
                self.snapshot("parent_hidden", false);
                self.display_next();
            }
            16 => {
                let Some(hidden) = self.display_web("display_snapshot", Some("parent_hidden"))
                else {
                    return;
                };
                self.check_hidden_snapshot(&hidden, true);
                if let Some(parent) = &mut self.display.parent {
                    parent.show();
                }
                self.snapshot("parent_shown", true);
                self.display_next();
            }
            17 => {
                let Some(shown) = self.display_web("display_snapshot", Some("parent_shown")) else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                let node = self
                    .browser
                    .as_ref()
                    .and_then(|browser| browser.clone().try_cast::<Node>().ok());
                if let (Some(parent), Some(node)) = (&mut self.display.parent, node) {
                    parent.remove_child(&node);
                    self.display.detached = true;
                } else {
                    self.fail("Could not detach browser Node for visibility reentry test");
                    return;
                }
                let inside = self.call("is_inside_tree", &[]).try_to::<bool>();
                self.check(
                    inside.is_ok_and(|inside| !inside),
                    "Detached browser must actually leave the SceneTree",
                );
                self.display_next();
            }
            18 if self.display_elapsed(SETTLE) => {
                self.snapshot("tree_before", false);
                self.display_next();
            }
            19 if self
                .display_web("display_snapshot", Some("tree_before"))
                .is_some()
                && self.display_elapsed(HIDDEN_SAMPLE) =>
            {
                self.snapshot("tree_after", false);
                self.display_next();
            }
            20 => {
                let Some(after) = self.display_web("display_snapshot", Some("tree_after")) else {
                    return;
                };
                self.check_paused_frames("tree_before", &after);
                let node = self
                    .browser
                    .as_ref()
                    .and_then(|browser| browser.clone().try_cast::<Node>().ok());
                if let (Some(parent), Some(node)) = (&mut self.display.parent, node) {
                    parent.add_child(&node);
                    self.display.detached = false;
                } else {
                    self.fail("Could not reattach browser Node for visibility reentry test");
                    return;
                }
                self.snapshot("tree_reentered", true);
                self.display_next();
            }
            21 => {
                let Some(shown) = self.display_web("display_snapshot", Some("tree_reentered"))
                else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                let inside = self.call("is_inside_tree", &[]).try_to::<bool>();
                self.check(
                    inside.is_ok_and(|inside| inside),
                    "Reattached browser must reenter the SceneTree",
                );
                self.check(
                    self.web_events
                        .iter()
                        .filter(|event| event["type"] == "display_loaded")
                        .count()
                        == 1,
                    "Tree reentry must resume the original browser instead of recreating its page",
                );
                self.set_hidden(true);
                self.call("set_pause_rendering_when_hidden", &[false.to_variant()]);
                self.display_next();
            }
            22 if self.display_elapsed(SETTLE) => {
                self.snapshot("manual_override", false);
                self.display_next();
            }
            23 => {
                let Some(hidden) = self.display_web("display_snapshot", Some("manual_override"))
                else {
                    return;
                };
                self.check_hidden_snapshot(&hidden, true);
                self.set_hidden(false);
                self.snapshot("final_shown", true);
                self.display_next();
            }
            24 => {
                let Some(shown) = self.display_web("display_snapshot", Some("final_shown")) else {
                    return;
                };
                self.check_hidden_snapshot(&shown, false);
                self.stop();
            }
            1 | 2 | 5 | 6 | 9 | 11 | 12 | 15 | 18 | 19 | 22 => {}
            _ => self.fail("Unexpected hidden-browser stage"),
        }
    }
}
