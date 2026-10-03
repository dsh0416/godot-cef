# 加速帧交接

`CefTexture` 和 `CefTexture2D` 在 `RenderingServer.frame_pre_draw` 发布浏览器纹理。
该信号在主线程执行，通过 `call_on_render_thread` 安排发布工作，本身不提供 GPU 完成保证。
浏览器生命周期、CEF 消息泵、事件和尺寸更新仍由 process 回调推进。

## 所有权与完成条件

每个浏览器的 view 和 popup 使用独立的流，每条流有三个持久快照槽位。
调整尺寸期间最多保留两代资源；尺寸与 BGRA/RGBA 格式共同确定资源代际。
没有空闲槽位时，在读取 CEF 借用资源之前跳过捕获，并保留最后显示的帧。
跳过的绘制会由主线程重新请求 view/popup 绘制，直到捕获成功，确保静态页面也能在
初始化、尺寸变化或槽位耗尽后恢复。

1. RD 创建并初始化 staging，将一个像素复制到每槽位独立的 sentinel，然后异步读回。
   读回完成后才能使用槽位，此时 Godot 与原生后端都处于已知的复制源状态。
   空资源通过一次性全零上传初始化，不要求 sRGB 渲染目标或 UAV 清空能力；浏览器帧始终留在 GPU。
2. `OnAcceleratedPaint` 打开当前借用源，复制到空闲槽位，恢复复制源状态，等待所有
   对借用源的 GPU 访问结束后才返回。不使用 HANDLE、IOSurface 或 DMA-BUF 标识缓存借用帧。
3. 绘制前，渲染线程选择最新完成的快照，记录 `RD.texture_copy(staging, display)`，
   由 Godot 跟踪显示纹理的复制与采样依赖。
4. 从 **display** 复制一个像素到该槽位的 sentinel，再执行
   `texture_get_data_async(sentinel)`。只有匹配资源代际与使用序号的完成回调才能释放槽位。

直接从 staging 复制 sentinel 无法保证发布复制的完成顺序。帧计数、`frame_post_draw`、
CPU 调用结束和超时都不能作为 GPU 完成证据。无法证明完成的资源会被隔离，直到设备销毁。
新显示纹理绑定后才回收旧资源，尚未完成的回调持有各自资源代际。

## 原生后端契约

| 后端 | 捕获与交接 |
| --- | --- |
| Windows D3D12 | 回调内直接通过 D3D12 打开 CEF 共享句柄，在自有队列捕获，通过 COMMON 状态获取并释放源，双向使用队列 fence 并等待捕获完成。保留 RD 的复制源状态，增强屏障与旧式屏障通过 COMMON 交接。 |
| Windows Vulkan | 每次回调重新导入 Win32 外部内存，使用 Godot 实际队列及队列族，获取并释放外部图像所有权，恢复 staging 的 `TRANSFER_SRC_OPTIMAL` 状态并等待复制完成。 |
| Linux Vulkan | 导入当前 DMA-BUF 平面与 DRM modifier，通过 sync-file 信号量等待生产者（旧内核使用原生 DMA-BUF fence 轮询），获取并释放 foreign 队列所有权，在回调返回前完成复制。 |
| macOS Metal | 打开当前 IOSurface，在 Godot 自身的线程安全 `MTLCommandQueue` 上复制到启用 hazard tracking 的 staging；同队列资源依赖保证可见性，`waitUntilCompleted` 保证借用源不再被访问。 |

Vulkan 钩子串行化实际队列的 CPU 访问，覆盖 Godot 后台传输工作。
初始化会验证设备与队列的来源；仅在渲染线程执行不能保证队列独占访问。
驱动会缓存队列包装函数的地址，因此其模块在进程生命周期内保持加载；重建扩展后需要重启 Godot。

Godot 4.5.0–4.5.2 返回内部 D3D12 队列包装对象。桥接代码按版本读取已审核的原生指针，
并校验队列所属设备。Godot 4.6 和 4.7 直接返回原生队列。未审核的 D3D12 版本会拒绝初始化。

## 错误处理与验证

原生导入器初始化失败会报告缺失能力。捕获或发布失败时保留最后成功发布的帧；
未能确认 GPU 工作完成的资源会被隔离，直到设备销毁。

图形像素与生命周期测试见
[`tests/rendering`](https://github.com/dsh0416/godot-cef/tree/main/tests/rendering)。
测试验证请求的渲染器与线程模式，遇到自动回退、引擎错误、过期像素序号、崩溃或超时即失败。
无界面测试只验证运行时推进，不能验证 GPU 共享与实际显示像素。

原生资源状态假设基于 Godot 4.5。编译通过不能代替各操作系统、GPU 驱动与 Godot 版本上的实机验证。
此项工作继续推进 [#227](https://github.com/dsh0416/godot-cef/issues/227)。
