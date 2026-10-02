---
title: 分发版本与源码构建
description: 选择完整 GitHub Release 或精简 Asset Store 插件，并构建自定义 ARM64 二进制。
---

# 分发版本与源码构建

Godot CEF 构建全部受支持的架构，同时生成两个分发包，以应对 Asset Store 的版本大小限制。
完整包继续提供 Windows/Linux ARM64 二进制。此方案不移除这些平台的支持，也不会改变完整包的默认行为。
背景讨论见 [#238](https://github.com/dsh0416/godot-cef/issues/238) 和 [#239](https://github.com/dsh0416/godot-cef/pull/239)。

| 分发包 | Release 文件名 | 平台目录 |
|---|---|---|
| 完整包（默认） | `godot_cef-v<version>.zip` | Windows x86_64/ARM64、Linux x86_64/ARM64、macOS universal |
| Asset Store 精简包 | `godot_cef-store-v<version>.zip` | Windows x86_64、Linux x86_64、macOS universal |

macOS universal 始终同时包含 x86_64 和 ARM64。CI 将两个 ZIP 作为独立构件上传，分别命名为
`godot_cef-addon` 和 `godot_cef-store-addon`。标签的 release 流程会把两者附加到草稿 release；
构建 PR 不会发布 release。

## 选择与切换

需要 Windows/Linux ARM64 原生 Godot 编辑器或导出时，请下载**完整包**。
Store 包不注册这两种架构，原生 ARM64 进程不能从中加载扩展。
Windows x86_64 模拟运行不是经本次改动验证的替代方案。

两个 ZIP 都保持原有 `dist/addons/godot_cef/` 布局，并安装为项目中的 `addons/godot_cef/`。
每个包只有一个名为 `godot_cef.gdextension` 的清单，其注册与包内二进制相符。
切换版本时先备份本地修改，然后**替换整个插件目录**，不要叠加解压到旧目录。
不要同时安装两个插件副本或清单，以免重复注册扩展类。

完整清单来自仓库 `addons/godot_cef/godot_cef.gdextension`。
Store 打包器自动移除其中 Windows/Linux ARM64 库条目和依赖块，也不复制这两种架构的构件。
两个分发包共享版本和 API；区别在于包含的架构。最终 ZIP 大小以实际构建测量为准，
更小的目标集合不代表已经确认满足 Asset Store 的精确字节限制。

## 在本地生成两种分发包

构件输入布局为 `artifacts/gdcef-<target>/`，其中包含该目标的扩展、helper 和 CEF 运行时资源。
`cargo xtask pack` 默认选择 `full`，保持此前行为；Store 版本需显式指定。
在仓库根目录运行以下命令（已启用 mise 环境）：

```bash
cargo xtask pack --artifacts artifacts --output staging/full/dist/addons/godot_cef --variant full
cargo xtask validate --addon staging/full/dist/addons/godot_cef --variant full
cargo xtask pack --artifacts artifacts --output staging/store/dist/addons/godot_cef --variant store
cargo xtask validate --addon staging/store/dist/addons/godot_cef --variant store
(cd staging/full && zip -r ../../godot_cef.zip dist)
(cd staging/store && zip -r ../../godot_cef-store.zip dist)
```

输入目标为 `universal-apple-darwin`、`x86_64-pc-windows-msvc`、`aarch64-pc-windows-msvc`、
`x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu`。
使用 `validate --variant` 时，必须存在所选版本的全部目标，且不得包含被排除的目标目录。
不传 `--variant` 则保留原有的部分插件验证行为。
打包输出目录会被重建；请使用独立暂存目录，不要将源码插件目录作为输出。

## 自定义 ARM64 源码构建

通常直接使用完整的预编译包即可。需要自行维护构建时，可使用以下现有源码入口。
命令按当前构建输入和资源布局整理；本地验证未执行 ARM64 编译、编辑器启动、渲染或导出，
完整平台构建由 PR CI 验证。编译器、CEF 可用性和目标系统运行时兼容性仍需自行确认。

## 通用前提

使用需要自行维护的确切 tag 或提交的干净 checkout。安装 Git 和 mise，并在仓库根目录运行命令。
`mise install` 安装 `mise.toml` 固定的 Rust nightly 和 `export-cef-dir`。
激活工具链后，以下命令从该 checkout 的 `Cargo.lock` 提取 `CEF_VERSION`，确保运行时与 Rust 绑定一致。
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
$env:CEF_VERSION = cargo run --locked --quiet -p xtask -- cef-version
if ($LASTEXITCODE -ne 0) { throw "Could not resolve CEF runtime version" }
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
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
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

## 安装与验证自编译插件

`cargo xtask bundle` 将完整资源部署到仓库 `addons/godot_cef/bin/<target>/`。
复制整个 `addons/godot_cef/` 到项目，或将目标目录复制到同一提交的完整插件。
仓库的完整 `.gdextension` 已注册 Windows/Linux ARM64，无需手工添加清单条目。
如果项目此前使用 Store 包，应切换为完整包的清单，并确保其引用的目标文件与实际部署相符。
不要复制缺少依赖的扩展 DLL/SO，也不要同时启用两个 `.gdextension`。
保留 Linux helper 和 `chrome-sandbox` 的可执行权限。

在目标系统上验证原生 ARM64 Godot 编辑器及导出游戏的扩展加载、helper 启动、网页渲染/输入、
以及导出资源。Linux 可在 ARM64 目标系统上对 `libgdcef.so`、`gdcef_helper`、`libcef.so` 运行 `ldd`。
Windows 需要匹配的 MSVC 运行时与 CEF 依赖。交叉编译成功不保证编辑器/导出运行时兼容。
Windows/Linux 的 Vulkan Hook 加速仍要求 x86_64；ARM64 先用软件渲染验证，见 [Vulkan 支持](./vulkan-support)。
