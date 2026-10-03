//! Test-only extension. It drives the separately loaded production extension through ClassDB.
//!
//! No test is started during import or unless the external harness explicitly opts in.
//! Callbacks only enqueue events: assertions and Godot calls run on subsequent main-loop frames,
//! outside production signal dispatch and without blocking CEF's message pump.

mod permissions;
mod scenarios;

use godot::classes::{
    ClassDb, DisplayServer, Engine, HttpRequest, Object, ProjectSettings, RenderingServer,
    SceneTree,
};
use godot::global::Error;
use godot::init::{ExtensionLibrary, InitStage, gdextension};
use godot::obj::InstanceId;
use godot::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(30);
const TEXT: &str = "gdcef-itest:hello:✓";
const BYTES: &[u8] = &[0, 1, 127, 128, 255];

thread_local! {
    static RUNNER: RefCell<Option<Runner>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
    static START_PENDING: RefCell<bool> = const { RefCell::new(false) };
}

struct IntegrationTests;

#[gdextension(entry_symbol = gdcef_itest_init)]
unsafe impl ExtensionLibrary for IntegrationTests {
    fn on_stage_init(stage: InitStage) {
        if stage == InitStage::MainLoop
            && std::env::var("GDCEF_ITEST").as_deref() == Ok("1")
            && !Engine::singleton().is_editor_hint()
        {
            START_PENDING.with(|pending| *pending.borrow_mut() = true);
        }
    }

    fn on_main_loop_frame() {
        let start = START_PENDING.with(|pending| pending.replace(false));
        if start {
            RUNNER.with(|runner| *runner.borrow_mut() = Some(Runner::new()));
        }
        // Taking ownership also prevents a callback from borrowing the live runner recursively.
        let active = RUNNER.with(|runner| runner.borrow_mut().take());
        if let Some(mut runner) = active {
            runner.tick();
            RUNNER.with(|slot| *slot.borrow_mut() = Some(runner));
        }
    }

    fn on_stage_deinit(stage: InitStage) {
        if stage == InitStage::MainLoop {
            RUNNER.with(|runner| {
                if let Some(mut runner) = runner.borrow_mut().take() {
                    runner.destroy_browser();
                    if let Some(mut poller) = runner.poller.take()
                        && poller.is_instance_valid()
                    {
                        poller.cancel_request();
                        poller.queue_free();
                    }
                    runner.disconnect_draw_observer();
                }
            });
            EVENTS.with(|events| events.borrow_mut().clear());
        }
    }
}

struct Event {
    name: &'static str,
    generation: u32,
    args: Vec<Variant>,
}

fn callback(name: &'static str, generation: u32) -> Callable {
    Callable::from_fn(format!("gdcef_itest_{name}"), move |args| {
        EVENTS.with(|events| {
            events.borrow_mut().push(Event {
                name,
                generation,
                args: args.iter().map(|value| (*value).clone()).collect(),
            });
        });
    })
}

#[derive(Clone)]
struct PermissionRequest {
    kind: String,
    url: String,
    id: i64,
}

struct Runner {
    class: String,
    case: String,
    origin: String,
    run: String,
    started: Instant,
    browser: Option<Gd<Object>>,
    poller: Option<Gd<HttpRequest>>,
    draw_observer: Option<Callable>,
    polling: bool,
    next_poll: Instant,
    web_events: Vec<Value>,
    failures: Vec<String>,
    checks: usize,
    stopping: Option<Instant>,
    reported: bool,
    generation: u32,
    previous_id: Option<InstanceId>,
    recreate_at: Option<Instant>,
    shutdown_at: Option<Instant>,
    requests: Vec<PermissionRequest>,
    finished: BTreeMap<i64, String>,
    unhandled_finished: BTreeMap<i64, String>,
    first_request: Option<Instant>,
    last_finished: Option<Instant>,
    action_started: bool,
    second_grant_at: Option<Instant>,
    retry_started: bool,
    ipc_sent: bool,
    ipc_received: BTreeMap<String, usize>,
    eval_sent: bool,
    load_finished: usize,
    stale_signals: usize,
    process_frames: usize,
    draw_frames: usize,
}

impl Runner {
    fn new() -> Self {
        let env = |name| std::env::var(name).unwrap_or_default();
        let now = Instant::now();
        let mut runner = Self {
            class: env("GDCEF_ITEST_CLASS"),
            case: env("GDCEF_ITEST_CASE"),
            origin: env("GDCEF_ITEST_ORIGIN"),
            run: env("GDCEF_ITEST_RUN"),
            started: now,
            browser: None,
            poller: None,
            draw_observer: None,
            polling: false,
            next_poll: now,
            web_events: Vec::new(),
            failures: Vec::new(),
            checks: 0,
            stopping: None,
            reported: false,
            generation: 1,
            previous_id: None,
            recreate_at: None,
            shutdown_at: None,
            requests: Vec::new(),
            finished: BTreeMap::new(),
            unhandled_finished: BTreeMap::new(),
            first_request: None,
            last_finished: None,
            action_started: false,
            second_grant_at: None,
            retry_started: false,
            ipc_sent: false,
            ipc_received: BTreeMap::new(),
            eval_sent: false,
            load_finished: 0,
            stale_signals: 0,
            process_frames: 0,
            draw_frames: 0,
        };
        if let Err(error) = runner.setup(&env("GDCEF_ITEST_PROFILE")) {
            runner.fail(error);
        }
        runner
    }

    fn setup(&mut self, profile: &str) -> Result<(), String> {
        if !matches!(self.class.as_str(), "CefTexture" | "CefTexture2D")
            || !matches!(
                self.case.as_str(),
                "permission_grant_all"
                    | "permission_deny_one"
                    | "permission_timeout"
                    | "permission_navigation"
                    | "permission_unhandled_then_listen"
                    | "js_ipc"
                    | "lifecycle"
            )
        {
            return Err("Harness supplied an unknown class or case".into());
        }
        let port = self
            .origin
            .strip_prefix("http://127.0.0.1:")
            .and_then(|value| value.parse::<u16>().ok());
        if port.is_none_or(|port| port == 0)
            || self.run.is_empty()
            || !self
                .run
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !std::path::Path::new(profile).is_absolute()
        {
            return Err(
                "Harness must supply a loopback origin, alphanumeric run ID and absolute profile"
                    .into(),
            );
        }
        ProjectSettings::singleton()
            .set_setting("godot_cef/storage/data_path", &profile.to_variant());
        self.check(
            DisplayServer::singleton().get_name() == "headless",
            "Integration run must use Godot's headless display server",
        );
        let draw_observer = callback("draw_frame", 0);
        let draw_error = RenderingServer::singleton().connect("frame_pre_draw", &draw_observer);
        if draw_error != Error::OK {
            return Err(format!("Could not observe draw frames: {draw_error:?}"));
        }
        self.draw_observer = Some(draw_observer);
        let mut poller = HttpRequest::new_alloc();
        let error = poller.connect("request_completed", &callback("http", 0));
        if error != Error::OK {
            poller.free();
            return Err(format!("Could not connect HTTPRequest: {error:?}"));
        }
        let Some(mut root) = scene_tree().and_then(|tree| tree.get_root()) else {
            poller.free();
            return Err("No SceneTree root after MainLoop initialization".into());
        };
        root.add_child(&poller);
        self.poller = Some(poller);
        if self.case == "lifecycle" && self.class == "CefTexture2D" {
            self.generation = 0;
        }
        self.create_browser()?;
        if self.generation == 0 {
            self.call("shutdown", &[]);
            self.shutdown_at = Some(Instant::now());
        }
        Ok(())
    }

    fn create_browser(&mut self) -> Result<(), String> {
        let class: StringName = self.class.as_str().into();
        let db = ClassDb::singleton();
        if !db.class_exists(&class) {
            return Err(format!(
                "Production extension did not register {}",
                self.class
            ));
        }
        let mut browser = db
            .instantiate(&class)
            .try_to::<Gd<Object>>()
            .map_err(|error| format!("Could not instantiate {}: {error}", self.class))?;
        // Retain the object before fallible setup so failure cleanup handles partially created browsers.
        self.browser = Some(browser.clone());
        for method in [
            "eval",
            "grant_permission",
            "deny_permission",
            "is_permission_pending",
            "send_ipc_message",
            "send_ipc_binary_message",
            "send_ipc_data",
        ] {
            self.check(
                browser.has_method(method),
                format!("Missing public method: {method}"),
            );
        }
        for signal in ["permission_requested", "permission_request_finished"] {
            if !browser.has_signal(signal) {
                return Err(format!("Missing public signal: {signal}"));
            }
        }
        browser.set("enable_accelerated_osr", &false.to_variant());
        browser.set("permission_policy", &2_i64.to_variant());
        browser.set(
            "preload_script",
            &"window.__gdcef_preload = true;".to_variant(),
        );
        let path = if self.case == "js_ipc" {
            "js"
        } else if self.case == "lifecycle" {
            "lifecycle"
        } else {
            "case"
        };
        let url = format!(
            "{}/{path}?run={}&generation={}",
            self.origin, self.run, self.generation
        );
        browser.set("url", &url.to_variant());
        self.check(
            browser
                .get("permission_policy")
                .try_to::<i64>()
                .is_ok_and(|policy| policy == 2),
            "Per-instance SIGNAL policy must override project DENY_ALL",
        );
        self.connect("permission_request_finished")?;
        if self.case != "permission_unhandled_then_listen" {
            self.connect("permission_requested")?;
        } else {
            self.check(
                browser
                    .get_signal_connection_list("permission_requested")
                    .is_empty(),
                "First permission request must have no listener",
            );
        }
        if self.class == "CefTexture" {
            for signal in [
                "ipc_message",
                "ipc_binary_message",
                "ipc_data_message",
                "load_finished",
                "load_error",
            ] {
                self.connect(signal)?;
            }
            browser.set("size", &Vector2::new(640.0, 360.0).to_variant());
            let node = browser
                .try_cast::<Node>()
                .map_err(|_| "CefTexture is not a Node")?;
            let Some(mut root) = scene_tree().and_then(|tree| tree.get_root()) else {
                return Err("SceneTree root disappeared".into());
            };
            root.add_child(&node);
        } else {
            browser.set("texture_size", &Vector2i::new(640, 360).to_variant());
        }
        for method in [
            "is_permission_pending",
            "grant_permission",
            "deny_permission",
        ] {
            let value = self.call_bool(method, -1);
            self.check(
                !value,
                format!("Unknown request must return false from {method}"),
            );
        }
        Ok(())
    }

    fn connect(&mut self, signal: &'static str) -> Result<(), String> {
        let Some(browser) = &mut self.browser else {
            return Err("Browser is absent".into());
        };
        let error = browser.connect(signal, &callback(signal, self.generation));
        if error == Error::OK {
            Ok(())
        } else {
            Err(format!("Could not connect {signal}: {error:?}"))
        }
    }

    fn tick(&mut self) {
        self.process_frames += 1;
        let events = EVENTS.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        for event in events {
            self.event(event);
        }
        if let Some(stopping) = self.stopping {
            if !self.reported && stopping.elapsed() >= Duration::from_millis(500) {
                self.report();
            }
            return;
        }
        if self.started.elapsed() >= DEADLINE {
            self.fail("Timed out waiting for real CEF callbacks and page reports");
            return;
        }
        if !self.polling && Instant::now() >= self.next_poll {
            self.next_poll = Instant::now() + Duration::from_millis(50);
            if let Some(poller) = &mut self.poller {
                let error = poller.request(&format!("{}/state?run={}", self.origin, self.run));
                if error != Error::OK {
                    self.fail(format!("Loopback HTTP request failed: {error:?}"));
                    return;
                }
                self.polling = true;
            }
        }
        if let Some(event) = self.web_events.iter().find(|event| {
            matches!(
                event["type"].as_str(),
                Some("unsupported" | "fixture_error")
            )
        }) {
            self.fail(format!("Browser fixture failed: {event}"));
            return;
        }
        if self.case.starts_with("permission_") {
            self.tick_permissions();
        } else if self.case == "js_ipc" {
            self.tick_ipc();
        } else {
            self.tick_lifecycle();
        }
        if !self.failures.is_empty() && self.stopping.is_none() {
            self.stop();
        }
    }

    fn event(&mut self, event: Event) {
        if event.name == "draw_frame" {
            self.draw_frames += 1;
            return;
        }
        if event.name == "http" {
            self.polling = false;
            if self.stopping.is_some() {
                return;
            }
            let result = event
                .args
                .first()
                .and_then(|value| value.try_to::<i64>().ok());
            let status = event
                .args
                .get(1)
                .and_then(|value| value.try_to::<i64>().ok());
            let body = event
                .args
                .get(3)
                .and_then(|value| value.try_to::<PackedByteArray>().ok());
            if result != Some(0) || status != Some(200) {
                self.fail(format!("Loopback response failed: {result:?}/{status:?}"));
                return;
            }
            let parsed = body.and_then(|body| serde_json::from_slice::<Value>(&body.to_vec()).ok());
            if let Some(events) = parsed.and_then(|value| value["events"].as_array().cloned()) {
                self.web_events = events;
            } else {
                self.fail("Invalid loopback status JSON");
            }
            return;
        }
        if event.generation != self.generation || self.browser.is_none() {
            self.stale_signals += 1;
            self.check(
                false,
                format!(
                    "Stale {} callback from destroyed generation {}",
                    event.name, event.generation
                ),
            );
            return;
        }
        match event.name {
            "permission_requested" => self.permission_requested(&event.args),
            "permission_request_finished" => self.permission_finished(&event.args),
            "load_finished" => {
                self.load_finished += 1;
            }
            "load_error" => self.fail(format!("Browser load failed: {:?}", event.args)),
            "ipc_message" | "ipc_binary_message" | "ipc_data_message" => {
                self.ipc_event(event.name, &event.args)
            }
            _ => self.fail(format!("Unexpected callback: {}", event.name)),
        }
    }

    fn call(&mut self, method: &str, args: &[Variant]) -> Variant {
        self.browser
            .as_mut()
            .map_or_else(Variant::nil, |browser| browser.call(method, args))
    }

    fn call_bool(&mut self, method: &str, id: i64) -> bool {
        let result = self.call(method, &[id.to_variant()]);
        match result.try_to::<bool>() {
            Ok(value) => value,
            Err(error) => {
                self.check(false, format!("{method} did not return bool: {error}"));
                false
            }
        }
    }

    fn eval(&mut self, code: &str) {
        self.call("eval", &[code.to_variant()]);
    }

    fn check(&mut self, condition: bool, message: impl Into<String>) {
        self.checks += 1;
        if !condition {
            self.failures.push(message.into());
        }
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.check(false, message);
        self.stop();
    }

    fn stop(&mut self) {
        if self.stopping.is_none() {
            self.stopping = Some(Instant::now());
            self.destroy_browser();
            if let Some(mut poller) = self.poller.take() {
                poller.cancel_request();
                poller.queue_free();
            }
        }
    }

    fn destroy_browser(&mut self) {
        self.release_browser(true);
    }

    fn release_browser(&mut self, explicit_shutdown: bool) {
        if let Some(mut browser) = self.browser.take()
            && browser.is_instance_valid()
        {
            self.previous_id = Some(browser.instance_id());
            if self.class == "CefTexture2D" {
                if explicit_shutdown {
                    browser.call("shutdown", &[]);
                }
            } else if let Ok(mut node) = browser.try_cast::<Node>() {
                node.queue_free();
            }
        }
    }

    fn disconnect_draw_observer(&mut self) {
        if let Some(callable) = self.draw_observer.take() {
            let mut rendering = RenderingServer::singleton();
            if rendering.is_connected("frame_pre_draw", &callable) {
                rendering.disconnect("frame_pre_draw", &callable);
            }
        }
    }

    fn report(&mut self) {
        self.reported = true;
        self.disconnect_draw_observer();
        self.check(
            self.draw_frames == 0,
            "Headless tests must complete without rendering frames",
        );
        if let Some(id) = self.previous_id {
            self.check(
                Gd::<Object>::try_from_instance_id(id).is_err(),
                "Final browser object must be freed",
            );
        }
        let requests: Vec<_> = self
            .requests
            .iter()
            .map(|request| json!({"type": request.kind, "url": request.url, "id": request.id}))
            .collect();
        let elapsed = self
            .first_request
            .zip(self.last_finished)
            .map(|(start, end)| end.duration_since(start).as_millis());
        godot_print!(
            "GDCEF_ITEST_RESULT {}",
            json!({
                "class": self.class, "case": self.case, "passed": self.failures.is_empty(),
                "checks": self.checks, "failures": self.failures, "requests": requests,
                "finished": self.finished, "unhandled_finished": self.unhandled_finished,
                "events": self.web_events, "request_to_finish_ms": elapsed,
                "generation": self.generation, "ipc_received": self.ipc_received,
                "load_finished": self.load_finished, "stale_signals": self.stale_signals,
                "process_frames": self.process_frames, "draw_frames": self.draw_frames,
                "elapsed_ms": self.started.elapsed().as_millis(),
            })
        );
        if let Some(mut tree) = scene_tree() {
            tree.quit_ex()
                .exit_code(i32::from(!self.failures.is_empty()))
                .done();
        }
    }
}

fn scene_tree() -> Option<Gd<SceneTree>> {
    Engine::singleton()
        .get_main_loop()?
        .try_cast::<SceneTree>()
        .ok()
}

fn string_arg(args: &[Variant], index: usize) -> Option<String> {
    args.get(index)?
        .try_to::<GString>()
        .ok()
        .map(|value| value.to_string())
}
