//! Real dialog callbacks and page results, driven through the registered Godot API.

use super::*;
use godot::classes::{InputEventMouseButton, InputEventMouseMotion, Window};
use godot::global::{MouseButton, MouseButtonMask};
use std::collections::BTreeSet;

const FIRST_TEXT: &str = "gdcef dialog fixture: 雪 ✓\n";
const SECOND_TEXT: &str = "gdcef second file: λ\n";

#[derive(Default)]
pub(super) struct DialogState {
    pub(super) requests: Vec<Value>,
    pub(super) finished: BTreeMap<i64, String>,
    answered: BTreeSet<i64>,
    requested_at: BTreeMap<i64, Instant>,
    finished_at: BTreeMap<i64, Instant>,
    old_ids: Vec<i64>,
    step: usize,
    stage: usize,
    recreate_at: Option<Instant>,
    unhandled_ids: BTreeMap<usize, i64>,
    pub(super) window: Option<Gd<Window>>,
}

impl Runner {
    pub(super) fn attach_dialog_node(&mut self, node: &Gd<Node>) -> Result<(), String> {
        let Some(mut root) = scene_tree().and_then(|tree| tree.get_root()) else {
            return Err("SceneTree root disappeared".into());
        };
        // Headless root windows have no native OS focus. An embedded Window
        // supports logical focus without creating a desktop window, allowing
        // CefTexture's normal public GUI input path and focus gate to run.
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
            "Dialog input window must remain embedded in headless Godot",
        );
        self.check(
            window.has_focus(),
            "Embedded dialog window must acquire logical focus",
        );
        window.add_child(node);
        window.notify_mouse_entered();
        if let Some(browser) = &mut self.browser {
            browser.call("grab_focus", &[]);
        }
        Ok(())
    }

    pub(super) fn setup_dialog_signals(&mut self) -> Result<(), String> {
        for method in [
            "continue_file_dialog",
            "cancel_file_dialog",
            "continue_js_dialog",
            "cancel_js_dialog",
            "is_dialog_pending",
        ] {
            let exists = self
                .browser
                .as_ref()
                .is_some_and(|browser| browser.has_method(method));
            self.check(exists, format!("Missing public dialog method: {method}"));
        }
        for signal in [
            "file_dialog_requested",
            "js_dialog_requested",
            "dialog_request_finished",
        ] {
            if !self
                .browser
                .as_ref()
                .is_some_and(|browser| browser.has_signal(signal))
            {
                return Err(format!("Missing public dialog signal: {signal}"));
            }
        }
        self.connect("dialog_request_finished")?;
        if self.case == "dialog_reentrant" {
            let Some(browser) = &mut self.browser else {
                return Err("No browser".into());
            };
            let instance = browser.instance_id();
            let generation = self.generation;
            // Capture the instance ID, not a strong reference which would retain a Resource.
            // This case intentionally resolves inside signal dispatch; all other cases defer.
            let immediate = Callable::from_fn("gdcef_itest_immediate_dialog", move |args| {
                let id = args
                    .first()
                    .and_then(|arg| arg.try_to::<Gd<Object>>().ok())
                    .map(|info| info.get("request_id"))
                    .and_then(|id| id.try_to::<i64>().ok());
                let mut values: Vec<Variant> = args.iter().map(|arg| (*arg).clone()).collect();
                let accepted = id
                    .and_then(|id| {
                        Gd::<Object>::try_from_instance_id(instance)
                            .ok()
                            .map(|mut browser| {
                                browser.call(
                                    "continue_js_dialog",
                                    &[id.to_variant(), true.to_variant(), "即时 ✓".to_variant()],
                                )
                            })
                    })
                    .and_then(|value| value.try_to::<bool>().ok())
                    .unwrap_or(false);
                values.push(accepted.to_variant());
                EVENTS.with(|queue| {
                    queue.borrow_mut().push(Event {
                        name: "dialog_immediate",
                        generation,
                        args: values,
                    })
                });
            });
            let error = browser.connect("js_dialog_requested", &immediate);
            if error != Error::OK {
                return Err(format!("Immediate dialog connection failed: {error:?}"));
            }
            self.connect("file_dialog_requested")?;
        } else if self.case != "dialog_unhandled" || self.dialogs.step >= 2 {
            self.connect("file_dialog_requested")?;
            self.connect("js_dialog_requested")?;
        }
        for method in [
            "is_dialog_pending",
            "cancel_file_dialog",
            "cancel_js_dialog",
        ] {
            let value = self.call_bool(method, -1);
            self.check(
                !value,
                format!("Unknown dialog ID must return false from {method}"),
            );
        }
        for key in ["GDCEF_ITEST_DIALOG_FILE", "GDCEF_ITEST_DIALOG_FILE_SECOND"] {
            let path = std::env::var(key).unwrap_or_default();
            self.check(
                std::path::Path::new(&path).is_absolute() && std::path::Path::new(&path).is_file(),
                format!("Harness must provide an absolute existing file via {key}"),
            );
        }
        Ok(())
    }

    pub(super) fn dialog_requested(&mut self, signal: &str, args: &[Variant]) {
        let Some(info) = args
            .first()
            .and_then(|value| value.try_to::<Gd<Object>>().ok())
        else {
            self.fail("Dialog signal must carry its public request-info object");
            return;
        };
        let file = signal == "file_dialog_requested";
        self.check(
            info.get_class()
                == if file {
                    "FileDialogRequestInfo"
                } else {
                    "JsDialogRequestInfo"
                },
            "Dialog DTO must have the public registered class",
        );
        let Some(id) = info.get("request_id").try_to::<i64>().ok() else {
            self.fail("Dialog DTO omitted integer request_id");
            return;
        };
        self.check(id > 0, "Dialog request IDs must be positive");
        self.check(
            !self
                .dialogs
                .requests
                .iter()
                .any(|request| request["id"] == id),
            "Dialog request IDs must be unique across file and JS dialogs",
        );
        self.check(
            !self.dialogs.old_ids.contains(&id),
            "Recreated browser must issue fresh dialog IDs",
        );
        let mut value = json!({"id": id, "generation": self.generation, "phase": self.dialogs.step, "frame": self.process_frames});
        if file {
            value["kind"] = json!("file");
            value["mode"] = json!(info.get("mode").try_to::<i64>().ok());
            for name in ["title", "default_file_path"] {
                let text = info.get(name).try_to::<GString>();
                self.check(text.is_ok(), format!("File DTO {name} must be a string"));
                value[name] = json!(text.ok().map(|text| text.to_string()));
            }
            for name in ["accept_filters", "accept_extensions", "accept_descriptions"] {
                let strings = info.get(name).try_to::<PackedStringArray>();
                self.check(
                    strings.is_ok(),
                    format!("File DTO {name} must be PackedStringArray"),
                );
                value[name] = json!(
                    strings
                        .ok()
                        .map(|values| values
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>())
                        .unwrap_or_default()
                );
            }
        } else {
            for name in ["kind", "origin_url", "message_text", "default_prompt_text"] {
                let text = info.get(name).try_to::<GString>();
                self.check(text.is_ok(), format!("JS DTO {name} must be a string"));
                value[name] = json!(text.ok().map(|text| text.to_string()));
            }
            let is_reload = info.get("is_reload").try_to::<bool>();
            self.check(is_reload.is_ok(), "JS DTO is_reload must be a bool");
            value["is_reload"] = json!(is_reload.ok());
            if value["kind"] != "before_unload" {
                self.check(
                    value["origin_url"]
                        .as_str()
                        .is_some_and(|url| url.starts_with(&self.origin)),
                    "JS dialog must identify the loopback fixture origin",
                );
            }
        }
        self.dialogs.requested_at.insert(id, Instant::now());
        if signal == "dialog_immediate" {
            self.check(args.get(1).is_some_and(|value| value.try_to::<bool>().is_ok_and(|value| value)),
                "Same-instance continuation must succeed inside a signal callback without reentrant binding failure");
            self.dialogs.answered.insert(id);
        }
        self.dialogs.requests.push(value);
    }

    pub(super) fn dialog_finished(&mut self, args: &[Variant]) {
        let Some(id) = args.first().and_then(|value| value.try_to::<i64>().ok()) else {
            self.fail("Invalid dialog completion ID");
            return;
        };
        let Some(result) = string_arg(args, 1) else {
            self.fail("Invalid dialog completion result");
            return;
        };
        self.check(id > 0, "Dialog completion IDs must be positive");
        self.check(
            !self.dialogs.finished.contains_key(&id),
            "Each dialog ID must finish exactly once",
        );
        self.dialogs.finished.insert(id, result);
        self.dialogs.finished_at.insert(id, Instant::now());
        if self.case == "dialog_unhandled" && self.dialogs.step < 2 {
            self.dialogs.unhandled_ids.insert(self.dialogs.step, id);
        }
    }

    fn dialog_web(&self, name: &str, phase: Option<usize>) -> Option<Value> {
        self.web_events
            .iter()
            .find(|event| {
                event["type"] == name
                    && (event["generation"].is_null() || event["generation"] == self.generation)
                    && phase.is_none_or(|phase| event["phase"] == phase)
            })
            .cloned()
    }

    fn current_dialog(&self) -> Option<Value> {
        self.dialogs
            .requests
            .iter()
            .rev()
            .find(|request| {
                request["generation"] == self.generation && request["phase"] == self.dialogs.step
            })
            .cloned()
    }

    fn dialog_id(&self) -> Option<i64> {
        self.current_dialog()?.get("id")?.as_i64()
    }

    fn check_dialog_pending(&mut self, id: i64, file: bool) {
        let pending = self.call_bool("is_dialog_pending", id);
        self.check(
            pending,
            "Signal request must remain pending until explicitly completed",
        );
        let wrong_cancel = self.call_bool(
            if file {
                "cancel_js_dialog"
            } else {
                "cancel_file_dialog"
            },
            id,
        );
        self.check(!wrong_cancel, "Wrong-kind cancellation must return false");
        let wrong_continue = if file {
            self.call(
                "continue_js_dialog",
                &[id.to_variant(), true.to_variant(), "wrong".to_variant()],
            )
        } else {
            self.call(
                "continue_file_dialog",
                &[id.to_variant(), self.dialog_paths(false).to_variant()],
            )
        };
        self.check(
            wrong_continue.try_to::<bool>().is_ok_and(|value| !value),
            "Wrong-kind continuation must return false",
        );
        let pending = self.call_bool("is_dialog_pending", id);
        self.check(
            pending,
            "Wrong-kind decisions must leave the request pending",
        );
    }

    fn check_dialog_stale(&mut self, id: i64) {
        for method in [
            "is_dialog_pending",
            "cancel_file_dialog",
            "cancel_js_dialog",
        ] {
            let value = self.call_bool(method, id);
            self.check(
                !value,
                format!("Finished dialog ID must return false from {method}"),
            );
        }
        for (method, args) in [
            (
                "continue_file_dialog",
                vec![id.to_variant(), self.dialog_paths(false).to_variant()],
            ),
            (
                "continue_js_dialog",
                vec![id.to_variant(), true.to_variant(), "stale".to_variant()],
            ),
        ] {
            let value = self.call(method, &args);
            self.check(
                value.try_to::<bool>().is_ok_and(|value| !value),
                format!("Finished dialog ID must return false from {method}"),
            );
        }
    }

    fn dialog_paths(&self, multiple: bool) -> PackedStringArray {
        let mut paths = PackedStringArray::new();
        paths.push(&std::env::var("GDCEF_ITEST_DIALOG_FILE").unwrap_or_default());
        if multiple {
            paths.push(&std::env::var("GDCEF_ITEST_DIALOG_FILE_SECOND").unwrap_or_default());
        }
        paths
    }

    fn answer_js(&mut self, id: i64, accepted: bool, text: &str) {
        self.check_dialog_pending(id, false);
        let value = self.call(
            "continue_js_dialog",
            &[id.to_variant(), accepted.to_variant(), text.to_variant()],
        );
        self.check(
            value.try_to::<bool>().is_ok_and(|value| value),
            "JS continuation must be accepted once",
        );
        self.dialogs.answered.insert(id);
        self.check_dialog_stale(id);
    }

    fn check_dialog_result(&mut self, id: i64, expected: &str) -> bool {
        let Some(result) = self.dialogs.finished.get(&id).cloned() else {
            return false;
        };
        self.check(
            result == expected,
            format!("Dialog {id} must finish as {expected}, received {result}"),
        );
        self.check_dialog_stale(id);
        true
    }

    fn next_dialog_step(&mut self) {
        self.dialogs.step += 1;
        self.dialogs.stage = 0;
    }

    fn begin_js_step(&mut self, kind: &str) {
        if self.dialogs.stage == 0 {
            self.dialogs.stage = 1;
            self.eval(&format!(
                "window.runJsFixture('{}',{})",
                kind, self.dialogs.step
            ));
        }
    }

    fn begin_file_step(&mut self, multiple: bool) {
        if self.dialogs.stage == 0 {
            self.dialogs.stage = 1;
            self.eval(&format!(
                "window.prepareFile({}, {multiple})",
                self.dialogs.step
            ));
        }
        if self.dialogs.stage == 1
            && self
                .dialog_web("file_prepared", Some(self.dialogs.step))
                .is_some()
        {
            self.dialogs.stage = 2;
            self.dialog_click(Vector2::new(50.0, 34.0));
        }
    }

    fn dialog_click(&mut self, position: Vector2) {
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
        for pressed in [true, false] {
            let mut event = InputEventMouseButton::new_gd();
            event.set_button_index(MouseButton::LEFT);
            event.set_pressed(pressed);
            if pressed {
                event.set_button_mask(MouseButtonMask::LEFT);
            }
            event.set_position(position);
            event.set_global_position(position);
            if self.class == "CefTexture2D" {
                self.call(
                    "forward_mouse_button_event",
                    &[
                        event.to_variant(),
                        1.0_f32.to_variant(),
                        1.0_f32.to_variant(),
                    ],
                );
            } else if let Some(window) = &mut self.dialogs.window {
                window.push_input_ex(&event).in_local_coords(true).done();
            }
        }
    }

    pub(super) fn tick_dialogs(&mut self) {
        if self.case == "dialog_lifecycle" && self.dialogs.recreate_at.is_some() {
            self.tick_dialog_lifecycle();
            return;
        }
        if self.dialog_web("dialog_ready", None).is_none() {
            return;
        }
        match self.case.as_str() {
            "dialog_js" => self.tick_dialog_js(),
            "dialog_file" => self.tick_dialog_file(),
            "dialog_timeout" => self.tick_dialog_timeout(),
            "dialog_unhandled" => self.tick_dialog_unhandled(),
            "dialog_navigation" | "dialog_policy" => self.tick_dialog_invalidation(),
            "dialog_lifecycle" => self.tick_dialog_lifecycle(),
            "dialog_before_unload" => self.tick_dialog_before_unload(),
            "dialog_reentrant" => self.tick_dialog_reentrant(),
            _ => self.fail("Unknown dialog case"),
        }
    }
}

impl Runner {
    fn tick_dialog_js(&mut self) {
        let decisions = [
            ("alert", true, "", json!("returned")),
            ("confirm", true, "", json!(true)),
            ("confirm", false, "", json!(false)),
            ("prompt", true, "用户输入 λ ✓", json!("用户输入 λ ✓")),
            ("prompt", true, "", json!("")),
            ("prompt", false, "", Value::Null),
        ];
        let Some((kind, accepted, text, expected)) = decisions.get(self.dialogs.step) else {
            self.check(
                self.dialogs.requests.len() == decisions.len(),
                "JS fixtures must each emit one dialog request",
            );
            self.check(
                self.dialogs.finished.len() == decisions.len(),
                "JS fixtures must each finish once",
            );
            self.stop();
            return;
        };
        self.begin_js_step(kind);
        let Some(request) = self.current_dialog() else {
            return;
        };
        let Some(id) = request["id"].as_i64() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.check(
                request["kind"] == *kind,
                "JS dialog kind must match alert/confirm/prompt",
            );
            self.check(
                request["message_text"] == format!("dialog-{}: 雪 ✓", self.dialogs.step),
                "JS dialog message must preserve Unicode",
            );
            self.check(
                request["is_reload"] == false,
                "Ordinary JS dialogs must not be marked as reload",
            );
            if *kind == "prompt" {
                self.check(
                    request["default_prompt_text"] == "default 雪",
                    "Prompt default must preserve Unicode",
                );
            }
            if self.dialogs.step == 5 {
                self.check_dialog_pending(id, false);
                let canceled = self.call_bool("cancel_js_dialog", id);
                self.check(canceled, "Explicit JS cancellation must be accepted once");
                self.dialogs.answered.insert(id);
            } else {
                self.answer_js(id, *accepted, text);
            }
        }
        let Some(event) = self.dialog_web("js_result", Some(self.dialogs.step)) else {
            return;
        };
        if !self.check_dialog_result(id, if *accepted { "accepted" } else { "canceled" }) {
            return;
        }
        self.check(
            event["value"] == *expected,
            format!("{kind} must return its exact JavaScript value"),
        );
        self.next_dialog_step();
    }

    fn tick_dialog_file(&mut self) {
        let step = self.dialogs.step;
        if step >= 4 {
            self.check(self.dialogs.requests.len() == 4 && self.dialogs.finished.len() == 4,
                "Single, multiple, explicit-cancel and empty-selection file dialogs must each complete once");
            self.stop();
            return;
        }
        self.begin_file_step(step == 1);
        let Some(request) = self.current_dialog() else {
            return;
        };
        let Some(id) = request["id"].as_i64() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.check(
                request["kind"] == "file",
                "Actual HTML file input must produce a file dialog",
            );
            // CEF modes: OPEN=0, OPEN_MULTIPLE=1, OPEN_FOLDER=2, SAVE=3.
            self.check(
                request["mode"] == i64::from(step == 1),
                "File dialog mode must distinguish single from multiple",
            );
            let filters = request["accept_filters"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut filter_names: Vec<_> = filters.iter().filter_map(Value::as_str).collect();
            filter_names.sort_unstable();
            self.check(
                filter_names == [".txt", "text/plain"],
                "File DTO must preserve both distinct HTML accept filters",
            );
            for name in ["accept_extensions", "accept_descriptions"] {
                self.check(
                    request[name].as_array().is_some_and(|values| {
                        values.len() == filters.len() && values.iter().all(Value::is_string)
                    }),
                    format!(
                        "File DTO {name} must preserve an aligned entry for each accept filter"
                    ),
                );
            }
            // CEF may invoke a custom handler before MIME expansion. In that case
            // its extension/description entries are empty strings, not missing entries.
            self.check_dialog_pending(id, true);
            let value = match step {
                2 => self.call("cancel_file_dialog", &[id.to_variant()]),
                3 => self.call(
                    "continue_file_dialog",
                    &[id.to_variant(), PackedStringArray::new().to_variant()],
                ),
                _ => self.call(
                    "continue_file_dialog",
                    &[id.to_variant(), self.dialog_paths(step == 1).to_variant()],
                ),
            };
            self.check(
                value.try_to::<bool>().is_ok_and(|value| value),
                "File decision must be accepted once",
            );
            self.dialogs.answered.insert(id);
            self.check_dialog_stale(id);
        }
        if step < 2 {
            let Some(event) = self.dialog_web("file_result", Some(step)) else {
                return;
            };
            if !self.check_dialog_result(id, "accepted") {
                return;
            }
            self.check_file_contents(&event, step == 1);
        } else {
            if self.dialog_web("file_canceled", Some(step)).is_none() {
                return;
            }
            if !self.check_dialog_result(id, "canceled") {
                return;
            }
        }
        self.next_dialog_step();
    }

    fn check_file_contents(&mut self, event: &Value, multiple: bool) {
        let files = event["files"].as_array().cloned().unwrap_or_default();
        self.check(
            files.len() == if multiple { 2 } else { 1 },
            "CEF must populate the actual HTML FileList",
        );
        self.check(files.first().is_some_and(|file| file["name"] == "selected-unicode-雪.txt" && file["text"] == FIRST_TEXT),
            "Page FileReader must read the selected real file and preserve its Unicode name and UTF-8 contents");
        if multiple {
            self.check(
                files.get(1).is_some_and(|file| {
                    file["name"] == "selected-second.txt" && file["text"] == SECOND_TEXT
                }),
                "Multiple selection must read both actual files in the supplied order",
            );
        }
    }

    fn tick_dialog_timeout(&mut self) {
        let step = self.dialogs.step;
        if step >= 2 {
            self.stop();
            return;
        }
        if step == 0 {
            self.begin_js_step("prompt");
        } else {
            self.begin_file_step(false);
        }
        let Some(id) = self.dialog_id() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.check_dialog_pending(id, step == 1);
            self.dialogs.answered.insert(id); // Mark inspected; intentionally never decide.
        }
        let terminal = if step == 0 {
            self.dialog_web("js_result", Some(step))
        } else {
            self.dialog_web("file_canceled", Some(step))
        };
        let Some(event) = terminal else {
            return;
        };
        if !self.check_dialog_result(id, "timed_out") {
            return;
        }
        let elapsed = self
            .dialogs
            .requested_at
            .get(&id)
            .zip(self.dialogs.finished_at.get(&id))
            .map(|(start, end)| end.duration_since(*start));
        self.check(elapsed.is_some_and(|elapsed| elapsed >= Duration::from_millis(700) && elapsed < Duration::from_secs(5)),
            "One-second dialog deadline must cancel asynchronously, without waiting for the outer test deadline");
        if step == 0 {
            self.check(
                event["value"].is_null(),
                "Timed-out prompt must return null",
            );
        }
        self.next_dialog_step();
    }

    fn tick_dialog_unhandled(&mut self) {
        let step = self.dialogs.step;
        if step < 2 {
            if step == 0 {
                self.begin_js_step("prompt");
            } else {
                self.begin_file_step(false);
            }
            let Some(id) = self.dialogs.unhandled_ids.get(&step).copied() else {
                return;
            };
            let terminal = if step == 0 {
                self.dialog_web("js_result", Some(step))
            } else {
                self.dialog_web("file_canceled", Some(step))
            };
            let Some(event) = terminal else {
                return;
            };
            self.check(
                self.dialogs.requests.is_empty(),
                "No listener must receive no requested signal",
            );
            if !self.check_dialog_result(id, "unhandled") {
                return;
            }
            if step == 0 {
                self.check(
                    event["value"].is_null(),
                    "Unhandled prompt must return null",
                );
            }
            self.dialogs.old_ids.push(id);
            self.next_dialog_step();
            if self.dialogs.step == 2 {
                for signal in ["file_dialog_requested", "js_dialog_requested"] {
                    if let Err(error) = self.connect(signal) {
                        self.fail(error);
                        return;
                    }
                }
            }
            return;
        }
        self.begin_js_step("prompt");
        let Some(id) = self.dialog_id() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.answer_js(id, true, "retry ✓");
        }
        let Some(event) = self.dialog_web("js_result", Some(step)) else {
            return;
        };
        if !self.check_dialog_result(id, "accepted") {
            return;
        }
        self.check(
            event["value"] == "retry ✓",
            "Adding a listener must allow a fresh dialog in the same browser",
        );
        self.check(self.dialogs.requests.len() == 1 && self.dialogs.finished.len() == 3,
            "Unhandled requests and subsequent handled retry must have distinct exactly-once completions");
        self.stop();
    }

    fn tick_dialog_invalidation(&mut self) {
        if self.dialogs.step == 0 {
            self.begin_js_step("prompt");
            let Some(id) = self.dialog_id() else {
                return;
            };
            if self.dialogs.stage == 1 {
                self.check_dialog_pending(id, false);
                self.dialogs.stage = 2;
                if self.case == "dialog_navigation" {
                    let url = format!("{}/blank?run={}", self.origin, self.run);
                    if let Some(browser) = &mut self.browser {
                        browser.set("url", &url.to_variant());
                    }
                } else if let Some(browser) = &mut self.browser {
                    browser.set("dialog_policy", &2_i64.to_variant());
                }
            }
            let expected = if self.case == "dialog_navigation" {
                "navigation"
            } else {
                "policy_changed"
            };
            if !self.check_dialog_result(id, expected) {
                return;
            }
            if self.case == "dialog_navigation" {
                if self.dialog_web("navigated", None).is_none() {
                    return;
                }
                self.check(
                    self.dialogs.finished.len() == 1,
                    "Navigation/reset must not finish the same ID twice",
                );
                self.stop();
            } else if self.dialog_web("js_result", Some(0)).is_some() {
                self.next_dialog_step();
            }
            return;
        }
        if self.dialogs.step == 1 {
            self.begin_js_step("confirm");
            let Some(event) = self.dialog_web("js_result", Some(1)) else {
                return;
            };
            self.check(
                event["value"] == false,
                "Cancel policy must return false without opening native UI",
            );
            self.check(
                self.dialogs.requests.len() == 1,
                "Cancel policy must emit no decision request",
            );
            if let Some(browser) = &mut self.browser {
                browser.set("dialog_policy", &1_i64.to_variant());
            }
            self.next_dialog_step();
            return;
        }
        self.begin_js_step("prompt");
        let Some(id) = self.dialog_id() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.answer_js(id, true, "restored");
        }
        let Some(event) = self.dialog_web("js_result", Some(2)) else {
            return;
        };
        if !self.check_dialog_result(id, "accepted") {
            return;
        }
        self.check(
            event["value"] == "restored",
            "Restoring Signal policy must accept a new request",
        );
        self.stop();
    }

    fn tick_dialog_lifecycle(&mut self) {
        if let Some(deadline) = self.dialogs.recreate_at {
            if Instant::now() < deadline {
                return;
            }
            self.dialogs.recreate_at = None;
            if self.class == "CefTexture2D" {
                let id = self.dialogs.old_ids[0];
                self.check(
                    self.dialogs
                        .finished
                        .get(&id)
                        .is_some_and(|result| result == "browser_closed"),
                    "Explicit Resource shutdown must finish its pending dialog as browser_closed",
                );
                self.check_dialog_stale(id);
                let pending = self.call_bool("is_dialog_pending", id);
                self.check(
                    !pending,
                    "Retained shutdown Resource must reject its old pending ID",
                );
                self.destroy_browser();
            }
            if let Some(id) = self.previous_id {
                self.check(
                    Gd::<Object>::try_from_instance_id(id).is_err(),
                    "Destroyed browser must be freed before recreation",
                );
            }
            self.generation += 1;
            self.next_dialog_step();
            if let Err(error) = self.create_browser() {
                self.fail(error);
                return;
            }
            for id in self.dialogs.old_ids.clone() {
                self.check_dialog_stale(id);
            }
            return;
        }
        if self.dialogs.step == 0 {
            self.begin_js_step("prompt");
            let Some(id) = self.dialog_id() else {
                return;
            };
            self.check_dialog_pending(id, false);
            self.dialogs.old_ids.push(id);
            if self.class == "CefTexture2D" {
                self.call("shutdown", &[]);
            } else {
                self.destroy_browser();
            }
            self.dialogs.recreate_at = Some(Instant::now() + Duration::from_millis(500));
            return;
        }
        self.begin_file_step(false);
        let Some(id) = self.dialog_id() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.check_dialog_pending(id, true);
            let value = self.call(
                "continue_file_dialog",
                &[id.to_variant(), self.dialog_paths(false).to_variant()],
            );
            self.check(
                value.try_to::<bool>().is_ok_and(|value| value),
                "Recreated browser must accept a fresh file dialog",
            );
            self.dialogs.answered.insert(id);
        }
        let Some(event) = self.dialog_web("file_result", Some(1)) else {
            return;
        };
        if !self.check_dialog_result(id, "accepted") {
            return;
        }
        self.check_file_contents(&event, false);
        self.check(
            self.stale_signals == 0,
            "Destroyed generation must not leak dialog signals into recreated browser",
        );
        self.stop();
    }

    fn tick_dialog_before_unload(&mut self) {
        if self.dialogs.stage == 0 {
            let loading = self.call("is_loading", &[]).try_to::<bool>();
            if loading.as_ref().is_ok_and(|loading| *loading) {
                return;
            }
            self.check(loading.is_ok(), "Browser loading state must be a bool");
            let Some(ready) = self.dialog_web("dialog_ready", None) else {
                return;
            };
            self.check(
                ready["activate_hit"] == "activate" && ready["leave_hit"] == "leave",
                "First CEF layout must place the before-unload controls under the click coordinates",
            );
            self.dialogs.stage = 1;
            self.dialog_click(Vector2::new(50.0, 100.0));
        }
        if self.dialogs.stage == 1 {
            let Some(event) = self.dialog_web("before_unload_armed", None) else {
                return;
            };
            self.check(
                event["activated"] == true,
                "Before-unload fixture must have real browser user activation",
            );
            self.dialogs.stage = 2;
            self.dialog_click(Vector2::new(50.0, 164.0));
        }
        let Some(request) = self.current_dialog() else {
            return;
        };
        let Some(id) = request["id"].as_i64() else {
            return;
        };
        if !self.dialogs.answered.contains(&id) {
            self.check(
                request["kind"] == "before_unload",
                "Navigation after activation must emit before_unload",
            );
            self.check(
                request["is_reload"] == false,
                "Link navigation must not be marked as reload",
            );
            self.answer_js(id, self.dialogs.step == 1, "");
        }
        if !self.check_dialog_result(
            id,
            if self.dialogs.step == 0 {
                "canceled"
            } else {
                "accepted"
            },
        ) {
            return;
        }
        if self.dialogs.step == 0 {
            if self.dialogs.stage == 2 {
                self.dialogs.stage = 3;
                self.eval("window.report({type:'before_unload_stayed',href:location.href})");
            }
            let Some(event) = self.dialog_web("before_unload_stayed", None) else {
                return;
            };
            self.check(
                event["href"]
                    .as_str()
                    .is_some_and(|url| url.contains("/dialogs?")),
                "Rejecting before-unload must leave the original page active",
            );
            self.check(
                self.dialog_web("navigated", None).is_none(),
                "Rejecting before-unload must prevent navigation",
            );
            self.next_dialog_step();
            self.dialogs.stage = 2;
            self.dialog_click(Vector2::new(50.0, 164.0));
        } else if self.dialog_web("navigated", None).is_some() {
            self.check(
                self.dialogs.requests.len() == 2 && self.dialogs.finished.len() == 2,
                "Canceled and accepted before-unload decisions must each finish once",
            );
            self.stop();
        }
    }

    fn tick_dialog_reentrant(&mut self) {
        self.begin_js_step("prompt");
        let Some(id) = self.dialog_id() else {
            return;
        };
        let Some(event) = self.dialog_web("js_result", Some(0)) else {
            return;
        };
        if !self.check_dialog_result(id, "accepted") {
            return;
        }
        self.check(
            event["value"] == "即时 ✓",
            "Immediate signal callback continuation must reach JavaScript intact",
        );
        self.check(
            self.dialogs.requests.len() == 1 && self.dialogs.finished.len() == 1,
            "Immediate signal callback must complete exactly once",
        );
        self.stop();
    }
}
