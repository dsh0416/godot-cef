---
title: Building Unsupported Targets
description: Build Windows ARM64 and Linux ARM64 from source for a privately maintained Godot CEF addon.
---

# Building unsupported targets

Official binaries and CI cover Windows x86_64, Linux x86_64, and macOS universal
(x86_64/ARM64). Windows ARM64 and Linux ARM64 are no longer built or distributed
in official addons. Removing those binary targets is a **breaking distribution
change** and may require a major release; see [#238](https://github.com/dsh0416/godot-cef/issues/238).

The source bundlers still accept both ARM64 targets for users maintaining their
own addon. This is a best-effort source-build path, not an officially tested
platform. The instructions follow the current bundler's target and runtime asset
layout; ARM64 builds, editor startup, rendering and exports have not been validated
for this change. Availability of a matching CEF runtime and a working target
compiler/sysroot remains a prerequisite. Keep an earlier release if you cannot
validate a replacement. Windows x86_64 emulation is also unvalidated.

## Common prerequisites

Use a clean checkout of the exact tag or commit you intend to maintain. Install
Git and mise, then run commands from the repository root. `mise install` provides
the Rust nightly and `export-cef-dir` pinned by `mise.toml`. Use that checkout's
`CEF_VERSION`, not an arbitrary newer CEF runtime. A C++ compiler, CMake, and
Godot 4.5+ for the target OS/architecture are also required. Builds can consume
substantial disk space and memory.

`cargo xtask bundle` dispatches by the **host OS**: use Windows for Windows
builds and Linux for Linux builds. Passing a Windows/Linux target on macOS does
not provide a cross-OS build path.

## Windows ARM64 from a Windows x64 host

Install Visual Studio 2022 Build Tools with Desktop development with C++, the
MSVC ARM64 build tools and a Windows SDK. Use a Developer Command Prompt configured
for x64-host/ARM64-target (`amd64_arm64`). For example, in `cmd.exe`, replace the
installation path with your actual Visual Studio location, then launch PowerShell
from that configured prompt so it inherits the compiler environment:

```bat
call "<Visual Studio installation>\VC\Auxiliary\Build\vcvarsall.bat" amd64_arm64
pwsh -NoProfile
```

Install CMake and PowerShell 7 (`pwsh`) and make them available on `PATH`. Run:

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

Do not point `CEF_PATH` at an x64 runtime. The bundled output includes the helper,
DLLs, locales and CEF resources, and is deployed to
`addons/godot_cef/bin/aarch64-pc-windows-msvc/`. Cargo's target build output is
`target/aarch64-pc-windows-msvc/release/` with the default target directory.
A native Windows ARM64 host requires an ARM64-host/ARM64-target developer
environment and matching native tools; that host setup is not validated here.

## Linux ARM64 from a Linux x64 host

For a Debian/Ubuntu-style host, install the ordinary build dependencies and the
ARM64 cross compiler/binutils:

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

The repository's `.cargo/config.toml` selects `aarch64-linux-gnu-gcc` and permits
unresolved dependencies from `libcef.so` during cross linking. Those libraries
must still exist on the target system. Host development packages alone do not
supply an ARM64 runtime/sysroot; install target-architecture dependencies or
configure a compatible ARM64 sysroot if required by the compiler or CEF build.
Check the CEF runtime's glibc/system-library requirements against the destination.

The Linux bundler uses `aarch64-linux-gnu-strip`, copies runtime assets and deploys
to `addons/godot_cef/bin/aarch64-unknown-linux-gnu/`. Cargo's default target output
is `target/aarch64-unknown-linux-gnu/release/`. A native Linux ARM64 build is also
accepted, but the current linker/strip configuration still requires the named
`aarch64-linux-gnu-*` tools; adjust your local toolchain if your distribution uses
other names. No native ARM64 host setup is validated here.

## Install and register your local addon

Copy the repository's whole `addons/godot_cef/` directory to your Godot project's
`addons/godot_cef/`, or copy the complete ARM64 `bin/<target>/` directory into an
existing addon from the same commit. Keep helper executables, locale directories
and every runtime resource beside the extension library. Preserve Linux executable
permissions when copying or archiving `gdcef_helper` and `chrome-sandbox`.

The official `godot_cef.gdextension` intentionally has no ARM64 Windows/Linux
entries. In **your local copy**, add the relevant library entry to the existing
`[libraries]` section and its dependency block to the existing `[dependencies]`
section. The example shows both targets; include only the target(s) you built.
Do not duplicate section headers or remove supported entries that your project
still needs.

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

These paths match the current Windows/Linux runtime asset lists in
`xtask/src/platform.rs`. Do not register a target until all referenced files and
directories exist. Build/debug profile changes do not change the addon paths.

## Validate on the destination

`cargo xtask pack` includes **only official targets** and intentionally ignores
ARM64 Windows/Linux artifacts. It cannot package your private ARM64 addon.
`cargo xtask validate` also checks only official target directories; a success
for a mixed addon does not validate its ARM64 contents, and an ARM64-only addon
will report no supported platform directories. Keep private distribution and
validation under your own control.

Run the matching native ARM64 Godot editor and exported game on the destination,
verify extension loading, helper startup, page rendering/input and all exported
runtime dependencies. For Linux, use `ldd` on `libgdcef.so`, `gdcef_helper` and
`libcef.so` **on the ARM64 destination** to diagnose missing shared libraries.
Windows requires the matching MSVC runtime and CEF dependencies. Cross compilation
success alone proves neither runtime compatibility nor export completeness.

Vulkan hook-based acceleration requires x86_64 on both Windows and Linux because
`retour` does not support ARM64. Start with software rendering
(`enable_accelerated_osr = false`); see [Vulkan support](./vulkan-support). No ARM64
accelerated backend or emulation compatibility is promised by this guide. Test
any backend you rely on and maintain your own rebuilt binaries across upgrades.
