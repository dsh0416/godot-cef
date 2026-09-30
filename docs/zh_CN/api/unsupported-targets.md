---
title: 不支持目标的自编译
description: 为自行维护的 Godot CEF 插件构建 Windows ARM64 和 Linux ARM64 二进制。
---

# 不支持目标的自编译

官方二进制和 CI 仅覆盖 Windows x86_64、Linux x86_64、macOS universal（x86_64/ARM64）。
官方插件不再构建或分发 Windows ARM64 和 Linux ARM64。
移除这些二进制目标是**破坏性的分发变更**，可能需要发布新的主版本；见 [#238](https://github.com/dsh0416/godot-cef/issues/238)。

源码打包器仍接受这两种 ARM64 目标，供用户自行维护插件。这是尽力保留的源码构建入口，
不代表官方测试或支持。本页命令依据当前打包器的目标和资源布局整理；本次变更并未验证
ARM64 构建、编辑器启动、渲染或导出。匹配版本的 CEF 运行时和可用的目标编译器/sysroot
仍是前提。无法验证替代构建时，请保留此前提供该目标的版本。Windows x86_64 模拟运行同样未经验证。

## 通用前提

使用需要自行维护的确切 tag 或提交的干净 checkout。安装 Git 和 mise，并在仓库根目录运行命令。
`mise install` 安装 `mise.toml` 固定的 Rust nightly 和 `export-cef-dir`。
使用该 checkout 的 `CEF_VERSION`，不要随意换用较新的 CEF 运行时。
还需要 C++ 编译器、CMake，以及目标系统/架构的 Godot 4.5+。构建可能占用较多磁盘和内存。

`cargo xtask bundle` 按**宿主操作系统**选择打包器：Windows 构建应在 Windows 上运行，
Linux 构建应在 Linux 上运行。在 macOS 上传入 Windows/Linux 目标并不能跨系统构建。

## 在 Windows x64 宿主上构建 Windows ARM64

安装 Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”、MSVC ARM64 工具和 Windows SDK。
使用配置为 x64 宿主、ARM64 目标（`amd64_arm64`）的开发者命令提示符。
例如，在 `cmd.exe` 中将路径替换为实际 Visual Studio 安装位置，然后从该环境启动 PowerShell，继承编译器环境：

```bat
call "<Visual Studio installation>\VC\Auxiliary\Build\vcvarsall.bat" amd64_arm64
pwsh -NoProfile
```

安装 CMake 和 PowerShell 7（`pwsh`），确保位于 `PATH`，然后运行：

```powershell
mise trust
mise install
mise exec -- pwsh -NoProfile
# The following commands run inside this mise environment.
$env:CEF_PATH = "$env:USERPROFILE/.local/share/cef_windows_arm64"
export-cef-dir --version $env:CEF_VERSION --target aarch64-pc-windows-msvc --force $env:CEF_PATH
rustup target add aarch64-pc-windows-msvc
cargo xtask bundle --release --target aarch64-pc-windows-msvc
```

不要让 `CEF_PATH` 指向 x64 运行时。打包器将扩展、helper、DLL、locales 和 CEF 资源复制到
`addons/godot_cef/bin/aarch64-pc-windows-msvc/`。
默认 Cargo 产物位于 `target/aarch64-pc-windows-msvc/release/`。
在 Windows ARM64 原生宿主上构建需要 ARM64 宿主/ARM64 目标的开发环境及对应工具；本页未验证该宿主配置。

## 在 Linux x64 宿主上构建 Linux ARM64

以 Debian/Ubuntu 类宿主为例，安装常规依赖以及 ARM64 交叉编译器和 binutils：

```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake libgtk-3-dev libnss3-dev \
  libatk1.0-dev libatk-bridge2.0-dev libcups2-dev libdrm-dev \
  libxkbcommon-dev libxcomposite-dev libxdamage-dev libxrandr-dev \
  libgbm-dev libpango1.0-dev libasound2-dev \
  gcc-aarch64-linux-gnu g++-aarch64-linux-gnu binutils-aarch64-linux-gnu
mise trust
mise install
mise exec -- bash
# The following commands run inside this mise environment.
export CEF_PATH="$HOME/.local/share/cef_linux_arm64"
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
export-cef-dir --version "$CEF_VERSION" --target aarch64-unknown-linux-gnu --force "$CEF_PATH"
rustup target add aarch64-unknown-linux-gnu
cargo xtask bundle --release --target aarch64-unknown-linux-gnu
```

仓库 `.cargo/config.toml` 选择 `aarch64-linux-gnu-gcc`，并允许交叉链接时 `libcef.so`
存在未解析的依赖。这些共享库仍必须在目标系统上可用。
宿主开发包不等于 ARM64 运行时/sysroot；编译器或 CEF 构建需要时，请安装目标架构依赖，
或配置兼容的 ARM64 sysroot。核对目标系统是否满足 CEF 的 glibc 和系统库要求。

Linux 打包器使用 `aarch64-linux-gnu-strip`，将运行时资源部署到
`addons/godot_cef/bin/aarch64-unknown-linux-gnu/`。
默认 Cargo 产物位于 `target/aarch64-unknown-linux-gnu/release/`。
打包器也接受 Linux ARM64 原生构建，但当前 linker/strip 配置仍要求上述
`aarch64-linux-gnu-*` 工具名；发行版命名不同时需自行调整本地工具链。本页未验证原生 ARM64 宿主配置。

## 安装并注册本地插件

把仓库整个 `addons/godot_cef/` 复制到 Godot 项目的 `addons/godot_cef/`，
或把完整 ARM64 `bin/<target>/` 目录复制到来自同一提交的已有插件。
扩展旁应保留 helper、locales 目录和每个运行时资源。复制/归档 Linux
`gdcef_helper` 和 `chrome-sandbox` 时保留可执行权限。

官方 `godot_cef.gdextension` 不包含 Windows/Linux ARM64 注册。
在**你的本地副本**已有 `[libraries]` 节添加对应库条目，并在已有 `[dependencies]` 节添加对应依赖块。
下面示例包含两个目标，只添加实际构建的目标。不要重复节标题，也不要移除项目仍需使用的官方目标条目。

```ini
[libraries]
windows.arm64 = "bin/aarch64-pc-windows-msvc/gdcef.dll"
linux.arm64 = "bin/aarch64-unknown-linux-gnu/libgdcef.so"

[dependencies]
windows.arm64 = {
  "bin/aarch64-pc-windows-msvc/gdcef.dll" : "",
  "bin/aarch64-pc-windows-msvc/gdcef_helper.exe" : "",

  "bin/aarch64-pc-windows-msvc/locales" : "",
  "bin/aarch64-pc-windows-msvc/bootstrap.exe" : "",
  "bin/aarch64-pc-windows-msvc/bootstrapc.exe" : "",
  "bin/aarch64-pc-windows-msvc/chrome_100_percent.pak" : "",
  "bin/aarch64-pc-windows-msvc/chrome_200_percent.pak" : "",
  "bin/aarch64-pc-windows-msvc/chrome_elf.dll" : "",
  "bin/aarch64-pc-windows-msvc/d3dcompiler_47.dll" : "",
  "bin/aarch64-pc-windows-msvc/dxcompiler.dll" : "",
  "bin/aarch64-pc-windows-msvc/dxil.dll" : "",
  "bin/aarch64-pc-windows-msvc/icudtl.dat" : "",
  "bin/aarch64-pc-windows-msvc/libcef.dll" : "",
  "bin/aarch64-pc-windows-msvc/resources.pak" : "",
  "bin/aarch64-pc-windows-msvc/v8_context_snapshot.bin" : "",
  "bin/aarch64-pc-windows-msvc/vk_swiftshader.dll" : "",
  "bin/aarch64-pc-windows-msvc/vk_swiftshader_icd.json" : "",
  "bin/aarch64-pc-windows-msvc/vulkan-1.dll" : ""
}

linux.arm64 = {
  "bin/aarch64-unknown-linux-gnu/libgdcef.so" : "",
  "bin/aarch64-unknown-linux-gnu/gdcef_helper" : "",

  "bin/aarch64-unknown-linux-gnu/chrome-sandbox" : "",
  "bin/aarch64-unknown-linux-gnu/libcef.so" : "",
  "bin/aarch64-unknown-linux-gnu/libvk_swiftshader.so" : "",
  "bin/aarch64-unknown-linux-gnu/libvulkan.so.1" : "",
  "bin/aarch64-unknown-linux-gnu/v8_context_snapshot.bin" : "",
  "bin/aarch64-unknown-linux-gnu/vk_swiftshader_icd.json" : "",
  "bin/aarch64-unknown-linux-gnu/icudtl.dat" : "",
  "bin/aarch64-unknown-linux-gnu/resources.pak" : "",
  "bin/aarch64-unknown-linux-gnu/chrome_100_percent.pak" : "",
  "bin/aarch64-unknown-linux-gnu/chrome_200_percent.pak" : "",
  "bin/aarch64-unknown-linux-gnu/locales" : ""
}
```

这些路径对应 `xtask/src/platform.rs` 中当前的 Windows/Linux 运行时资源列表。
确认所有引用的文件和目录存在后再注册目标。debug/release 配置不会改变插件中的目标路径。

## 在目标系统验证

`cargo xtask pack` **只包含官方目标**，会忽略 Windows/Linux ARM64 构件，
不能用于打包你的 ARM64 私有插件。
`cargo xtask validate` 同样只检查官方目标目录：混合插件验证成功不代表 ARM64 内容正确，
纯 ARM64 插件会报告找不到受支持平台目录。私有分发和验证需要自行维护。

在目标系统上使用匹配架构的原生 ARM64 Godot 编辑器和导出的游戏，检查扩展加载、helper 启动、
网页渲染/输入，以及导出后的全部运行时依赖。
Linux 可**在 ARM64 目标系统上**对 `libgdcef.so`、`gdcef_helper`、`libcef.so` 运行 `ldd`，
排查缺少的共享库。Windows 需要匹配的 MSVC 运行时和 CEF 依赖。
交叉编译成功并不保证运行时兼容或导出资源完整。

Windows 和 Linux 的 Vulkan Hook 加速都要求 x86_64，因为 `retour` 不支持 ARM64。
建议先使用软件渲染（`enable_accelerated_osr = false`）；见 [Vulkan 支持](./vulkan-support)。
本页不保证任何 ARM64 加速后端或模拟运行兼容性。自行验证所依赖的后端，并在升级时重新构建和维护二进制。
