extends Node

const ORIGIN := Vector2(64, 64)
const PEER_ORIGIN := Vector2(384, 64)
const COLORS := [Color.RED, Color.GREEN, Color.BLUE, Color.YELLOW]
const DEADLINE_MS := 90000
var browser: Object
var surface: TextureRect
var peer_browser: Object
var peer_surface: TextureRect
var peer_last_sequence := 0
var class_name_under_test := ""
var accelerated := false
var evidence := ""
var checks := 0
var failures: Array[String] = []
var samples: Array[Dictionary] = []
var page_dimensions: Array[Dictionary] = []
var last_sequence := 0
var start_ms := 0
var finished := false


func _ready() -> void:
	process_mode = Node.PROCESS_MODE_ALWAYS
	Engine.max_fps = 60
	start_ms = Time.get_ticks_msec()
	class_name_under_test = OS.get_environment("GDCEF_RENDER_CLASS")
	accelerated = OS.get_environment("GDCEF_RENDER_MODE") == "accelerated"
	evidence = OS.get_environment("GDCEF_RENDER_OUTPUT")
	ProjectSettings.set_setting("godot_cef/storage/data_path", evidence.path_join("cef-profile"))
	if class_name_under_test not in ["CefTexture", "CefTexture2D"] or evidence.is_empty():
		fail("Runner configuration is missing or invalid")
		return
	if not ClassDB.class_exists(class_name_under_test):
		fail("Production extension did not register " + class_name_under_test)
		return
	call_deferred("run")


func _process(_delta: float) -> void:
	if not finished and Time.get_ticks_msec() - start_ms > DEADLINE_MS:
		fail("Graphical test exceeded its internal deadline")


func create_browser_surface(origin: Vector2) -> Dictionary:
	var instance: Object = ClassDB.instantiate(class_name_under_test)
	instance.set("enable_accelerated_osr", accelerated)
	instance.set("url", "res://fixture/page.html")
	if instance.has_signal("console_message"):
		instance.connect("console_message", _on_browser_console_message)
	var rect: TextureRect
	if class_name_under_test == "CefTexture":
		rect = instance as TextureRect
		rect.process_mode = Node.PROCESS_MODE_PAUSABLE
	else:
		instance.set("texture_size", Vector2i(256, 192))
		rect = TextureRect.new()
		rect.texture = instance as Texture2D
		rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	rect.position = origin
	rect.size = Vector2(256, 192)
	add_child(rect)
	return {"browser": instance, "surface": rect}


func create_browser() -> void:
	last_sequence = 0
	var created := create_browser_surface(ORIGIN)
	browser = created.browser
	surface = created.surface


func create_peer_browser() -> void:
	peer_last_sequence = 0
	var created := create_browser_surface(PEER_ORIGIN)
	peer_browser = created.browser
	peer_surface = created.surface


func resize_browser(size: Vector2i) -> bool:
	surface.size = Vector2(size)
	if class_name_under_test == "CefTexture2D":
		browser.set("texture_size", size)
	if surface.size != Vector2(size):
		fail("Requested surface size %s was constrained to %s" % [size, surface.size])
		return false
	return true


func _on_browser_console_message(_level: int, message: String, _source: String, _line: int) -> void:
	const PREFIX := "GDCEF_PAGE_DIMENSIONS "
	if message.begins_with(PREFIX):
		var dimensions = JSON.parse_string(message.substr(PREFIX.length()))
		if dimensions is Dictionary:
			page_dimensions.append(dimensions)
		print(message)


func read_sequence(image: Image, target: TextureRect = null) -> int:
	if image == null or image.is_empty():
		return -1
	if target == null:
		target = surface
	var size := target.size
	var origin := target.position
	var sequence := 0
	for bit in range(8):
		var point := origin + Vector2((float(bit) + 0.5) * size.x / 8.0, size.y * 0.75)
		var color := image.get_pixelv(Vector2i(point))
		if color.r > 0.8 and color.g > 0.8 and color.b > 0.8:
			sequence |= 1 << bit
		elif color.r > 0.2 or color.g > 0.2 or color.b > 0.2:
			return -1
	var sentinel := image.get_pixelv(Vector2i(origin + size * Vector2(0.5, 0.25)))
	var expected: Color = COLORS[sequence % COLORS.size()]
	if absf(sentinel.r - expected.r) > 0.2 or absf(sentinel.g - expected.g) > 0.2 or absf(sentinel.b - expected.b) > 0.2:
		return -1
	return sequence


func frame_image() -> Image:
	await RenderingServer.frame_post_draw
	return get_viewport().get_texture().get_image()


func expect_sequence(sequence: int, label: String, allow_resize: bool = false) -> bool:
	var deadline := Time.get_ticks_msec() + 12000
	var matched := 0
	while not finished and Time.get_ticks_msec() < deadline:
		var image := await frame_image()
		var observed := read_sequence(image)
		if observed >= 0 and observed < last_sequence and (not allow_resize or last_sequence == sequence):
			image.save_png(evidence.path_join("failure.png"))
			fail("%s: sequence regressed from %d to %d" % [label, last_sequence, observed])
			return false
		if observed == sequence:
			last_sequence = sequence
			matched += 1
			if matched == 4:
				checks += 1
				samples.append({"label": label, "sequence": sequence, "elapsed_ms": Time.get_ticks_msec() - start_ms})
				image.save_png(evidence.path_join("latest.png"))
				return true
		else:
			matched = 0
	if not finished:
		var image := await frame_image()
		image.save_png(evidence.path_join("failure.png"))
		fail("%s: did not observe stable sequence %d (last decoded %d)" % [label, sequence, read_sequence(image)])
	return false


func expect_frozen(sequence: int, label: String) -> bool:
	for frame in range(16):
		var image := await frame_image()
		if read_sequence(image) != sequence:
			image.save_png(evidence.path_join("failure.png"))
			fail(label + ": displayed pixels changed while processing was suspended")
			return false
	checks += 1
	return true


func expect_pair(sequence: int, peer_sequence: int, label: String, primary_frozen: bool = false) -> bool:
	var deadline := Time.get_ticks_msec() + 12000
	var matched := 0
	while not finished and Time.get_ticks_msec() < deadline:
		var image := await frame_image()
		var observed := read_sequence(image)
		var peer_observed := read_sequence(image, peer_surface)
		if (primary_frozen and observed != sequence) or (observed >= 0 and observed < last_sequence) or (peer_observed >= 0 and peer_observed < peer_last_sequence):
			image.save_png(evidence.path_join("failure.png"))
			fail("%s: frozen/monotonic streams violated (%d,%d), prior (%d,%d)" % [label, observed, peer_observed, last_sequence, peer_last_sequence])
			return false
		if observed >= 0:
			last_sequence = observed
		if peer_observed >= 0:
			peer_last_sequence = peer_observed
		if observed == sequence and peer_observed == peer_sequence:
			matched += 1
			if matched == 4:
				checks += 1
				samples.append({"label": label, "sequence": sequence, "peer_sequence": peer_sequence, "elapsed_ms": Time.get_ticks_msec() - start_ms})
				image.save_png(evidence.path_join("latest.png"))
				return true
		else:
			matched = 0
	if not finished:
		var image := await frame_image()
		image.save_png(evidence.path_join("failure.png"))
		fail("%s: expected streams (%d,%d), last observed (%d,%d)" % [label, sequence, peer_sequence, read_sequence(image), read_sequence(image, peer_surface)])
	return false


func paint(sequence: int) -> void:
	browser.call("eval", "window.paint(%d)" % sequence)


func destroy_browser() -> void:
	var id := browser.get_instance_id()
	if class_name_under_test == "CefTexture2D":
		browser.call("shutdown")
		surface.texture = null
	surface.queue_free()
	browser = null
	surface = null
	for frame in range(12):
		await get_tree().process_frame
	if is_instance_id_valid(id):
		fail("Browser instance survived teardown")
	else:
		checks += 1


func destroy_peer_browser() -> void:
	var id := peer_browser.get_instance_id()
	if class_name_under_test == "CefTexture2D":
		peer_browser.call("shutdown")
		peer_surface.texture = null
	peer_surface.queue_free()
	peer_browser = null
	peer_surface = null
	for frame in range(12):
		await get_tree().process_frame
	if is_instance_id_valid(id):
		fail("Peer browser instance survived teardown")
	else:
		checks += 1


func stress_independent_streams(current_sequence: int, generation: int) -> bool:
	create_peer_browser()
	if not await expect_pair(current_sequence, 1, "peer-initial-%d" % generation):
		return false
	peer_browser.call("eval", "window.paint(128)")
	if not await expect_pair(current_sequence, 128, "independent-streams-%d" % generation):
		return false
	for wave in range(3):
		var first := 32 + wave * 32
		var peer_first := 144 + wave * 32
		var suspended := wave == 0 and class_name_under_test == "CefTexture"
		if suspended:
			surface.set_process(false)
		# Timers keep changing the producer while the independent peer continues
		# pumping CEF and rendering. Only completed captures may reach the display.
		browser.call("eval", "window.startBurst(%d,32,8)" % first)
		peer_browser.call("eval", "window.startBurst(%d,32,8)" % peer_first)
		if suspended:
			if not await expect_pair(current_sequence, peer_first + 31, "delayed-publication-%d" % generation, true):
				return false
			surface.set_process(true)
		if not await expect_pair(first + 31, peer_first + 31, "burst-%d-%d" % [generation, wave]):
			return false
		current_sequence = first + 31
	await destroy_peer_browser()
	if finished:
		return false
	paint(128)
	return await expect_sequence(128, "surviving-stream-%d" % generation)


func run() -> void:
	for generation in range(2):
		create_browser()
		if not await expect_sequence(1, "initial-%d" % generation):
			return
		var retained_texture: Texture2DRD = null
		if accelerated and class_name_under_test == "CefTexture":
			retained_texture = surface.texture as Texture2DRD
			if retained_texture == null:
				fail("Accelerated node did not expose a Texture2DRD")
				return
		for sequence in range(2, 6):
			paint(sequence)
			if not await expect_sequence(sequence, "paint-%d-%d" % [generation, sequence]):
				return
		var current_sequence := 5
		for size in [Vector2i(320, 180), Vector2i(200, 160), Vector2i(256, 192)]:
			if not resize_browser(size):
				return
			current_sequence += 1
			# Only the page's actual new layout can paint this unique sequence;
			# stretching the previously published texture cannot satisfy it.
			# Nodes request physical pixels after viewport stretch; CEF converts
			# them to integer CSS dimensions using the page's devicePixelRatio.
			# Standalone texture_size already specifies the CSS dimensions.
			var requested := Vector2(size)
			var physical_pixels := class_name_under_test == "CefTexture"
			if physical_pixels:
				requested *= get_viewport().get_stretch_transform().x.x
			browser.call("eval", "window.paintWhenSize(%d,%f,%f,%s)" % [current_sequence, requested.x, requested.y, "true" if physical_pixels else "false"])
			if not await expect_sequence(current_sequence, "resize-%d-%dx%d" % [generation, size.x, size.y], true):
				return
			if retained_texture != null:
				if surface.texture != retained_texture:
					fail("Resize replaced the externally retained Texture2DRD wrapper")
					return
				checks += 1
		get_tree().paused = true
		paint(current_sequence + 1)
		if class_name_under_test == "CefTexture":
			if not await expect_frozen(current_sequence, "paused node"):
				return
		else:
			if not await expect_sequence(current_sequence + 1, "resource during pause"):
				return
		get_tree().paused = false
		current_sequence += 1
		if not await expect_sequence(current_sequence, "resume"):
			return
		if class_name_under_test == "CefTexture":
			surface.set_process(false)
			paint(current_sequence + 1)
			if not await expect_frozen(current_sequence, "process-disabled node"):
				return
			surface.set_process(true)
			current_sequence += 1
			if not await expect_sequence(current_sequence, "processing reenabled"):
				return
			# A retained node must reconnect its hook when reattached.
			remove_child(surface)
			await get_tree().process_frame
			add_child(surface)
			current_sequence += 1
			paint(current_sequence)
			if not await expect_sequence(current_sequence, "tree reentry"):
				return
		if not await stress_independent_streams(current_sequence, generation):
			return
		await destroy_browser()
		if finished:
			return
		if retained_texture != null:
			# Godot 4.6 clears the backing texture and size on detach but leaves
			# get_texture_rd_rid() cached; inspect the actual empty texture instead.
			# https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/scene/resources/texture_rd.cpp#L70-L81
			if retained_texture.get_width() != 0 or retained_texture.get_height() != 0:
				fail("Destroyed node left its retained Texture2DRD populated")
				return
			checks += 1
	finish()


func fail(message: String) -> void:
	if finished:
		return
	failures.append(message)
	finish()


func finish() -> void:
	if finished:
		return
	finished = true
	get_tree().paused = false
	var result := {
		"class": class_name_under_test, "accelerated_requested": accelerated,
		"driver": RenderingServer.get_current_rendering_driver_name(),
		"thread_model": ProjectSettings.get_setting("rendering/threads/thread_model"),
		"checks": checks, "passed": failures.is_empty(), "failures": failures,
		"samples": samples, "elapsed_ms": Time.get_ticks_msec() - start_ms,
		"page_dimensions": page_dimensions,
	}
	var file := FileAccess.open(evidence.path_join("result.json"), FileAccess.WRITE)
	if file != null:
		file.store_string(JSON.stringify(result, "  "))
	print("GDCEF_RENDER_RESULT " + JSON.stringify(result))
	get_tree().quit(0 if failures.is_empty() else 1)
