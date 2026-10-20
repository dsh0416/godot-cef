use super::*;

impl Runner {
    pub(super) fn permission_requested(&mut self, args: &[Variant]) {
        let Some(kind) = string_arg(args, 0) else {
            self.fail("Invalid permission type argument");
            return;
        };
        let Some(url) = string_arg(args, 1) else {
            self.fail("Invalid permission URL argument");
            return;
        };
        let Some(id) = args.get(2).and_then(|value| value.try_to::<i64>().ok()) else {
            self.fail("Invalid permission ID argument");
            return;
        };
        self.first_request.get_or_insert_with(Instant::now);
        self.check(
            matches!(kind.as_str(), "camera" | "microphone"),
            format!("Unexpected permission: {kind}"),
        );
        self.check(
            url == self.origin || url.starts_with(&format!("{}/", self.origin)),
            "Permission URL must identify fixture origin",
        );
        self.check(id > 0, "Permission IDs must be positive");
        self.check(
            !self.unhandled_finished.contains_key(&id),
            "Retry must receive fresh IDs",
        );
        self.check(
            !self.requests.iter().any(|request| request.id == id),
            "Permission IDs must be distinct",
        );
        self.check(
            !self.requests.iter().any(|request| request.kind == kind),
            "Permission types must be emitted once",
        );
        self.requests.push(PermissionRequest { kind, url, id });
        if self.requests.len() > 2 {
            self.fail("Combined media request emitted more than two IDs");
        }
    }

    pub(super) fn permission_finished(&mut self, args: &[Variant]) {
        let Some(id) = args.first().and_then(|value| value.try_to::<i64>().ok()) else {
            self.fail("Invalid completion ID");
            return;
        };
        let Some(result) = string_arg(args, 1) else {
            self.fail("Invalid completion result");
            return;
        };
        if self.case == "permission_unhandled_then_listen" && !self.retry_started {
            self.check(
                !self.unhandled_finished.contains_key(&id),
                "Unhandled ID must finish exactly once",
            );
            self.check(
                result == "dismissed",
                "Absent listener must dismiss without caching denial",
            );
            self.unhandled_finished.insert(id, result);
        } else {
            self.check(
                !self.finished.contains_key(&id),
                "Each ID must finish exactly once",
            );
            let expected = match self.case.as_str() {
                "permission_grant_all" | "permission_unhandled_then_listen" => "allowed",
                "permission_deny_one" => "denied",
                "permission_timeout" => "timed_out",
                "permission_navigation" => "navigation",
                _ => "unexpected",
            };
            self.check(
                result == expected,
                format!("Expected {expected} completion, received {result}"),
            );
            self.finished.insert(id, result);
            self.last_finished = Some(Instant::now());
        }
    }

    pub(super) fn tick_permissions(&mut self) {
        if self.case == "permission_unhandled_then_listen" && !self.retry_started {
            self.retry_unhandled();
            return;
        }
        if self.requests.len() == 2 && !self.action_started {
            self.action_started = true;
            self.act_on_requests();
        }
        if let Some(deadline) = self.second_grant_at
            && Instant::now() >= deadline
        {
            self.second_grant_at = None;
            self.check(
                self.finished.is_empty(),
                "One grant must not finish the combined request",
            );
            let premature_media = self.web_events.iter().any(|event| {
                event["type"] == "media" && event["attempt"].as_u64().unwrap_or(1) == self.attempt()
            });
            self.check(!premature_media, "JavaScript must wait for both grants");
            if let Some(request) = self.requests.get(1) {
                let id = request.id;
                let granted = self.call_bool("grant_permission", id);
                self.check(granted, "Second grant must be accepted");
            }
        }
        if self.requests.len() != 2 || self.finished.len() != 2 {
            return;
        }
        let terminal = self
            .web_events
            .iter()
            .find(|event| {
                if self.case == "permission_navigation" {
                    event["type"] == "navigated"
                } else {
                    event["type"] == "media"
                        && event["attempt"].as_u64().unwrap_or(1) == self.attempt()
                }
            })
            .cloned();
        let Some(terminal) = terminal else {
            return;
        };
        for request in self.requests.clone() {
            self.check(
                self.finished.contains_key(&request.id),
                "Completion ID must match a request",
            );
            self.check_stale(request.id);
        }
        match self.case.as_str() {
            "permission_grant_all" | "permission_unhandled_then_listen" => {
                self.check(
                    terminal["result"] == "resolved",
                    "Both grants must resolve getUserMedia",
                );
                let tracks = terminal["tracks"].as_array().cloned().unwrap_or_default();
                let mut kinds: Vec<_> = tracks
                    .iter()
                    .filter_map(|track| track["kind"].as_str())
                    .collect();
                kinds.sort_unstable();
                self.check(
                    kinds == ["audio", "video"],
                    "Browser must return both synthetic tracks",
                );
                for track in tracks {
                    let fake = track["label"]
                        .as_str()
                        .is_some_and(|label| label.to_lowercase().contains("fake"));
                    self.check(fake, "Only fake media device labels are permitted");
                }
            }
            "permission_deny_one" => {
                self.check(
                    terminal["result"] == "rejected",
                    "One denial must reject the whole group",
                );
                self.check(
                    terminal["name"] == "NotAllowedError",
                    "Denial must surface NotAllowedError",
                );
            }
            "permission_timeout" => {
                let elapsed = self
                    .first_request
                    .zip(self.last_finished)
                    .map(|(start, end)| end.duration_since(start));
                self.check(
                    elapsed.is_some_and(|elapsed| {
                        elapsed >= Duration::from_millis(700) && elapsed < Duration::from_secs(5)
                    }),
                    format!(
                        "Configured 1s permission timeout outside scheduling tolerance: {elapsed:?}"
                    ),
                );
                self.check(
                    terminal["result"] == "rejected",
                    "Timeout must reject getUserMedia",
                );
            }
            _ => {}
        }
        self.stop();
    }

    fn act_on_requests(&mut self) {
        let ids: Vec<_> = self.requests.iter().map(|request| request.id).collect();
        let [first, second] = ids.as_slice() else {
            self.fail("Expected two permission IDs");
            return;
        };
        let (first, second) = (*first, *second);
        for id in [first, second] {
            let pending = self.call_bool("is_permission_pending", id);
            self.check(pending, "Undecided permission must be pending");
        }
        match self.case.as_str() {
            "permission_grant_all" | "permission_unhandled_then_listen" => {
                let accepted = self.call_bool("grant_permission", first);
                self.check(accepted, "First grant must be accepted");
                let pending = self.call_bool("is_permission_pending", first);
                self.check(!pending, "First grant must consume only that decision");
                let duplicate = self.call_bool("grant_permission", first);
                self.check(!duplicate, "Duplicate grant must return false");
                let pending = self.call_bool("is_permission_pending", second);
                self.check(pending, "Sibling must stay pending until its own decision");
                self.second_grant_at = Some(Instant::now() + Duration::from_millis(100));
            }
            "permission_deny_one" => {
                let accepted = self.call_bool("deny_permission", first);
                self.check(accepted, "Denial must be accepted");
                let pending = self.call_bool("is_permission_pending", second);
                self.check(!pending, "Denial must invalidate the entire group");
                let accepted = self.call_bool("grant_permission", second);
                self.check(!accepted, "Sibling cannot be granted after denial");
            }
            "permission_navigation" => {
                if let Some(browser) = &mut self.browser {
                    browser.set(
                        "url",
                        &format!("{}/blank?run={}", self.origin, self.run).to_variant(),
                    );
                }
            }
            _ => {}
        }
    }

    fn retry_unhandled(&mut self) {
        let terminal = self
            .web_events
            .iter()
            .find(|event| event["type"] == "media" && event["attempt"] == 1)
            .cloned();
        if self.unhandled_finished.len() != 2 {
            return;
        }
        let Some(terminal) = terminal else {
            return;
        };
        self.check(
            terminal["result"] == "rejected",
            "No listener must reject the first request",
        );
        self.check(
            terminal["name"] == "NotAllowedError",
            "Unhandled request must surface NotAllowedError",
        );
        self.check(
            self.requests.is_empty(),
            "Absent listener must receive no request signals",
        );
        for id in self.unhandled_finished.keys().copied().collect::<Vec<_>>() {
            self.check_stale(id);
        }
        self.retry_started = true;
        if let Err(error) = self.connect("permission_requested") {
            self.fail(error);
            return;
        }
        self.eval("window.requestPermissionFixture()");
    }

    fn check_stale(&mut self, id: i64) {
        for method in [
            "is_permission_pending",
            "grant_permission",
            "deny_permission",
        ] {
            let value = self.call_bool(method, id);
            self.check(
                !value,
                format!("Finished ID must return false from {method}"),
            );
        }
    }

    fn attempt(&self) -> u64 {
        if self.case == "permission_unhandled_then_listen" {
            2
        } else {
            1
        }
    }
}
