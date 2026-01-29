# 音频捕获

Godot CEF 支持两种处理浏览器音频的模式：

1. **直接播放（默认）：** 音频直接通过系统默认音频输出播放。这更简单且延迟更低。
2. **音频捕获：** 音频被捕获并通过 Godot 的音频系统路由，允许您处理、混音或空间化浏览器音频。

## 启用音频捕获

音频捕获通过**项目设置**配置，并应用于所有 `CefTexture` 实例。

1. 转到 **项目 → 项目设置**
2. 导航到 **godot_cef → audio**
3. 启用 **enable_audio_capture**

::: warning
音频捕获模式必须在创建任何浏览器之前配置。更改此设置需要重启 Godot 应用程序。
:::

## 工作原理

启用音频捕获后：

1. CEF 将音频数据发送给 Godot 而不是直接播放
2. 音频作为 PCM 样本缓冲在内部队列中
3. 您创建一个 `AudioStreamGenerator` 并将其连接到 `AudioStreamPlayer`
4. 每帧将缓冲的音频推送到播放器

```
[CEF 浏览器] → [音频处理器] → [缓冲队列] → [AudioStreamGenerator] → [AudioStreamPlayer]
```

## 基本用法

```gdscript
extends Control

@onready var cef_texture: CefTexture = $CefTexture
@onready var audio_player: AudioStreamPlayer = $AudioStreamPlayer

func _ready():
    # 检查项目设置中是否启用了音频捕获
    if cef_texture.is_audio_capture_enabled():
        # 创建并分配音频流
        var audio_stream = cef_texture.create_audio_stream()
        audio_player.stream = audio_stream
        audio_player.play()

func _process(_delta):
    # 每帧推送音频数据
    if cef_texture.is_audio_capture_enabled():
        var playback = audio_player.get_stream_playback()
        if playback:
            cef_texture.push_audio_to_playback(playback)
```

## API 参考

### 方法

#### `is_audio_capture_enabled() -> bool`

如果项目设置中启用了音频捕获模式，返回 `true`。

```gdscript
if cef_texture.is_audio_capture_enabled():
    print("音频捕获已启用")
```

#### `create_audio_stream() -> AudioStreamGenerator`

创建并返回一个配置了正确采样率（与 Godot 音频输出匹配）的 `AudioStreamGenerator`。

::: tip
采样率会自动从 Godot 的 `AudioServer.get_mix_rate()` 读取，确保与您项目的音频设置兼容。
:::

```gdscript
var audio_stream = cef_texture.create_audio_stream()
audio_player.stream = audio_stream
```

#### `push_audio_to_playback(playback: AudioStreamGeneratorPlayback) -> int`

将 CEF 缓冲的音频数据推送到给定的播放器。返回推送的帧数。

在 `_process()` 中每帧调用此方法以持续提供音频数据。

```gdscript
func _process(_delta):
    var playback = audio_player.get_stream_playback()
    if playback:
        var frames_pushed = cef_texture.push_audio_to_playback(playback)
```

#### `has_audio_data() -> bool`

如果缓冲区中有可用的音频数据，返回 `true`。

```gdscript
if cef_texture.has_audio_data():
    print("有可用的音频数据")
```

#### `get_audio_buffer_size() -> int`

返回当前缓冲的音频数据包数量。

```gdscript
var buffer_size = cef_texture.get_audio_buffer_size()
print("缓冲数据包: ", buffer_size)
```

## 高级用法

### 3D 空间音频

您可以使用 `AudioStreamPlayer3D` 在 3D 空间中空间化浏览器音频：

```gdscript
extends Node3D

@onready var cef_texture: CefTexture = $Screen/CefTexture
@onready var audio_player: AudioStreamPlayer3D = $AudioStreamPlayer3D

func _ready():
    if cef_texture.is_audio_capture_enabled():
        var audio_stream = cef_texture.create_audio_stream()
        audio_player.stream = audio_stream
        audio_player.play()

func _process(_delta):
    if cef_texture.is_audio_capture_enabled():
        var playback = audio_player.get_stream_playback()
        if playback:
            cef_texture.push_audio_to_playback(playback)
```

### 多个浏览器

每个 `CefTexture` 都有自己的音频缓冲区。您可以将不同的浏览器路由到不同的音频播放器：

```gdscript
@onready var browser1: CefTexture = $Browser1
@onready var browser2: CefTexture = $Browser2
@onready var player1: AudioStreamPlayer = $AudioPlayer1
@onready var player2: AudioStreamPlayer = $AudioPlayer2

func _ready():
    if browser1.is_audio_capture_enabled():
        player1.stream = browser1.create_audio_stream()
        player1.play()
        
        player2.stream = browser2.create_audio_stream()
        player2.play()

func _process(_delta):
    if browser1.is_audio_capture_enabled():
        var pb1 = player1.get_stream_playback()
        var pb2 = player2.get_stream_playback()
        if pb1:
            browser1.push_audio_to_playback(pb1)
        if pb2:
            browser2.push_audio_to_playback(pb2)
```

### 使用 AudioEffects 进行音频处理

由于浏览器音频通过 Godot 的音频系统，您可以应用 AudioEffects：

1. 在音频选项卡中创建 AudioBus
2. 添加效果（混响、均衡器、压缩器等）
3. 将您的 AudioStreamPlayer 设置为使用该总线

```gdscript
audio_player.bus = "BrowserAudio"  # 带有效果的自定义总线
```

## 对比：直接播放 vs 音频捕获

| 特性 | 直接播放 | 音频捕获 |
|------|----------|----------|
| 设置复杂度 | 无 | 需要代码 |
| 延迟 | 更低 | 略高 |
| CPU 使用 | 更低 | 略高 |
| 3D 空间化 | ❌ | ✅ |
| 音频效果 | ❌ | ✅ |
| 音量控制 | 仅系统 | 完整 Godot 控制 |
| 多输出 | ❌ | ✅ |
| 音频混音 | ❌ | ✅ |

## 故障排除

### 无音频

1. 验证项目设置中是否启用了 `enable_audio_capture`
2. 确保每帧都调用 `push_audio_to_playback()`
3. 检查是否已调用 `audio_player.play()`
4. 验证 AudioStreamPlayer 音量不为零

### 音频卡顿

- 确保在 `_process()` 中调用 `push_audio_to_playback()`，而不是 `_physics_process()`
- 检查您的游戏是否保持稳定的帧率
- 内部缓冲区可容纳约 100 个数据包；如果处理太慢，音频可能会丢失

### 音频延迟

由于缓冲，音频捕获本身会增加少量延迟。如果低延迟很重要且您不需要音频处理，请考虑使用直接播放模式。

