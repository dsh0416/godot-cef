extends Node

const EXPECTED_RESULTS := {
	"grant_all": "allowed",
	"deny_one": "denied",
	"timeout": "timed_out",
	"navigation": "navigation",
	"unhandled_then_listen": "allowed",
}

var browser: Object
var view: TextureRect
var poller: HTTPRequest
var browser_class := OS.get_environment("PERMISSION_TEST_CLASS")
var case_name := OS.get_environment("PERMISSION_TEST_CASE")
var origin := OS.get_environment("PERMISSION_TEST_ORIGIN")
var run_id := OS.get_environment("PERMISSION_TEST_RUN")
var requests: Array[Dictionary] = []
var finished: Dictionary = {}
var web_events: Array = []
var failures: Array[String] = []
var first_request_ms := 0
var started_ms := 0
var finished_ms := 0
var next_poll_ms := 0
var polling := false
var action_scheduled := false
var completing := false
var checks := 0
var unhandled_finished: Dictionary = {}
var retry_started := false


func _ready() -> void:
	started_ms = Time.get_ticks_msec()
	if not EXPECTED_RESULTS.has(case_name) or browser_class not in ["CefTexture", "CefTexture2D"]:
		_fail("Runner must supply a known case and browser class")
		return
	if not origin.begins_with("http://127.0.0.1:"):
		_fail("Fixture origin must be loopback HTTP")
		return
	ProjectSettings.set_setting("godot_cef/storage/data_path", OS.get_environment("PERMISSION_TEST_PROFILE"))
	if not ClassDB.class_exists(browser_class):
		_fail("Extension class is not registered: " + browser_class)
		return
	browser = ClassDB.instantiate(browser_class)
	for method in ["grant_permission", "deny_permission", "is_permission_pending"]:
		if not browser.has_method(method):
			_fail("Missing public method: " + method)
			return
	for signal_name in ["permission_requested", "permission_request_finished"]:
		if not browser.has_signal(signal_name):
			_fail("Missing public signal: " + signal_name)
			return
	browser.set("permission_policy", 2)
	browser.set("enable_accelerated_osr", false)
	browser.set("url", origin + "/case?run=" + run_id)
	_check(browser.get("permission_policy") == 2, "Per-instance SIGNAL policy must override project DENY_ALL")
	_check(not browser.call("is_permission_pending", -1), "Unknown request must not be pending")
	_check(not browser.call("grant_permission", -1), "Unknown grant must return false")
	_check(not browser.call("deny_permission", -1), "Unknown denial must return false")
	if case_name != "unhandled_then_listen":
		browser.connect("permission_requested", _on_permission_requested)
	else:
		_check(browser.get_signal_connection_list("permission_requested").is_empty(), "First request must have no listener")
	browser.connect("permission_request_finished", _on_permission_finished)
	if browser_class == "CefTexture":
		browser.set("size", Vector2(640, 360))
		add_child(browser as Node)
	else:
		browser.set("texture_size", Vector2i(640, 360))
		view = TextureRect.new()
		view.size = Vector2(640, 360)
		view.texture = browser as Texture2D
		add_child(view)
	poller = HTTPRequest.new()
	add_child(poller)
	poller.request_completed.connect(_on_poll_completed)


func _on_permission_requested(permission_type: String, url: String, request_id: int) -> void:
	if first_request_ms == 0:
		first_request_ms = Time.get_ticks_msec()
	_check(permission_type in ["camera", "microphone"], "Unexpected permission type: " + permission_type)
	_check(url == origin or url.begins_with(origin + "/"), "Request URL must identify the loopback origin")
	_check(request_id > 0, "Request ids must be positive")
	_check(not unhandled_finished.has(request_id), "Retried request must receive a fresh id")
	for request in requests:
		_check(request.id != request_id, "Camera and microphone need distinct request ids")
		_check(request.type != permission_type, "Each media type must be emitted once")
	requests.append({"type": permission_type, "url": url, "id": request_id})
	if requests.size() == 2 and not action_scheduled:
		action_scheduled = true
		_act_on_requests.call_deferred()
	elif requests.size() > 2:
		_fail("One combined media request emitted more than two permission signals")


func _act_on_requests() -> void:
	for request in requests:
		_check(browser.call("is_permission_pending", request.id), "Undecided media request must be pending")
	match case_name:
		"grant_all", "unhandled_then_listen":
			var first_id: int = requests[0].id
			var second_id: int = requests[1].id
			_check(browser.call("grant_permission", first_id), "First grant must be accepted")
			_check(not browser.call("is_permission_pending", first_id), "Decided id must not remain pending")
			_check(not browser.call("grant_permission", first_id), "Duplicate grant must return false")
			_check(browser.call("is_permission_pending", second_id), "Sibling stays pending until its decision")
			await get_tree().create_timer(0.1).timeout
			_check(finished.is_empty(), "One grant must not finish a combined request")
			for event in web_events:
				if event.get("attempt", 1) == _expected_attempt():
					_check(event.get("type") != "media", "JavaScript must wait for both grants")
			_check(browser.call("grant_permission", second_id), "Second grant must be accepted")
		"deny_one":
			var denied_id: int = requests[0].id
			var sibling_id: int = requests[1].id
			_check(browser.call("deny_permission", denied_id), "Denial must be accepted")
			_check(not browser.call("is_permission_pending", sibling_id), "Denial invalidates the entire group")
			_check(not browser.call("grant_permission", sibling_id), "Sibling cannot be granted after denial")
		"navigation":
			browser.set("url", origin + "/blank?run=" + run_id)
		"timeout":
			pass


func _on_permission_finished(request_id: int, result: String) -> void:
	if case_name == "unhandled_then_listen" and not retry_started:
		_check(not unhandled_finished.has(request_id), "Unhandled id must finish exactly once")
		_check(result == "dismissed", "An absent listener must dismiss without permanent denial")
		unhandled_finished[request_id] = result
		return
	_check(not finished.has(request_id), "Each id must finish exactly once")
	_check(result == EXPECTED_RESULTS[case_name], "Unexpected finish result: " + result)
	finished[request_id] = result
	finished_ms = Time.get_ticks_msec()


func _process(_delta: float) -> void:
	if completing:
		return
	var now := Time.get_ticks_msec()
	if now - started_ms > 20000:
		_fail("Timed out waiting for CEF signals/browser report; requests=%s events=%s" % [requests, web_events])
		return
	if not is_instance_valid(poller):
		return
	if not polling and now >= next_poll_ms:
		polling = true
		next_poll_ms = now + 100
		var error := poller.request(origin + "/state?run=" + run_id)
		if error != OK:
			_fail("Loopback status request failed: " + str(error))
			return
	for event in web_events:
		if event.get("type") in ["unsupported", "fixture_error"]:
			_fail("CEF cannot run this media fixture: " + JSON.stringify(event))
			return
	if case_name == "unhandled_then_listen" and not retry_started:
		_try_after_unhandled()
		return
	if requests.size() != 2 or finished.size() != 2:
		return
	var terminal_event := _browser_terminal_event()
	if terminal_event.is_empty():
		return
	for request in requests:
		_check(finished.has(request.id), "Finish id must match a permission request")
		_check(not browser.call("is_permission_pending", request.id), "Finished id must not remain pending")
		_check(not browser.call("grant_permission", request.id), "Finished/stale grant must return false")
		_check(not browser.call("deny_permission", request.id), "Finished/stale denial must return false")
	if case_name == "timeout":
		var elapsed := finished_ms - first_request_ms
		_check(elapsed >= 700 and elapsed < 5000, "Configured 1s timeout outside scheduling tolerance: %dms" % elapsed)
	elif case_name in ["grant_all", "unhandled_then_listen"]:
		_check(terminal_event.get("result") == "resolved", "Both grants must resolve getUserMedia")
		var kinds: Array = []
		for track in terminal_event.get("tracks", []):
			kinds.append(track.get("kind"))
			_check("fake" in str(track.get("label", "")).to_lower(), "Only synthetic device labels are permitted")
		kinds.sort()
		_check(kinds == ["audio", "video"], "Both synthetic tracks must be returned")
	elif case_name == "deny_one":
		_check(terminal_event.get("result") == "rejected", "One denial must reject the whole media request")
		_check(terminal_event.get("name") == "NotAllowedError", "Denial should surface NotAllowedError")
	if case_name == "timeout":
		_check(terminal_event.get("result") == "rejected", "Timeout must reject getUserMedia")
	_complete()


func _browser_terminal_event() -> Dictionary:
	for event in web_events:
		if case_name == "navigation" and event.get("type") == "navigated":
			return event
		if case_name != "navigation" and event.get("type") == "media" and event.get("attempt", 1) == _expected_attempt():
			return event
	return {}


func _expected_attempt() -> int:
	return 2 if case_name == "unhandled_then_listen" else 1


func _try_after_unhandled() -> void:
	for event in web_events:
		if event.get("type") != "media" or event.get("attempt") != 1:
			continue
		if unhandled_finished.size() != 2:
			return
		_check(event.get("result") == "rejected", "No listener must reject the first media request")
		_check(event.get("name") == "NotAllowedError", "Unhandled media request should surface NotAllowedError")
		_check(requests.is_empty(), "Unhandled request must not deliver a permission request to the application")
		for old_id in unhandled_finished:
			_check(not browser.call("is_permission_pending", old_id), "Unhandled id must not remain pending")
			_check(not browser.call("grant_permission", old_id), "Unhandled id cannot be granted later")
			_check(not browser.call("deny_permission", old_id), "Unhandled id cannot be denied later")
		retry_started = true
		browser.connect("permission_requested", _on_permission_requested)
		browser.call("eval", "window.requestPermissionFixture()")
		return


func _on_poll_completed(result: int, response_code: int, _headers: PackedStringArray, body: PackedByteArray) -> void:
	polling = false
	if completing:
		return
	if result != HTTPRequest.RESULT_SUCCESS or response_code != 200:
		_fail("Loopback status response failed: %s/%s" % [result, response_code])
		return
	var parsed = JSON.parse_string(body.get_string_from_utf8())
	if parsed is Dictionary and parsed.get("events") is Array:
		web_events = parsed.events
	else:
		_fail("Invalid loopback status JSON")


func _check(condition: bool, message: String) -> void:
	checks += 1
	if not condition:
		failures.append(message)
		push_error(message)


func _fail(message: String) -> void:
	_check(false, message)
	_complete()


func _complete() -> void:
	if completing:
		return
	completing = true
	print("PERMISSION_RESULT " + JSON.stringify({
		"class": browser_class, "case": case_name, "passed": failures.is_empty(),
		"checks": checks, "failures": failures, "requests": requests,
		"finished": finished, "events": web_events,
		"unhandled_finished": unhandled_finished,
		"request_to_finish_ms": finished_ms - first_request_ms,
	}))
	if is_instance_valid(browser):
		if browser_class == "CefTexture2D":
			browser.call("shutdown")
			if is_instance_valid(view):
				view.texture = null
			browser = null
		elif browser is Node:
			(browser as Node).queue_free()
	await get_tree().create_timer(0.2).timeout
	get_tree().quit(0 if failures.is_empty() else 1)
