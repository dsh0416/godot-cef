# GPU 设备绑定

本页面解释 Godot CEF 如何在多 GPU 系统上确保 CEF 与 Godot 使用同一块 GPU，从而在启用加速渲染时实现可靠的纹理共享。

## 多 GPU 问题

现代系统通常有多个 GPU：

- **笔记本电脑** — 集成 GPU（Intel/AMD）+ 独立 GPU（NVIDIA/AMD）
- **台式机** — 多个独立 GPU 用于多显示器设置
- **工作站** — 专业 GPU 与消费级 GPU 共存

在进程之间共享纹理（Godot 和 CEF 的渲染器）时，两者必须使用**相同的物理 GPU**。底层 API 不支持跨 GPU 纹理共享。

### 没有设备绑定会发生什么

没有显式 GPU 选择：
1. Godot 选择一个 GPU（通常是独立 GPU 以获得更好性能）
2. CEF 的渲染器子进程独立选择一个 GPU（通常默认选择索引 0，即集成 GPU）
3. Godot 从 GPU A 导出纹理句柄
4. CEF 尝试在 GPU B 上导入它
5. **导入失败** — 句柄在不同设备上无效

这会导致黑色纹理或渲染失败。

## 解决方案：命令行 GPU 选择

Godot CEF 使用 Chromium 的 `--gpu-vendor-id` 和 `--gpu-device-id` 命令行开关来指定 CEF 应使用哪个 GPU。这种方法在所有平台上都有效，无需钩子或环境变量操作。

### 工作原理

```
┌─────────────────────────────────────────────────────────────────┐
│                        Godot Process                            │
│                                                                 │
│  1. Query RenderingDevice for GPU vendor/device IDs             │
│     - Windows D3D12: DXGI adapter description                   │
│     - Windows/Linux Vulkan: VkPhysicalDeviceProperties          │
│     - macOS Metal: IOKit registry properties                    │
│                                                                 │
│  2. Pass IDs to CEF subprocesses via command-line switches      │
│     --gpu-vendor-id=4318 --gpu-device-id=7815                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     CEF Subprocess                              │
│                                                                 │
│  Chromium's GPU process uses the vendor/device IDs to select    │
│  the matching GPU adapter for rendering                         │
└─────────────────────────────────────────────────────────────────┘
```

### 平台特定 GPU ID 获取

| 平台 | 后端 | 方法 |
|------|------|------|
| Windows | D3D12 | 查询 `IDXGIAdapter::GetDesc()` 获取 `VendorId` 和 `DeviceId` |
| Windows | Vulkan | 通过 `vkGetPhysicalDeviceProperties2` 查询 `VkPhysicalDeviceProperties` |
| Linux | Vulkan | 通过 `vkGetPhysicalDeviceProperties2` 查询 `VkPhysicalDeviceProperties` |
| macOS | Metal | 查询 IOKit 注册表获取 `vendor-id` 和 `device-id` 属性 |

### 代码流程

**步骤 1：** 在 CEF 初始化期间，Godot CEF 查询 GPU ID：

```rust
// In gdcef/src/cef_init.rs
use crate::accelerated_osr::get_godot_gpu_device_ids;
if let Some((vendor_id, device_id)) = get_godot_gpu_device_ids() {
    osr_app = osr_app.with_gpu_device_ids(vendor_id, device_id);
}
```

**步骤 2：** ID 在 `on_before_child_process_launch` 中传递给 CEF 子进程：

```rust
// In cef_app/src/lib.rs
if let Some(ids) = &self.handler.gpu_device_ids {
    command_line.append_switch_with_value(
        Some(&"gpu-vendor-id".into()),
        Some(&ids.to_vendor_arg().as_str().into()),  // e.g., "4318" (decimal)
    );
    command_line.append_switch_with_value(
        Some(&"gpu-device-id".into()),
        Some(&ids.to_device_arg().as_str().into()),  // e.g., "7815" (decimal)
    );
}
```

## 平台可用性

| 平台 | GPU 绑定 | 状态 |
|------|----------|------|
| Windows (D3D12) | 命令行开关 | ✅ 支持 |
| Windows (Vulkan) | 命令行开关 | ✅ 支持 |
| Linux (Vulkan) | 命令行开关 | ✅ 支持 |
| macOS (Metal) | 命令行开关 | ✅ 支持 |

### macOS 备注

在 Apple Silicon（M 系列芯片）上，`vendor-id` 和 `device-id` 属性在 IOKit 注册表中不存在，因为 GPU 集成在 SoC 中而不是作为独立的 PCI 设备。在这种情况下，GPU 设备绑定会被跳过。这没问题，因为 Apple Silicon Mac 只有一个 GPU — Godot 和 CEF 都会使用相同的 GPU，无需显式绑定。

## 调试 GPU 绑定

### 诊断输出

Godot CEF 在初始化期间打印 GPU 信息：

```
[AcceleratedOSR/D3D12] Godot GPU: vendor=0x10de, device=0x1e87, name=NVIDIA GeForce RTX 3080
[CefInit] Godot GPU: vendor=0x10de, device=0x1e87 - will pass to CEF subprocesses
```

### 常见问题

**黑色纹理**
- 验证 Godot 和 CEF 在日志中报告相同的 GPU
- 检查外部内存扩展是否已启用（参见 [Vulkan 支持](./vulkan-support.md)）
- 在多 GPU 系统上，确保选择了正确的 GPU

**GPU ID 获取失败**
- 检查 Godot 是否使用支持的渲染后端（D3D12、Vulkan 或 Metal）
- 验证显卡驱动程序是最新的

### 验证 GPU 选择

要确认 CEF 使用了正确的 GPU：

1. 启用 CEF 远程调试（`remote_debugging_port` 属性）
2. 打开 Chrome 开发者工具（`chrome://inspect`）
3. 在 CEF 浏览器中导航到 `chrome://gpu`
4. 检查"图形功能状态"中的活动 GPU

## 常见 GPU 供应商 ID

| 供应商 | ID |
|--------|-----|
| NVIDIA | `0x10de` |
| AMD | `0x1002` |
| Intel | `0x8086` |
| Apple | `0x106b` |

## 相比之前方法的优势

命令行开关方法相比之前基于钩子的实现有几个优势：

1. **更简单的架构** — 不需要函数钩子或虚表修补
2. **跨平台** — 相同的机制在 Windows、Linux 和 macOS 上工作
3. **更可靠** — 没有钩子安装的时序问题
4. **对防病毒友好** — 没有可能触发安全软件的内存操作

## 另请参见

- [Vulkan 支持](./vulkan-support.md) — 外部内存扩展注入
- [属性](./properties.md) — `enable_accelerated_osr` 配置

