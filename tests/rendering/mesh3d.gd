extends Node3D

const COLORS := [Color.RED, Color.GREEN, Color.BLUE, Color.YELLOW]
const DEADLINE_MS := 90000
var browser: Object
var mesh_instance: MeshInstance3D
var material: StandardMaterial3D
var initial_rid: RID
var class_name_under_test := ""
var accelerated := false
var evidence := ""
var checks := 0
var failures: Array[String] = []
var samples: Array[Dictionary] = []
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
	if class_name_under_test != "CefTexture2D" or evidence.is_empty():
		fail("mesh3d scenario requires CefTexture2D and valid output path")
		return
	if not ClassDB.class_exists(class_name_under_test):
		fail("Production extension did not register " + class_name_under_test)
		return
	call_deferred("run")


func _process(_delta: float) -> void:
	if not finished and Time.get_ticks_msec() - start_ms > DEADLINE_MS:
		fail("3D mesh rendering test exceeded its internal deadline")


func setup_scene() -> void:
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 2.0
	camera.position = Vector3(0.0, 0.0, 2.0)
	add_child(camera)

	# Bind CefTexture2D to StandardMaterial3D immediately after instantiation,
	# before any process/draw frame creates the underlying browser. This mirrors
	# scene deserialization where BaseMaterial3D caches texture->get_rid() once.
	browser = ClassDB.instantiate(class_name_under_test)
	browser.set("enable_accelerated_osr", accelerated)
	browser.set("texture_size", Vector2i(256, 192))
	browser.set("url", OS.get_environment("GDCEF_RENDER_URL"))
	var texture := browser as Texture2D
	initial_rid = texture.get_rid()
	if not initial_rid.is_valid():
		fail("CefTexture2D returned an invalid RID before browser creation")
		return

	material = StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_texture = texture

	var plane := PlaneMesh.new()
	plane.size = Vector2(2.0, 2.0)
	plane.material = material

	mesh_instance = MeshInstance3D.new()
	mesh_instance.mesh = plane
	mesh_instance.rotation_degrees = Vector3(90.0, 0.0, 0.0)
	add_child(mesh_instance)


func frame_image() -> Image:
	await RenderingServer.frame_post_draw
	return get_viewport().get_texture().get_image()


func read_sequence(image: Image) -> int:
	if image == null or image.is_empty():
		return -1
	# Orthogonal camera of height 2.0 on 640x360 maps the 2x2 plane to a 360x360
	# square centered horizontally at x in [140, 500], y in [0, 360].
	var origin := Vector2(140.0, 0.0)
	var size := Vector2(360.0, 360.0)
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
				image.save_png(evidence.path_join(label + ".png"))
				return true
		else:
			matched = 0
	if not finished:
		var image := await frame_image()
		image.save_png(evidence.path_join("failure.png"))
		fail("%s: did not observe stable sequence %d on MeshInstance3D (last decoded %d)" % [label, sequence, read_sequence(image)])
	return false


func assert_stable_rid(label: String) -> bool:
	var current_rid := (browser as Texture2D).get_rid()
	if current_rid != initial_rid:
		fail("%s: CefTexture2D RID changed from %s to %s" % [label, initial_rid, current_rid])
		return false
	checks += 1
	return true


func run() -> void:
	setup_scene()
	if finished:
		return
	if not await expect_sequence(1, "mesh3d-initial"):
		return
	if not assert_stable_rid("after-initial-paint"):
		return
	for sequence in range(2, 5):
		browser.call("eval", "window.paint(%d)" % sequence)
		if not await expect_sequence(sequence, "mesh3d-paint-%d" % sequence):
			return
	var current_sequence := 4
	for size in [Vector2i(320, 180), Vector2i(256, 192)]:
		browser.set("texture_size", size)
		current_sequence += 1
		browser.call("eval", "window.paintWhenSize(%d,%f,%f,false)" % [current_sequence, float(size.x), float(size.y)])
		if not await expect_sequence(current_sequence, "mesh3d-resize-%dx%d" % [size.x, size.y], true):
			return
		if not assert_stable_rid("after-resize-%dx%d" % [size.x, size.y]):
			return
	var id := browser.get_instance_id()
	browser.call("shutdown")
	# Keep rendering the material for a few frames after shutdown to ensure
	# the persistent proxy safely points back to the fallback texture.
	for _frame in range(4):
		await RenderingServer.frame_post_draw
	if not assert_stable_rid("after-shutdown"):
		return
	material.albedo_texture = null
	mesh_instance.queue_free()
	browser = null
	material = null
	mesh_instance = null
	for _frame in range(12):
		await get_tree().process_frame
	if is_instance_id_valid(id):
		fail("CefTexture2D survived teardown after MeshInstance3D release")
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
	var result := {
		"class": class_name_under_test, "accelerated_requested": accelerated,
		"driver": RenderingServer.get_current_rendering_driver_name(),
		"thread_model": ProjectSettings.get_setting("rendering/threads/thread_model"),
		"checks": checks, "passed": failures.is_empty(), "failures": failures,
		"samples": samples, "elapsed_ms": Time.get_ticks_msec() - start_ms,
		"scenario": "mesh3d",
	}
	var file := FileAccess.open(evidence.path_join("result.json"), FileAccess.WRITE)
	if file != null:
		file.store_string(JSON.stringify(result, "  "))
	print("GDCEF_RENDER_RESULT " + JSON.stringify(result))
	get_tree().quit(0 if failures.is_empty() else 1)
