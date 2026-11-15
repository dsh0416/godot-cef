---
title: 分发版本与源码构建
description: 选择离线完整包或原生运行时安装器，并构建自定义 ARM64 二进制。
---

# 分发版本与源码构建

Godot CEF 提供两种分发包，平台覆盖和浏览器 API 相同。两者的区别是 CEF 和 helper 应用是否已经安装。

| 分发包 | Release 文件名 | 包含的组件 |
|---|---|---|
| 完整包（默认） | `godot_cef-v<version>.zip` | 原生安装器和浏览器库、CEF、已展开的 helper，可离线使用 |
| Asset Store 安装包 | `godot_cef-store-v<version>.zip` | 原生安装器和浏览器库、内嵌 helper 数据；在编辑器中安装 CEF |

**两种包都支持 Windows x86_64/ARM64、Linux x86_64/ARM64 和 macOS universal。**
macOS universal 同时包含 x86_64 和 ARM64。CI 将两个 ZIP 作为独立构件上传，分别命名为
`godot_cef-addon` 和 `godot_cef-store-addon`。标签的 release 流程会把两者附加到草稿 release；
构建 PR 不会发布 release。

## 选择与切换

需要离线安装时选择**完整包**。希望减小首次下载体积时选择 **Store 包**，然后在
**Godot CEF Setup** 中选择本机平台和需要导出的平台。安装器下载与插件匹配的 CEF，
并展开内嵌的 helper。安装完成后重启编辑器。离线导入和命令行用法见
[运行时安装](./runtime-installation.md)。

两个 ZIP 都保持原有 `dist/addons/godot_cef/` 布局，并安装为项目中的 `addons/godot_cef/`。
每个平台有两个原生库：`gdcef_bootstrap` 提供不依赖 CEF 的 Rust 编辑器安装器，
`gdcef` 提供浏览器 API，不需要 GDScript 安装脚本。

两种包都启用 `godot_cef_bootstrap.gdextension`。Store 包以
`godot_cef.gdextension.in` 保存浏览器清单模板；安装完成后，安装器在编辑器正常退出或重启时
发布 `godot_cef.gdextension`，其中只包含安装完整的平台。完整包已经包含有效的浏览器清单。
`runtime-install.json` 记录确切版本、平台路径、必需文件和内嵌 helper 数据的位置。

原生文件保存在 `bin/<target>/<plugin-version>-cef-<cef-version>/`，
不同运行时版本使用不同目录，避免覆盖正在被编辑器或游戏使用的动态库。
手动切换版本时先关闭编辑器及运行中的游戏，备份本地修改，再**替换整个插件目录**，不要叠加解压到旧目录。
不要同时安装两个分发包副本；同一插件内的安装器和浏览器清单则需要共存。
Store ZIP 不包含独立可执行的 helper 或 CEF 运行时，体积缩小来自安装时下载组件，
不再通过移除 ARM64 平台减小体积。最终 ZIP 大小以实际构建产物为准。

## 在本地生成两种分发包

构件输入布局为 `artifacts/gdcef-<target>/`，其中包含该目标的两个原生库、展开后的 helper、
CEF 运行时资源，以及组装步骤生成的 `runtime-platform.json`。
`mise run pack` 默认选择 `full`；Store 版本需显式指定。
Store 打包器保留原生库和内嵌 helper 数据，省略展开的 helper 和 CEF。
在仓库根目录运行以下命令（已启用 mise 环境）：

```bash
mise run pack --artifacts artifacts --output staging/full/dist/addons/godot_cef --variant full
mise run validate --addon staging/full/dist/addons/godot_cef --variant full
mise run pack --artifacts artifacts --output staging/store/dist/addons/godot_cef --variant store
mise run validate --addon staging/store/dist/addons/godot_cef --variant store
(cd staging/full && zip -ry ../../godot_cef.zip dist)
(cd staging/store && zip -ry ../../godot_cef-store.zip dist)
```

输入目标为 `universal-apple-darwin`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`、
`x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu`。
无论在什么系统上打包，都必须提供全部五个目标；缺少构件时会在修改输出目录前报错。
仅本地开发可以显式传入 `pack --allow-partial` 打包部分平台；这类开发包只提供所含平台，不能作为正式分发包。
使用 `validate --variant full|store` 时，必须包含全部五个目标，并验证最终原生库中的
内嵌 payload 校验和、清单以及运行时布局。Store 验证还会拒绝额外文件，包括展开的可执行组件。
不传 `--variant` 时可以验证只包含部分平台的插件。
打包输出目录会被重建；请使用独立暂存目录，不要将源码插件目录作为输出。

下载的 CI 构件包含内层 `gdcef-<target>.tar`。打包前将其解压到
`artifacts/gdcef-<target>/`，并保留 Unix 权限和符号链接。
上述最终 ZIP 命令也通过 `-y` 保留符号链接。

## 自定义 ARM64 源码构建

两种预编译包都包含 ARM64 原生库。需要自行维护构建时，可使用以下源码入口，并在目标系统上验证。
交叉编译成功不代表 ARM64 编辑器、渲染和导出游戏均可正常运行。PR CI 构建 Linux 原生目标，
完整平台矩阵在 main、tag 和手动工作流运行。编译器、CEF 可用性和目标系统兼容性仍需确认。

## 通用前提

使用需要自行维护的确切 tag 或提交的干净 checkout。安装 Git 和 mise，并在仓库根目录运行命令。
`mise install` 安装 `mise.toml` 固定的 Rust nightly 和 `export-cef-dir`。
激活工具链后，以下命令从该 checkout 的 `Cargo.lock` 提取 `CEF_VERSION`，确保运行时与 Rust 绑定一致。
还需要 C++ 编译器、CMake，以及目标系统/架构的 Godot 4.6+。构建可能占用较多磁盘和内存。

`mise run bundle` 按**宿主操作系统**选择打包器：Windows 构建应在 Windows 上运行，
Linux 构建应在 Linux 上运行。在 macOS 上传入 Windows/Linux 目标并不能跨系统构建。

## 在 Windows x64 宿主上构建 Windows ARM64

安装 Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”、MSVC ARM64 工具和 Windows SDK。
使用配置为 x64 宿主、ARM64 目标（`amd64_arm64`）的开发者命令提示符。
例如，在 `cmd.exe` 中将路径替换为实际 Visual Studio 安装位置，然后从该环境启动 PowerShell，继承编译器环境：

```bat
call "<Visual Studio installation>\VC\Auxiliary\Build\vcvarsall.bat" amd64_arm64
pwsh
```

安装 CMake 和 PowerShell 7（`pwsh`），确保位于 `PATH`，然后运行：

```powershell
mise trust
mise install
mise exec -- pwsh
# The following commands run inside this mise environment.
$env:CEF_PATH = "$env:USERPROFILE/.local/share/cef_windows_arm64"
$env:CEF_VERSION = mise run --quiet cef-version
if ($LASTEXITCODE -ne 0) { throw "Could not resolve CEF runtime version" }
export-cef-dir --version $env:CEF_VERSION --target aarch64-pc-windows-msvc --force $env:CEF_PATH
rustup target add aarch64-pc-windows-msvc
mise run bundle --release --target aarch64-pc-windows-msvc
```

不要让 `CEF_PATH` 指向 x64 运行时。打包器将两个原生库、helper、DLL、locales 和 CEF 资源复制到
`addons/godot_cef/bin/aarch64-pc-windows-msvc/<plugin-version>-cef-<cef-version>/`。
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
CEF_VERSION="$(mise run --quiet cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --target aarch64-unknown-linux-gnu --force "$CEF_PATH"
rustup target add aarch64-unknown-linux-gnu
mise run bundle --release --target aarch64-unknown-linux-gnu
```

仓库 `.cargo/config.toml` 选择 `aarch64-linux-gnu-gcc`，并允许交叉链接时 `libcef.so`
存在未解析的依赖。这些共享库仍必须在目标系统上可用。
宿主开发包不等于 ARM64 运行时/sysroot；编译器或 CEF 构建需要时，请安装目标架构依赖，
或配置兼容的 ARM64 sysroot。核对目标系统是否满足 CEF 的 glibc 和系统库要求。

Linux 打包器使用 `aarch64-linux-gnu-strip`，将运行时资源部署到
`addons/godot_cef/bin/aarch64-unknown-linux-gnu/<plugin-version>-cef-<cef-version>/`。
默认 Cargo 产物位于 `target/aarch64-unknown-linux-gnu/release/`。
打包器也接受 Linux ARM64 原生构建，但当前 linker/strip 配置仍要求上述
`aarch64-linux-gnu-*` 工具名；发行版命名不同时需自行调整本地工具链。本页未验证原生 ARM64 宿主配置。

## 安装与验证自编译插件

`mise run bundle` 将完整资源部署到仓库
`addons/godot_cef/bin/<target>/<plugin-version>-cef-<cef-version>/`，同时生成安装元数据和两个扩展清单。
构建流程先编译 helper，再将其压缩并嵌入浏览器库；安装器原生库不链接 CEF。
需要可分发产物时使用 `mise run bundle`，不要直接复制普通 `cargo build` 的输出。

将组装后的整个 `addons/godot_cef/` 复制到项目。需要合并多个平台时，收集各平台组装构件，
再运行 `mise run pack`，使元数据与库注册一同生成。不要只复制 DLL/SO，也不要手动混合不同版本的清单。
保留 Linux helper 和 `chrome-sandbox` 的可执行权限；替换已被加载的插件后重启编辑器。

在目标系统上验证原生 ARM64 Godot 编辑器及导出游戏的扩展加载、helper 启动、网页渲染/输入、
以及导出资源。Linux 可在 ARM64 目标系统上对 `libgdcef.so`、`gdcef_helper`、`libcef.so` 运行 `ldd`。
Windows 需要匹配的 MSVC 运行时与 CEF 依赖。交叉编译成功不保证编辑器/导出运行时兼容。
Windows/Linux 的 Vulkan Hook 加速仍要求 x86_64；ARM64 先用软件渲染验证，见 [Vulkan 支持](./vulkan-support)。
