extends Node

const SIZE := Vector2(640, 360)
const COLOR_POINTS := [Vector2i(500, 60), Vector2i(500, 230), Vector2i(610, 340)]
var browser: Object
var surface: TextureRect
var class_name_under_test := ""
var accelerated := false
var evidence := ""
var checks := 0
var failures: Array[String] = []
var samples: Array[Dictionary] = []
var baseline: Array[Color] = []
var start_ms := 0
var finished := false
var stable_rid: RID


func _ready() -> void:
	Engine.max_fps = 60
	start_ms = Time.get_ticks_msec()
	class_name_under_test = OS.get_environment("GDCEF_RENDER_CLASS")
	accelerated = OS.get_environment("GDCEF_RENDER_MODE") == "accelerated"
	evidence = OS.get_environment("GDCEF_RENDER_OUTPUT")
	ProjectSettings.set_setting("godot_cef/storage/data_path", evidence.path_join("cef-profile"))
	call_deferred("run")


func _process(_delta: float) -> void:
	if not finished and Time.get_ticks_msec() - start_ms > 60000:
		fail("Popup test exceeded its internal deadline")


func run() -> void:
	var underlay := ColorRect.new()
	underlay.color = Color(0.2, 0.3, 0.4, 1.0)
	underlay.size = SIZE
	underlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(underlay)
	browser = ClassDB.instantiate(class_name_under_test)
	browser.set("enable_accelerated_osr", accelerated)
	# The runner redirects localhost to 127.0.0.1, replacing Chromium's widget
	# while this Godot control keeps focus, just like TestUFO's HTTPS redirect.
	browser.set("url", OS.get_environment("GDCEF_RENDER_URL"))
	if class_name_under_test == "CefTexture":
		surface = browser as TextureRect
	else:
		browser.set("texture_size", Vector2i(SIZE))
		surface = TextureRect.new()
		surface.texture = browser as Texture2D
		surface.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
		var material := CanvasItemMaterial.new()
		material.blend_mode = CanvasItemMaterial.BLEND_MODE_PREMULT_ALPHA
		surface.material = material
	surface.focus_mode = Control.FOCUS_ALL
	surface.size = SIZE
	add_child(surface)
	get_window().grab_focus()
	surface.grab_focus()
	if not await expect_page(): return
	stable_rid = surface.texture.get_rid()
	if not await click_at(Vector2(120, 50)): return
	if not await expect_popup(true, "popup-open"): return
	await key(KEY_DOWN)
	await key(KEY_ENTER)
	if not await expect_popup(false, "popup-selected"): return
	if not await expect_selection(): return
	if not await click_at(Vector2(120, 50)): return
	if not await expect_popup(true, "popup-reopen"): return
	if not await click_at(Vector2(610, 340)): return
	if not await expect_popup(false, "popup-hidden"): return
	browser.call("eval", "document.querySelector('#menu').style.width='43.75vw';document.querySelector('#menu').style.left='12.5vw'")
	await get_tree().create_timer(0.2).timeout
	if not await click_at(Vector2(140, 50)): return
	if not await expect_popup(true, "popup-resized-moved"): return
	if surface.texture.get_rid() != stable_rid:
		fail("Opening, hiding or resizing a popup replaced the public texture RID")
		return
	checks += 1
	# Exit with a live popup: cleanup must detach the canvas and native bindings.
	finish()


func frame_image() -> Image:
	await RenderingServer.frame_post_draw
	return get_viewport().get_texture().get_image()


func expect_page() -> bool:
	var deadline := Time.get_ticks_msec() + 15000
	while Time.get_ticks_msec() < deadline and not finished:
		var image := await frame_image()
		var color := image.get_pixelv(COLOR_POINTS[0])
		if color_distance(color, Color(64.0 / 255, 96.0 / 255, 128.0 / 255)) < 0.02:
			for point in COLOR_POINTS: baseline.append(image.get_pixelv(point))
			image.save_png(evidence.path_join("popup-before.png"))
			checks += 1
			return true
	fail("Redirected browser page did not produce the expected midtone patch")
	return false


func expect_popup(visible: bool, label: String) -> bool:
	var deadline := Time.get_ticks_msec() + 5000
	while Time.get_ticks_msec() < deadline and not finished:
		var image := await frame_image()
		var red := 0
		# The page has no red pixels. Its native popup paints red option rows,
		# independent of OS selection-highlight color and menu row metrics.
		for y in range(100, 330, 4):
			for x in range(20, 390, 4):
				var color := image.get_pixel(x, y)
				if color.r > 0.9 and color.g < 0.1 and color.b < 0.1: red += 1
		if (red > 100) == visible:
			var colors: Array = []
			var expected_colors: Array = []
			for point in COLOR_POINTS:
				var color := image.get_pixelv(point)
				colors.append([color.r, color.g, color.b, color.a])
			for color in baseline:
				expected_colors.append([color.r, color.g, color.b, color.a])
			for index in COLOR_POINTS.size():
				if color_distance(image.get_pixelv(COLOR_POINTS[index]), baseline[index]) > 0.02:
					image.save_png(evidence.path_join("failure.png"))
					samples.append({"label": label, "popup_visible": visible, "red_samples": red, "colors": colors, "expected_colors": expected_colors, "colors_preserved": false})
					fail(label + " changed color sample %d: expected %s, got %s" % [index, baseline[index], image.get_pixelv(COLOR_POINTS[index])])
					return false
			image.save_png(evidence.path_join(label + ".png"))
			samples.append({"label": label, "popup_visible": visible, "red_samples": red, "colors": colors, "expected_colors": expected_colors, "colors_preserved": true})
			checks += 1
			return true
	fail(label + ": native popup pixels did not reach the expected visibility")
	return false


func expect_selection() -> bool:
	var deadline := Time.get_ticks_msec() + 5000
	while Time.get_ticks_msec() < deadline and not finished:
		var image := await frame_image()
		var color := image.get_pixel(630, 8)
		if color.r > 0.9 and color.g > 0.9 and color.b < 0.1:
			checks += 1
			return true
	fail("Native popup keyboard selection did not select TWO")
	return false


func click_at(position: Vector2) -> bool:
	if finished: return false
	# Focusing the native window is needed by CEF; do not blur/refocus the
	# control after navigation, because that would hide the focus-cache bug.
	get_window().grab_focus()
	await get_tree().process_frame
	for pressed in [true, false]:
		var event := InputEventMouseButton.new()
		event.position = position
		event.global_position = position
		event.button_index = MOUSE_BUTTON_LEFT
		event.pressed = pressed
		if class_name_under_test == "CefTexture":
			get_viewport().push_input(event)
		else:
			# CefTexture2D's texture_size is in CSS pixels. The node path gets
			# physical viewport pixels and performs its own DPI conversion.
			browser.call("forward_mouse_button_event", event, 1.0, 1.0)
		await get_tree().process_frame
	return true


func key(code: Key) -> void:
	for pressed in [true, false]:
		var event := InputEventKey.new()
		event.keycode = code
		event.physical_keycode = code
		event.pressed = pressed
		if class_name_under_test == "CefTexture":
			get_viewport().push_input(event)
		else:
			browser.call("forward_key_event", event, false)
		await get_tree().process_frame


func color_distance(a: Color, b: Color) -> float:
	return (Vector3(a.r, a.g, a.b) - Vector3(b.r, b.g, b.b)).length()


func fail(message: String) -> void:
	if finished: return
	failures.append(message)
	finish()


func finish() -> void:
	if finished: return
	finished = true
	var result := {
		"class": class_name_under_test, "accelerated_requested": accelerated,
		"driver": RenderingServer.get_current_rendering_driver_name(),
		"thread_model": ProjectSettings.get_setting("rendering/threads/thread_model"),
		"checks": checks, "passed": failures.is_empty(), "failures": failures,
		"samples": samples, "elapsed_ms": Time.get_ticks_msec() - start_ms,
		"scenario": "popup",
	}
	var file := FileAccess.open(evidence.path_join("result.json"), FileAccess.WRITE)
	if file != null: file.store_string(JSON.stringify(result, "  "))
	print("GDCEF_RENDER_RESULT " + JSON.stringify(result))
	get_tree().quit(0 if failures.is_empty() else 1)
