# 兼容性矩阵

该矩阵列出 GPU 加速 OSR 支持的平台与渲染后端组合。

本页包含完整 GitHub Release 包支持的架构。精简的 Asset Store 包不含 Windows/Linux ARM64；
需要这些架构时请选择完整包。详见[分发版本](./distribution-variants)。

## 版本基线

要求 Godot 4.6 或更高版本。

CEF 运行时版本由 `Cargo.lock` 中 Rust `cef` / `cef-dll-sys` crate 的构建元数据确定。按[开发环境配置](https://github.com/dsh0416/godot-cef/blob/main/CONTRIBUTING.md#development-setup)安装并激活项目工具链后，在仓库根目录提取该版本，用于手动安装 CEF 二进制文件：

```bash
export CEF_PATH="$HOME/.local/share/cef"
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"
```

这样可以确保下载的运行时文件与 Rust 绑定保持一致。

## 受支持的 GPU 后端

| 平台 | 架构 | Godot 后端 | 集成方式 |
|------|------|------------|----------|
| Windows | x86_64、ARM64 | Direct3D12 | 原生共享纹理 |
| Windows | x86_64 | Vulkan | 扩展与队列钩子 |
| macOS | x86_64、ARM64 | Metal | 原生 IOSurface 共享 |
| Linux | x86_64 | Vulkan | 扩展与队列钩子 |

## 回退条件

即使平台理论上支持加速，以下情况也会回退到软件渲染：

- `CefTexture` 上关闭了 `enable_accelerated_osr`。
- 平台纹理导入器初始化失败。
- Vulkan 外部内存扩展注入失败或目标设备不支持。

上述条件适用于浏览器初始化阶段。加速渲染启动后，捕获或发布失败时保留最后成功发布的帧。
GPU 完成条件与资源生命周期详见[加速帧交接](./accelerated-handoff)。

## 诊断日志

启动时会输出：

- 当前检测到的后端，以及是否支持加速 OSR。
- 无法使用加速时的明确回退原因。

在创建浏览器实例时，还会输出当前实例使用的是加速渲染还是软件渲染。
