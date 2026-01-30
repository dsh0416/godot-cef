# Vulkan 支持

本页面介绍 Godot CEF 如何通过运行时函数钩子在 Vulkan 后端启用 GPU 加速渲染，以及该方案的限制与注意事项。

## 背景

CEF 中的 GPU 加速离屏渲染（OSR）需要在 CEF 渲染器进程和宿主应用程序（Godot）之间共享纹理。这通过平台特定的外部内存 API 实现：

| 平台 | 图形 API | 共享机制 |
|------|----------|----------|
| Windows | DirectX 12 | NT 句柄（原生支持） |
| Windows | Vulkan | `VK_KHR_external_memory_win32` |
| macOS | Vulkan | `VK_EXT_metal_objects` |
| macOS | Metal | IOSurface（原生支持） |
| Linux | Vulkan | `VK_EXT_external_memory_dma_buf` `VK_KHR_external_memory_fd` |

问题是 **Godot 在创建 Vulkan 设备时默认不启用这些 Vulkan 外部内存扩展**。没有这些扩展，CEF 和 Godot 之间的纹理共享是不可能的。

## 钩子解决方案

由于 Godot 不提供在设备创建期间请求额外 Vulkan 扩展的 API，Godot CEF 使用**运行时函数钩子**来注入所需的扩展。

### 工作原理

1. 在 GDExtension 初始化期间（`Core` 阶段，在 `RenderingServer` 创建之前），我们在 `vkCreateDevice` 上安装钩子
2. 当 Godot 调用 `vkCreateDevice` 创建其 Vulkan 设备时，我们的钩子拦截调用
3. 钩子修改 `VkDeviceCreateInfo` 结构以添加所需的外部内存扩展
4. 修改后的请求传递给真正的 `vkCreateDevice` 函数
5. Godot 现在拥有启用了外部内存支持的 Vulkan 设备

### 平台特定扩展

**Windows：**
- `VK_KHR_external_memory` — 外部内存基础扩展
- `VK_KHR_external_memory_win32` — Windows 特定的句柄共享

**macOS：**
- `VK_KHR_external_memory` — 外部内存基础扩展
- `VK_EXT_metal_objects` — Metal 对象共享

**Linux：**
- `VK_KHR_external_memory` — 外部内存基础扩展
- `VK_KHR_external_memory_fd` — 基于文件描述符的共享
- `VK_EXT_external_memory_dma_buf` — DMA-BUF 共享用于零拷贝传输

## 多 GPU 支持

在具有多个 GPU 的系统上（例如笔记本电脑的集成显卡 + 独立显卡），**CEF 必须使用与 Godot 相同的 GPU** 才能使纹理共享工作。这通过传递给 CEF 子进程的命令行开关（`--gpu-vendor-id` 和 `--gpu-device-id`）处理。

::: tip
有关 GPU 设备绑定工作原理的详细信息，请参见 [GPU 设备绑定](./gpu-device-pinning.md)。
:::

## 局限性

### 架构要求（仅 x86_64）

::: warning
基于 Vulkan 钩子的加速**仅在 x86_64（64 位 x86）架构上可用**。
:::

钩子机制依赖于 [retour](https://github.com/darfink/retour-rs) 库进行运行时函数重定向。该库目前不支持 ARM64 架构，这意味着：

- **Windows ARM64** — Vulkan 钩子不可用
- **Linux ARM64** — Vulkan 钩子不可用
- **macOS（Apple Silicon）** — Vulkan 钩子不可用

在不支持的架构上，扩展会自动回退到软件渲染。

### macOS Vulkan 不支持

由于根本性的技术限制，macOS Vulkan 支持（通过 MoltenVK）无法从钩子机制中受益：

1. **静态链接** — Godot 将 MoltenVK 静态链接到其二进制文件中。这意味着 `vkCreateDevice` 调用直接进入嵌入式代码，而不是通过动态库的 PLT/GOT（过程链接表/全局偏移表）。像 retour 这样的函数钩子库通过在这些间接点拦截调用来工作，而静态链接不存在这些间接点。即使 retour 支持 ARM64，也没有可行的钩子目标。

2. **原生 Metal 替代** — macOS 已经有原生 Metal 支持，提供更好的性能且不需要任何钩子。Metal 的 IOSurface 共享机制原生工作，无需扩展注入。

3. **有限收益** — MoltenVK 是将 Vulkan 转换为 Metal 的兼容层。在 macOS 上使用 Vulkan 与直接使用 Metal 相比增加了开销。

::: tip
在 macOS 上使用 **Metal 后端** 进行 GPU 加速渲染。它是原生 API，在 Intel 和 Apple Silicon Mac 上开箱即用。
:::

### 时序敏感性

钩子必须在 Godot 创建其 Vulkan 设备**之前**安装。这就是为什么安装发生在 GDExtension 的 `Core` 初始化阶段。如果钩子安装得太晚，Vulkan 设备将在没有所需扩展的情况下创建。

### 稳定性考虑

函数钩子本质上是脆弱的：

- Vulkan 驱动程序的更新可能会改变行为
- 防病毒软件可能会标记基于钩子的修改
- 某些 Vulkan 层或调试工具可能会干扰钩子

如果您遇到加速渲染问题，请尝试：
1. 更新您的显卡驱动程序
2. 在正常使用期间禁用 Vulkan 验证层
3. 通过设置 `enable_accelerated_osr = false` 回退到软件渲染

## 平台支持摘要

| 平台 | 架构 | Vulkan 加速 OSR | 备注 |
|------|------|-----------------|------|
| Windows | x86_64 | ✅ 支持 | 通过 `vkCreateDevice` 扩展注入钩子 |
| Windows | ARM64 | ❌ 不支持 | retour 不支持 ARM64 |
| Linux | x86_64 | ✅ 支持 | 通过 `vkCreateDevice` 扩展注入钩子 |
| Linux | ARM64 | ❌ 不支持 | retour 不支持 ARM64 |
| macOS | 任意 | ❌ 不适用 | MoltenVK 静态链接阻止钩子；使用 Metal 后端 |

## 未来：正式 Godot API

这种基于钩子的方法是一种变通方案。正确的解决方案是让 Godot 提供一个 API，允许 GDExtension 在设备创建期间请求额外的 Vulkan 扩展。

此功能的提案已存在：[godotengine/godot-proposals#13969](https://github.com/godotengine/godot-proposals/issues/13969)

一旦此提案实现，Godot CEF 可以从基于钩子的方法迁移到更干净、官方支持的方法。

## 调试

安装钩子时，诊断消息会打印到 stderr：

```
[VulkanHook/Windows] Installing vkCreateDevice hook...
[VulkanHook/Windows] Hook installed successfully
[VulkanHook/Windows] Injecting external memory extensions
[VulkanHook/Windows] Adding VK_KHR_external_memory
[VulkanHook/Windows] Adding VK_KHR_external_memory_win32
[VulkanHook/Windows] Successfully created device with external memory extensions
```

在 Linux 上：
```
[VulkanHook/Linux] Installing vkCreateDevice hook...
[VulkanHook/Linux] Hook installed successfully
[VulkanHook/Linux] Injecting external memory extensions
[VulkanHook/Linux] Adding VK_KHR_external_memory
[VulkanHook/Linux] Adding VK_KHR_external_memory_fd
[VulkanHook/Linux] Adding VK_EXT_external_memory_dma_buf
[VulkanHook/Linux] Successfully created device with external memory extensions
```

如果您看到关于扩展不支持或钩子安装失败的消息，加速渲染将回退到软件模式。

## 另请参见

- [GPU 设备绑定](./gpu-device-pinning.md) — 通过命令行开关支持多 GPU
- [属性](./properties.md) — `enable_accelerated_osr` 属性文档
- [GitHub Issue #4](https://github.com/dsh0416/godot-cef/issues/4) — Vulkan 支持跟踪问题

