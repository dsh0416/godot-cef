---
title: 运行时安装
description: 使用原生编辑器插件安装 CEF、准备导出平台，或离线导入运行时。
---

# 运行时安装

Godot CEF 包含两个原生扩展。安装器扩展无需 CEF 即可加载，负责编辑器中的安装窗口；
浏览器扩展提供 `CefTexture`、`CefTexture2D` 和现有浏览器 API。
两者均使用 Rust 编写，无需 GDScript 安装器，也不需要用户安装额外命令行工具。

## 首次安装

1. 将插件解压到 `addons/godot_cef`，使用 Godot 4.6+ 打开项目。
2. 在 **Godot CEF Setup** 中选择任意目标平台，或点击 **Select all platforms** 全选。
3. 点击 **Install selected**。插件下载固定版本的 CEF，进行校验，并展开浏览器库中内嵌的 helper。
4. 安装完成后点击 **Save and restart editor**。

可以通过 **Project > Tools > Godot CEF Setup** 重新打开安装界面。
下载由用户明确触发并在后台执行；取消或安装失败会保留此前安装的版本。
成功安装的运行时可以离线使用。

在任意宿主系统上都可以下载全部五个目标：Windows x86_64/ARM64、Linux x86_64/ARM64
以及 universal macOS。本机平台默认选中，也可以取消勾选，仅下载其他平台的组件。
编辑器使用 CEF 时仍需安装本机组件。Windows 和 Linux 可以下载 macOS 组件，
最终签名需在 Mac 上完成。

浏览器扩展需要在 Godot 创建渲染服务器之前安装渲染 hook，因此首次安装后必须重启编辑器。
重启并加载浏览器扩展后，使用 CEF 类型的场景和脚本才能正常解析。

## 分发包与版本

Store ZIP 包含两个原生库、内嵌 helper 数据、图标和安装元数据，不包含独立的 helper 可执行文件或 CEF 运行时。
其中有效的 `.gdextension` 只描述安装器；选定平台完整安装后，安装器在编辑器正常退出或重启时
发布浏览器扩展清单，供下次启动加载。

完整 ZIP 包含相同的原生库，CEF 和 helper 已经安装。
两种包都覆盖 Windows 和 Linux x86_64/ARM64，以及 universal macOS。

原生库和浏览器组件位于 `bin/<target>/<plugin-version>-cef-<cef-version>/`。
升级使用新的版本目录，避免覆盖仍在运行的旧浏览器所加载的文件。CEF 版本与扩展构建锁定，
不要混用不同版本的 CEF，也不要从其他应用复制 CEF DLL。旧版本目录不会自动删除。

## 离线安装

完整包是最直接的离线安装方式。也可以在安装器中选择单个 Windows 或 Linux 平台，点击 **Import CEF archive…**，
导入版本和平台完全匹配的 `cef_binary_*_minimal.tar.bz2` 压缩包。

对于已经通过 `export-cef-dir` 展开的 CEF 目录，保留其中的 `archive.json`，并使用下方命令行安装方式。
macOS 需要两种架构的运行时，并在 macOS 上完成最终 bundle 签名。
在其他操作系统上准备的 macOS 运行时会保持“待签名”状态，直到在 Mac 上完成安装。

## 无界面安装与 CI

原生安装器接受 Godot `--` 分隔符之后的显式参数：

```sh
godot --headless --editor --path /path/to/project -- --godot-cef-install=host
godot --headless --editor --path /path/to/project -- --godot-cef-install=all
godot --headless --editor --path /path/to/project -- --godot-cef-install=x86_64-pc-windows-msvc
godot --headless --editor --path /path/to/project -- --godot-cef-install=host --godot-cef-source=/path/to/cef
```

`--godot-cef-source` 接受已展开的 CEF 目录或匹配的压缩包；未指定时从 CEF 分发服务器下载。
目标列表可以使用逗号分隔。安装成功时退出码为零，失败时为非零，并以
`GODOT_CEF_INSTALL_OK` 标记成功。后续导入、测试或导出命令应使用新的 Godot 进程。

导出前可以显式检查目标平台：

```sh
godot --headless --editor --path /path/to/project -- --godot-cef-check=x86_64-pc-windows-msvc
```

只有检查命令退出码为零，CI 才应继续运行 Godot 的导出命令。
每次检查还会确认本机已加载当前版本的浏览器扩展，检查其他导出平台时也一样。
检查失败时先安装或修复对应平台，不要仅凭编辑器能打开就认为跨平台导出依赖已经完整。

## 导出

导出前安装每个目标平台。导出游戏携带其浏览器运行时和 helper，启动时不需要下载组件。
macOS 应在最终应用签名前完成 helper apps 与 CEF Framework 的组装。
安装器扩展在导出游戏中不会执行安装或联网操作。

原生导出插件会检查目标平台并报告缺失组件；缺少实际原生依赖时，普通可执行项目导出还会在 Godot
复制依赖的步骤失败。Godot 4.6 的导出插件 API 没有通用的取消导出接口，单纯报错不保证导出进程失败。
因此 CI 和仅导出 PCK 的流程必须先运行 `--godot-cef-check=<target>` 并检查退出码。
PCK 本身不包含完整的原生运行环境。

运行时安装清单和安装器模板属于编辑器数据，不作为游戏资源导出。
浏览器依赖按照 GDExtension 的要求放在 PCK 外，由生成的浏览器清单声明。
原生安装器库在导出游戏中保留但不运行编辑器功能，以兼容 Godot 4.6 的扩展列表导出行为。

## 构建插件

构建任务先编译 helper，生成压缩 payload，再把它嵌入浏览器库。
安装器库独立编译，不链接 CEF。构建和打包检查会验证最终原生库中的 payload 与元数据相符。
具体命令见[分发版本与源码构建](./distribution-variants.md)。
