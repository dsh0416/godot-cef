# 权限

`CefTexture` 和 `CefTexture2D` 提供相同的权限接口，处理 CEF 上报的摄像头、
麦克风、局域网访问等请求。应用可以显示 Godot 对话框，并异步回答请求。

## 策略与生命周期

通过浏览器的 `permission_policy` 属性配置策略：

| 值 | 行为 |
|----|------|
| `-1`（默认） | 继承 `godot_cef/security/default_permission_policy` |
| `0` | 拒绝请求 |
| `1` | 允许已知权限请求，不显示应用层提示 |
| `2` | 发出 `permission_requested`，由应用决定 |

项目默认策略是 `0`。请先连接信号，再将策略设为 `2` 或加载会请求权限的内容。
如果没有连接 `permission_requested` 监听器，请求会取消且不授权，完成结果为
`dismissed`。CEF 权限提示会关闭，不记为用户显式拒绝该站点。
这只取消当前请求所在的权限组，不影响此前已经交给应用处理的其他请求组。
未知的权限位会被拒绝，即使策略为 `1` 也一样。

项目设置 `godot_cef/security/permission_request_timeout_seconds` 控制应答期限，
默认为 `60` 秒，最小为 `1` 秒；请在创建浏览器之前配置。
未回答的请求会在期限到达时过期。超时处理依赖 Godot 主循环，主线程阻塞会推迟
通知；应答方法仍会检查实际期限，拒绝迟到的回答。在运行时更改 `permission_policy`
会取消待处理请求。导航、关闭浏览器或渲染进程终止也会使尚未完成的请求失效。

对于 CEF 权限提示，超时和生命周期取消会关闭提示，而不是记为用户显式拒绝；
应用显式拒绝时使用 CEF 的拒绝结果。媒体请求在超时或生命周期结束时取消，
显式拒绝时则按拒绝处理。
这些操作都不会清除 Chromium 已经保存的权限决定。

CEF 可能在一次请求中包含多项权限。每种权限分别发出信号，并具有独立的
`request_id`，但**整个请求按一个整体授权**：所有 ID 都获准后才允许 CEF 请求，
任意一个 ID 被拒绝都会拒绝整组。例如，同时请求摄像头和麦克风时不能只授予
摄像头权限。`grant_permission()` 返回 `true` 仅表示成功记录该项回答，
不代表整组权限已获准。

## 信号与方法

| API | 含义 |
|-----|------|
| `permission_requested(permission_type: String, url: String, request_id: int)` | 请求应用决定是否授权 |
| `permission_request_finished(request_id: int, result: String)` | 清理此 ID 对应的待处理界面 |
| `grant_permission(request_id: int) -> bool` | 记录允许决定 |
| `deny_permission(request_id: int) -> bool` | 拒绝请求及同组权限 |
| `is_permission_pending(request_id: int) -> bool` | 此 ID 是否仍可回答 |
| `get_permission_setting(permission_type: String, requesting_url: String, top_level_url: String) -> String` | 查询支持的权限在当前配置中的内容设置 |

对已超时、不存在、已失效或已回答的 ID 调用应答方法会返回 `false`。
不要重复使用 ID，也不要把它作为持久权限记录。

整个 CEF 请求结束时，同组的所有 ID 都会收到相同的完成结果，包括之前已经回答
允许的 ID。完成信号的 `result` 可能是 `allowed`、`denied`、`dismissed`、`timed_out`、
`navigation`、`browser_closed`、`renderer_terminated` 或 `policy_changed`。
`allowed` 表示权限决定完成，不保证对应设备或网络操作一定成功。

只有 Godot 对象仍存活时才能送达完成信号。显式调用 `CefTexture2D.shutdown()`
可以报告 `browser_closed`，但释放节点或资源时不保证发出最终信号。
浏览器或其所有者释放时，也应主动清理对话框，不能依赖已销毁对象的最终信号。

`url` 是 **CEF 提供的请求来源 origin**，不是下载地址、目标局域网地址或顶层页面 URL。
校验可信来源时应精确比较 CEF 序列化的完整 origin，包括协议、主机、端口和
HTTP(S) origin 末尾的 `/`，例如 `https://trusted.example/`。字符串前缀匹配也可能接受
`https://trusted.example.attacker.test` 这样的其他主机。

## 查询配置中的权限设置

两种浏览器类型都提供同步、只读的查询方法：

```gdscript
var setting: String = browser.get_permission_setting(
    "local_network", "https://app.example/", "https://app.example/"
)
print("局域网权限的配置值：", setting)
```

浏览器准备好后，在 Godot 主线程调用。`requesting_url` 是 CEF 的 primary URL，
`top_level_url` 是 secondary URL，分别表示请求页面与顶层页面。
顶层页面自己发起请求时，两个参数传入相同的页面 URL。嵌入页面场景应显式提供
两个 URL；权限请求信号只提供请求来源 origin。Chromium 根据设置类型决定匹配规则，
例如存储访问设置可能同时取决于两个站点。查询局域网权限时应传入页面 URL，
而不是目标设备的 IP 地址。

两个参数都必须是带主机名的绝对 HTTP(S) URL，且不能包含用户名或密码。
允许包含路径、查询参数和片段，由 CEF 解释。空字符串、相对地址、`res://`、
`user://`、`file://` 及其他协议会返回 `invalid_url`；方法不会自动使用当前页面地址。

| 返回值 | 含义 |
|--------|------|
| `allow` | 当前适用的内容设置为允许 |
| `block` | 当前适用的内容设置为阻止 |
| `ask` | 当前适用的内容设置为询问 |
| `default` | CEF 返回默认或无值标记，不等同于 `ask` |
| `session_only` | CEF 返回仅会话内容设置 |
| `unknown` | CEF 返回其他内容设置值 |
| `unsupported` | 此权限标签没有受支持的标量查询映射 |
| `unavailable` | 浏览器或请求上下文不可用，或调用不在 CEF UI 线程 |
| `invalid_url` | 至少一个 URL 不符合上述要求 |

查询使用浏览器共享的请求上下文，结果包含适用的配置默认值。它不区分结果来自
显式站点决定还是默认值，也不说明值会保留多久。查询不会修改 `permission_policy`、
消费待处理请求或弹出权限提示。

**配置值不保证当前操作一定获准。** 它没有合并应用策略、安全上下文、Permissions
Policy、操作系统权限或设备可用性。尤其是 CEF 摄像头和麦克风请求回调可以允许
捕获，而不更新配置中的内容设置，因此 `grant_permission()` 成功不保证查询变为
`allow`。应通过操作本身的结果判断是否成功。

支持查询的标签为 `ar_session`、`camera_pan_tilt_zoom`、`camera`、
`captured_surface_control`、`clipboard`、`top_level_storage_access`、`local_fonts`、
`hand_tracking`、`idle_detection`、`microphone`、`midi_sysex`、`notifications`、
`keyboard_lock`、`pointer_lock`、`storage_access`、`vr_session`、
`web_app_installation`、`window_management`、`local_network`、`loopback_network`
和 `sensors`。`clipboard` 查询读写设置，不涵盖所有剪贴板操作。各类型默认值不同，
例如 `sensors` 没有经过应用提示也可能返回 `allow`。

其他标签返回 `unsupported`，即使它们可能出现在 `permission_requested` 中。
这包括 `geolocation`（CEF 154 可能使用结构化定位设置）、已弃用的
`local_network_access`、桌面捕获、文件访问，以及没有单一跨平台标量设置的权限。
此方法不暴露任意 CEF 内容设置类型，也不清除已保存的决定。

底层约定见锁定版本的
[CEF 请求上下文 API](https://github.com/chromiumembedded/cef/blob/564dd6c/include/cef_request_context.h#L240)
和 [Chromium 权限映射](https://github.com/chromium/chromium/blob/154.0.8037.58/components/permissions/request_type.cc#L383)。

## 在 Godot 中排队显示权限对话框

为下面的 `Control` 添加 `CefTexture` 和 `ConfirmationDialog` 子节点。
此示例每次只显示一个请求，并在请求完成或失效后清理队列与界面。
也可以将 `browser` 引用替换为 `CefTexture2D` 资源引用。

```gdscript
extends Control

@onready var browser: CefTexture = $CefTexture
@onready var dialog: ConfirmationDialog = $ConfirmationDialog

var pending: Array[Dictionary] = []
var active_id: int = -1

func _ready() -> void:
    dialog.ok_button_text = "允许"
    dialog.cancel_button_text = "拒绝"
    dialog.confirmed.connect(_answer.bind(true))
    dialog.canceled.connect(_answer.bind(false))
    browser.permission_requested.connect(_on_permission_requested)
    browser.permission_request_finished.connect(_on_permission_finished)
    browser.permission_policy = 2

func _on_permission_requested(kind: String, origin: String, id: int) -> void:
    pending.append({"id": id, "kind": kind, "origin": origin})
    _show_next.call_deferred()

func _show_next() -> void:
    if active_id != -1 or not is_instance_valid(browser):
        return
    while not pending.is_empty():
        var request: Dictionary = pending.pop_front()
        var id: int = request["id"]
        if not browser.is_permission_pending(id):
            continue
        active_id = id
        dialog.dialog_text = "%s 请求 %s 权限，是否允许？" % [
            request["origin"], request["kind"]
        ]
        dialog.popup_centered()
        return

func _answer(allow: bool) -> void:
    var id: int = active_id
    active_id = -1
    dialog.hide()
    if is_instance_valid(browser) and browser.is_permission_pending(id):
        if allow:
            browser.grant_permission(id)
        else:
            browser.deny_permission(id)
    # 等当前对话框的按钮事件结束后再显示下一个请求。
    _show_next.call_deferred()

func _on_permission_finished(id: int, _result: String) -> void:
    for index in range(pending.size() - 1, -1, -1):
        if pending[index]["id"] == id:
            pending.remove_at(index)
    if active_id == id:
        active_id = -1
        dialog.hide()
    _show_next.call_deferred()
```

建议让对话框与浏览器具有相同的所有者。应用单独释放浏览器时，也应在相应的
清理流程中隐藏对话框，并清空 `pending` 和 `active_id`。

如果应用使用来源白名单，可先精确比较 origin，直接拒绝不在白名单内的请求，
再将其余请求加入 `pending`。仍需用户决定的权限继续通过对话框询问。

## 权限类型名称

下列名称对应项目当前附带的 CEF 版本。部分类型只适用于特定平台或 Chrome 风格
的浏览器功能；存在对应绑定不代表每个平台都能使用该 Web API。

| 分类 | `permission_type` 值 |
|------|----------------------|
| 本地网络连接 | `local_network`、`loopback_network` |
| 旧版本地网络连接 | `local_network_access`（CEF 已弃用的权限位） |
| 摄像头与麦克风 | `camera`、`microphone`、`camera_pan_tilt_zoom` |
| 屏幕捕获 | `desktop_audio_capture`、`desktop_video_capture`、`captured_surface_control` |
| 位置与设备 | `geolocation`、`midi_sysex`、`sensors`、`idle_detection` |
| 剪贴板与通知 | `clipboard`、`notifications` |
| 输入与窗口 | `keyboard_lock`、`pointer_lock`、`window_management` |
| 存储与文件 | `storage_access`、`top_level_storage_access`、`disk_quota`、`file_system_access`、`multiple_downloads` |
| XR | `ar_session`、`vr_session`、`hand_tracking` |
| 其他 | `local_fonts`、`identity_provider`、`protected_media_identifier`、`register_protocol_handler`、`web_app_installation` |
| 未识别 | `unknown_permission`、`unknown_media_permission`（不可授权） |

信号模式下可能会上报未知权限以便诊断，但尝试允许此权限仍会将整组 CEF 请求
以 `denied` 结果结束。

`local_network` 对应局域网访问，`loopback_network` 对应本机服务访问，
例如 `127.0.0.1` 或 `::1`。这些权限与摄像头权限、常规 CORS 检查相互独立。
允许局域网请求不会关闭 CORS、TLS 证书校验或其他浏览器检查。

## 浏览器与操作系统的边界

- 摄像头、麦克风和局域网 Web API 仍需满足安全上下文与 Permissions Policy 要求。
  跨来源 iframe 可能还需要父页面显式委托权限。回答允许不能绕过这些条件。
- 操作系统可能单独要求摄像头、麦克风、屏幕捕获或局域网权限。
  例如 macOS 应用需要对应的用途说明和系统授权。此接口只回答 CEF 的请求，
  不会代替操作系统授权，也不负责选择捕获设备。
- Chromium 可能在当前浏览器 profile 中记住权限提示的决定。此接口不保证
  授权仅对本次有效，也不会清除已有决定。上述配置查询只读取部分设置，
  不负责列出或管理持久权限存储。
  已存在的决定可能使新的操作不再触发此接口。
- 使用应用控制媒体权限时应避免 `enable-media-stream` 开关：CEF 明确说明，
  此开关会绕过媒体权限回调。

另见[安全基线](./security-baseline.md)、当前版本的
[CEF 权限契约](https://github.com/chromiumembedded/cef/blob/564dd6c/include/cef_permission_handler.h)、
[CEF 提示实现](https://github.com/chromiumembedded/cef/blob/564dd6c/libcef/browser/permission_prompt.cc)、
[Web 媒体权限要求](https://www.w3.org/TR/mediacapture-streams/#permissions-policy-integration)
和 [Chrome 的局域网访问说明](https://developer.chrome.com/blog/local-network-access)。
