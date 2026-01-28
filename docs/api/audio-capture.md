# Audio Capture

Godot CEF supports two modes for handling browser audio:

1. **Direct Playback (Default):** Audio plays directly through the system's default audio output. This is simpler and has lower latency.
2. **Audio Capture:** Audio is captured and routed through Godot's audio system, allowing you to process, mix, or spatialize browser audio.

## Enabling Audio Capture

Audio capture is configured via **Project Settings** and applies to all `CefTexture` instances.

1. Go to **Project → Project Settings**
2. Navigate to **godot_cef → audio**
3. Enable **enable_audio_capture**

::: warning
Audio capture mode must be configured before any browsers are created. Changing this setting requires restarting your Godot application.
:::

## How It Works

When audio capture is enabled:

1. CEF sends audio data to Godot instead of playing it directly
2. Audio is buffered as PCM samples in an internal queue
3. You create an `AudioStreamGenerator` and connect it to an `AudioStreamPlayer`
4. Each frame, you push the buffered audio to the playback

```
[CEF Browser] → [Audio Handler] → [Buffer Queue] → [AudioStreamGenerator] → [AudioStreamPlayer]
```

## Basic Usage

```gdscript
extends Control

@onready var cef_texture: CefTexture = $CefTexture
@onready var audio_player: AudioStreamPlayer = $AudioStreamPlayer

func _ready():
    # Check if audio capture is enabled in project settings
    if cef_texture.is_audio_capture_enabled():
        # Create and assign the audio stream
        var audio_stream = cef_texture.create_audio_stream()
        audio_player.stream = audio_stream
        audio_player.play()

func _process(_delta):
    # Push audio data every frame
    if cef_texture.is_audio_capture_enabled():
        var playback = audio_player.get_stream_playback()
        if playback:
            cef_texture.push_audio_to_playback(playback)
```

## API Reference

### Methods

#### `is_audio_capture_enabled() -> bool`

Returns `true` if audio capture mode is enabled in project settings.

```gdscript
if cef_texture.is_audio_capture_enabled():
    print("Audio capture is enabled")
```

#### `create_audio_stream() -> AudioStreamGenerator`

Creates and returns an `AudioStreamGenerator` configured with the correct sample rate (matching Godot's audio output).

::: tip
The sample rate is automatically read from Godot's `AudioServer.get_mix_rate()`, ensuring compatibility with your project's audio settings.
:::

```gdscript
var audio_stream = cef_texture.create_audio_stream()
audio_player.stream = audio_stream
```

#### `push_audio_to_playback(playback: AudioStreamGeneratorPlayback) -> int`

Pushes buffered audio data from CEF to the given playback. Returns the number of frames pushed.

Call this method every frame in `_process()` to continuously feed audio data.

```gdscript
func _process(_delta):
    var playback = audio_player.get_stream_playback()
    if playback:
        var frames_pushed = cef_texture.push_audio_to_playback(playback)
```

#### `has_audio_data() -> bool`

Returns `true` if there is audio data available in the buffer.

```gdscript
if cef_texture.has_audio_data():
    print("Audio data available")
```

#### `get_audio_buffer_size() -> int`

Returns the number of audio packets currently buffered.

```gdscript
var buffer_size = cef_texture.get_audio_buffer_size()
print("Buffered packets: ", buffer_size)
```

## Advanced Usage

### 3D Spatial Audio

You can use `AudioStreamPlayer3D` to spatialize browser audio in 3D space:

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

### Multiple Browsers

Each `CefTexture` has its own audio buffer. You can route different browsers to different audio players:

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

### Audio Processing with AudioEffects

Since browser audio goes through Godot's audio system, you can apply AudioEffects:

1. Create an AudioBus in the Audio tab
2. Add effects (reverb, EQ, compression, etc.)
3. Set your AudioStreamPlayer to use that bus

```gdscript
audio_player.bus = "BrowserAudio"  # Custom bus with effects
```

## Comparison: Direct Playback vs Audio Capture

| Feature | Direct Playback | Audio Capture |
|---------|-----------------|---------------|
| Setup Complexity | None | Requires code |
| Latency | Lower | Slightly higher |
| CPU Usage | Lower | Slightly higher |
| 3D Spatialization | ❌ | ✅ |
| Audio Effects | ❌ | ✅ |
| Volume Control | System only | Full Godot control |
| Multiple outputs | ❌ | ✅ |
| Audio mixing | ❌ | ✅ |

## Troubleshooting

### No Audio

1. Verify `enable_audio_capture` is enabled in Project Settings
2. Ensure you're calling `push_audio_to_playback()` every frame
3. Check that `audio_player.play()` has been called
4. Verify the AudioStreamPlayer volume is not zero

### Audio Stuttering

- Ensure `push_audio_to_playback()` is called in `_process()`, not `_physics_process()`
- Check that your game maintains a stable framerate
- The internal buffer can hold ~100 packets; if you're processing too slowly, audio may be dropped

### Audio Delay

Audio capture inherently adds a small amount of latency due to buffering. If low latency is critical and you don't need audio processing, consider using direct playback mode instead.
