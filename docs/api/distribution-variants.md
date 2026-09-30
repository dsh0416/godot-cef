---
title: Distribution Variants
description: Choose the full GitHub Release or smaller Asset Store addon and build custom ARM64 binaries.
---

# Distribution variants

Godot CEF builds every supported architecture and produces two distribution
packages to address the Asset Store size limit. The full package continues to
provide Windows/Linux ARM64 binaries. This approach preserves platform support
and the default full-package behavior. See [#238](https://github.com/dsh0416/godot-cef/issues/238)
and [#239](https://github.com/dsh0416/godot-cef/pull/239) for the discussion.

| Package | Release filename | Platform directories |
|---|---|---|
| Full (default) | `godot_cef-v<version>.zip` | Windows x86_64/ARM64, Linux x86_64/ARM64, macOS universal |
| Asset Store | `godot_cef-store-v<version>.zip` | Windows x86_64, Linux x86_64, macOS universal |

macOS universal always includes x86_64 and ARM64. CI uploads separate ZIP
artifacts named `godot_cef-addon` and `godot_cef-store-addon`. The tag release
workflow attaches both to a draft release; PR builds do not publish releases.

## Choose or switch packages

Use the **full package** for native Windows/Linux ARM64 Godot editors or exports.
The Store package has no registrations for those architectures, so native ARM64
processes cannot load the extension from it. Windows x86_64 emulation is not a
replacement validated by this change.

Both ZIPs preserve the existing `dist/addons/godot_cef/` archive layout and install
as `addons/godot_cef/` in the project. Each package contains one
`godot_cef.gdextension` descriptor whose registrations match its binaries.
Back up local changes and **replace the whole addon directory** when switching
packages instead of extracting over an older installation. Do not install both
addon copies or descriptors together, as they register the same extension classes.

The full descriptor comes from `addons/godot_cef/godot_cef.gdextension` in the
repository. The Store packer automatically removes Windows/Linux ARM64 library
entries and dependency dictionaries and does not copy those architecture
artifacts. Both packages share a version and API; architecture coverage differs.
Measure the final ZIP size from the actual build: a smaller target set does not
by itself confirm compliance with the Asset Store's exact byte limit.

## Generate both packages locally

Inputs use `artifacts/gdcef-<target>/`, containing the extension, helper and CEF
runtime assets for that target. `cargo xtask pack` defaults to `full`, preserving
existing behavior; the Store variant is explicit. From the repository root with
the mise environment active:

```bash
cargo xtask pack --artifacts artifacts --output staging/full/dist/addons/godot_cef --variant full
cargo xtask validate --addon staging/full/dist/addons/godot_cef --variant full
cargo xtask pack --artifacts artifacts --output staging/store/dist/addons/godot_cef --variant store
cargo xtask validate --addon staging/store/dist/addons/godot_cef --variant store
(cd staging/full && zip -r ../../godot_cef.zip dist)
(cd staging/store && zip -r ../../godot_cef-store.zip dist)
```

The input targets are `universal-apple-darwin`, `x86_64-pc-windows-msvc`,
`aarch64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, and
`aarch64-unknown-linux-gnu`. `validate --variant` requires every selected target
and rejects excluded target directories. Omitting `--variant` preserves the
existing partial-addon validation behavior. Packing recreates the output
folder; stage separately and do not use the source addon directory as output.

## Custom ARM64 source builds

Most users can choose the full prebuilt package. If you need to maintain your own
build, the existing source entry points are shown below. These commands were
reviewed against current build inputs and runtime layout; local verification did
not compile ARM64 binaries or run an ARM64 editor/rendering/export. Full platform
builds are checked by PR CI. You must still verify compiler/runtime availability
and compatibility with the destination system.

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

## Install and validate a custom build

`cargo xtask bundle` deploys the complete runtime to the repository's
`addons/godot_cef/bin/<target>/`. Copy the whole `addons/godot_cef/` into the project,
or copy the target directory into a full addon from the same commit. The full
repository descriptor already registers Windows/Linux ARM64; no manual manifest
entries are needed. If you previously used a Store package, switch to the full
descriptor and ensure its referenced target files match your deployed contents.
Do not copy only the extension DLL/SO without dependencies, or enable two
`.gdextension` descriptors together. Preserve Linux helper and `chrome-sandbox`
executable permissions.

On the ARM64 destination, verify native Godot editor and exported-game extension
loading, helper startup, page rendering/input and exported runtime dependencies.
On Linux, run `ldd` on `libgdcef.so`, `gdcef_helper` and `libcef.so` on that
ARM64 system. Windows needs the matching MSVC runtime and CEF dependencies.
Cross-compilation success does not guarantee editor/export runtime compatibility.
Windows/Linux Vulkan hook acceleration still requires x86_64; start ARM64 runtime
validation with software rendering; see [Vulkan support](./vulkan-support).
