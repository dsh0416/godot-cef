use super::*;

fn data_payload() -> Variant {
    let mut data = VarDictionary::new();
    data.set("text", "typed:✓");
    data.set("number", 42_i64);
    data.set("flag", true);
    let mut items = VarArray::new();
    items.push(&1_i64.to_variant());
    items.push(&"two".to_variant());
    items.push(&Variant::nil());
    data.set("items", &items);
    data.to_variant()
}

impl Runner {
    pub(super) fn ipc_event(&mut self, name: &str, args: &[Variant]) {
        if self.case != "js_ipc" {
            self.fail(format!("Unexpected IPC outside js_ipc: {name}"));
            return;
        }
        let Some(value) = args.first() else {
            self.fail("IPC callback omitted its value");
            return;
        };
        let (lane, matched) = match name {
            "ipc_message" => (
                "text",
                value.try_to::<GString>().is_ok_and(|text| text == TEXT),
            ),
            "ipc_binary_message" => (
                "binary",
                value
                    .try_to::<PackedByteArray>()
                    .is_ok_and(|bytes| bytes.as_slice() == BYTES),
            ),
            "ipc_data_message" => ("data", *value == data_payload()),
            _ => {
                self.fail("Unknown IPC callback");
                return;
            }
        };
        self.check(
            matched,
            format!("{lane} IPC must preserve payload in the browser-to-Godot round trip"),
        );
        let count = self.ipc_received.entry(lane.into()).or_default();
        *count += 1;
        let count = *count;
        self.check(
            count == 1,
            format!("{lane} IPC callback must arrive exactly once"),
        );
    }

    pub(super) fn tick_ipc(&mut self) {
        if !self.ipc_sent {
            let ready = self
                .web_events
                .iter()
                .find(|event| event["type"] == "js_ready")
                .cloned();
            let Some(ready) = ready else {
                return;
            };
            self.check(
                ready["preload"] == true,
                "Preload must execute before the page's initial script",
            );
            self.ipc_sent = true;
            self.call("send_ipc_message", &[TEXT.to_variant()]);
            self.call(
                "send_ipc_binary_message",
                &[PackedByteArray::from(BYTES).to_variant()],
            );
            self.call("send_ipc_data", &[data_payload()]);
            self.eval("window.report({type:'js_eval',value:6*7})");
        }
        let receipts: Vec<_> = self
            .web_events
            .iter()
            .filter(|event| event["type"] == "ipc_received")
            .cloned()
            .collect();
        let evaluated = self
            .web_events
            .iter()
            .find(|event| event["type"] == "js_eval")
            .cloned();
        if receipts.len() < 3 || evaluated.is_none() {
            return;
        }
        if self.class == "CefTexture" && self.ipc_received.len() < 3 {
            return;
        }
        self.check(
            receipts.len() == 3,
            "Page must receive exactly three IPC messages",
        );
        for (lane, expected) in [
            ("text", json!(TEXT)),
            ("binary", json!(BYTES)),
            (
                "data",
                json!({"text":"typed:✓", "number":42, "flag":true, "items":[1,"two",null]}),
            ),
        ] {
            let values: Vec<_> = receipts
                .iter()
                .filter(|event| event["lane"] == lane)
                .collect();
            self.check(
                values.len() == 1 && values[0]["value"] == expected,
                format!("Godot-to-browser {lane} IPC must preserve the exact payload"),
            );
        }
        self.check(
            evaluated.is_some_and(|event| event["value"] == 42),
            "eval must execute JavaScript and produce its HTTP report",
        );
        if self.class == "CefTexture" {
            self.check(
                self.load_finished > 0,
                "CefTexture must deliver real load_finished callbacks",
            );
        }
        self.stop();
    }

    pub(super) fn tick_lifecycle(&mut self) {
        if let Some(shutdown_at) = self.shutdown_at {
            if shutdown_at.elapsed() < Duration::from_millis(500) {
                return;
            }
            self.shutdown_at = None;
            let ready_count = self
                .web_events
                .iter()
                .filter(|event| {
                    event["type"] == "lifecycle_ready" && event["generation"] == self.generation
                })
                .count();
            self.check(ready_count == usize::from(self.generation != 0),
                "A retained shutdown resource must not create or restart a browser on later process frames");
            self.check(
                self.browser.as_ref().is_some_and(Gd::is_instance_valid),
                "Resource must remain a valid Godot object after explicit shutdown",
            );
            // Calling shutdown again while dropping also verifies its public idempotent cleanup path.
            if self.generation == 3 {
                self.stop();
            } else {
                self.destroy_browser();
                self.recreate_at = Some(Instant::now() + Duration::from_millis(250));
            }
            return;
        }
        if let Some(deadline) = self.recreate_at {
            if Instant::now() < deadline {
                return;
            }
            self.recreate_at = None;
            if let Some(id) = self.previous_id {
                self.check(
                    Gd::<Object>::try_from_instance_id(id).is_err(),
                    "Destroyed browser object must not remain alive",
                );
            }
            self.generation += 1;
            self.eval_sent = false;
            if let Err(error) = self.create_browser() {
                self.fail(error);
            }
            return;
        }
        let ready = self
            .web_events
            .iter()
            .find(|event| {
                event["type"] == "lifecycle_ready" && event["generation"] == self.generation
            })
            .cloned();
        let Some(ready) = ready else {
            return;
        };
        if !self.eval_sent {
            self.check(
                ready["preload"] == true,
                "Every recreated browser must run preload before page script",
            );
            if let Some(id) = self.previous_id {
                self.check(
                    self.browser
                        .as_ref()
                        .is_some_and(|browser| browser.instance_id() != id),
                    "Recreation must create a new Godot object",
                );
            }
            if let Some(browser) = &mut self.browser {
                if self.class == "CefTexture2D" {
                    browser.set("texture_size", &Vector2i::new(320, 180).to_variant());
                    let size = browser.get("texture_size").try_to::<Vector2i>();
                    self.check(
                        size.is_ok_and(|size| size == Vector2i::new(320, 180)),
                        "Resource resize must update public dimensions",
                    );
                } else {
                    browser.set("size", &Vector2::new(320.0, 180.0).to_variant());
                    let size = browser.get("size").try_to::<Vector2>();
                    self.check(
                        size.is_ok_and(|size| size == Vector2::new(320.0, 180.0)),
                        "Node resize must update public dimensions",
                    );
                }
            }
            self.eval_sent = true;
            // Wait inside the real browser for CEF to apply WasResized; a Rust property
            // round trip alone would miss a broken resize notification or view rectangle.
            self.eval(&format!(
                "(function waitResize(left){{if((innerWidth===320&&innerHeight===180)||left===0){{window.report({{type:'lifecycle_eval',generation:{},width:innerWidth,height:innerHeight}})}}else{{setTimeout(()=>waitResize(left-1),25)}}}})(80)",
                self.generation
            ));
            return;
        }
        let evaluated = self
            .web_events
            .iter()
            .find(|event| {
                event["type"] == "lifecycle_eval" && event["generation"] == self.generation
            })
            .cloned();
        let Some(evaluated) = evaluated else {
            return;
        };
        self.check(
            evaluated["width"] == 320 && evaluated["height"] == 180,
            "CEF's actual JavaScript viewport must resize to 320x180",
        );
        let ready_count = self
            .web_events
            .iter()
            .filter(|event| {
                event["type"] == "lifecycle_ready" && event["generation"] == self.generation
            })
            .count();
        self.check(
            ready_count == 1,
            "Each generation must create exactly one live browser page",
        );
        if self.class == "CefTexture2D" && self.generation == 2 {
            // Exercise implicit Resource destruction independently of shutdown(). A
            // signal hookup that accidentally retained the resource fails the ID check.
            self.release_browser(false);
            self.recreate_at = Some(Instant::now() + Duration::from_millis(250));
        } else if self.class == "CefTexture2D" {
            self.call("shutdown", &[]);
            self.shutdown_at = Some(Instant::now());
        } else if self.generation == 3 {
            if self.class == "CefTexture" {
                self.check(
                    self.load_finished == 3,
                    "Each recreated node must emit one load_finished callback",
                );
            }
            self.stop();
        } else {
            self.destroy_browser();
            self.recreate_at = Some(Instant::now() + Duration::from_millis(250));
        }
    }
}
