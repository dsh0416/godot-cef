---
title: Distribution Variants
description: Choose an offline package or the native runtime installer, and build custom ARM64 binaries.
---

# Distribution variants

Godot CEF produces two distributions with the same platform coverage and browser
API. They differ in whether CEF and the helper applications are already installed.

| Package | Release filename | Included components |
|---|---|---|
| Full (default) | `godot_cef-v<version>.zip` | Native setup and browser libraries, CEF, and prepared helpers; usable offline |
| Asset Store | `godot_cef-store-v<version>.zip` | Native setup and browser libraries with embedded helper data; CEF is installed from the editor |

**Both packages support Windows x86_64/ARM64, Linux x86_64/ARM64, and macOS
universal.** macOS universal includes x86_64 and ARM64. CI uploads separate ZIP
artifacts named `godot_cef-addon` and `godot_cef-store-addon`. The tag release
workflow attaches both to a draft release; PR builds do not publish releases.

## Choose or switch packages

Use the **full package** when you need an offline installation. Use the **Store
package** for a smaller initial download, then select this computer's platform
and any export platforms in **Godot CEF Setup**. Installation downloads the exact
CEF version required by the addon and expands its embedded helpers. Restart the
editor when setup finishes. See [runtime installation](./runtime-installation.md)
for offline imports and command-line setup.

Both ZIPs preserve the existing `dist/addons/godot_cef/` archive layout and install
as `addons/godot_cef/` in the project. There are two native libraries per platform:
`gdcef_bootstrap` provides the CEF-independent Rust editor installer, and `gdcef`
provides the browser API. No GDScript installer is required.

`godot_cef_bootstrap.gdextension` is active in both packages. The Store package
ships the browser descriptor as `godot_cef.gdextension.in`; after installation,
setup publishes the active `godot_cef.gdextension` when the editor exits or restarts.
It includes only fully installed platforms. The full package
already contains that active descriptor. `runtime-install.json` records exact
versions, platform paths, required files, and the embedded helper payloads.

Native files live in
`bin/<target>/<plugin-version>-cef-<cef-version>/`. This separates runtime
versions and avoids replacing a library used by a running editor or game.
Close the editor and running games, back up local changes, and **replace the whole addon directory** when switching
packages instead of extracting over an older installation. Do not install both
package copies together; the setup and browser descriptors within one addon are
intended to coexist. The Store ZIP does not contain standalone executable helpers
or CEF components. Its size reduction comes from deferred runtime installation,
not from dropping ARM64 support. Measure final ZIP sizes from actual artifacts.

## Generate both packages locally

Inputs use `artifacts/gdcef-<target>/`, containing that target's two native
libraries, prepared helper and CEF runtime assets, and `runtime-platform.json`
from the assembly step. `mise run pack` defaults to `full`; the Store variant is
explicit. The Store packer keeps native libraries and embedded helper data while
omitting expanded helpers and CEF. From the repository root with
the mise environment active:

```bash
mise run pack --artifacts artifacts --output staging/full/dist/addons/godot_cef --variant full
mise run validate --addon staging/full/dist/addons/godot_cef --variant full
mise run pack --artifacts artifacts --output staging/store/dist/addons/godot_cef --variant store
mise run validate --addon staging/store/dist/addons/godot_cef --variant store
(cd staging/full && zip -ry ../../godot_cef.zip dist)
(cd staging/store && zip -ry ../../godot_cef-store.zip dist)
```

The input targets are `universal-apple-darwin`, `x86_64-pc-windows-msvc`,
`aarch64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, and
`aarch64-unknown-linux-gnu`. `validate --variant full|store` requires all five
targets, checks payload checksums against the final libraries, and verifies the
appropriate descriptor and runtime layout. Store validation also rejects
unexpected files, including expanded runtime executables. Omitting `--variant`
allows validation of a partial addon. Packing recreates the output
folder; stage separately and do not use the source addon directory as output.

Downloaded CI artifacts contain an inner `gdcef-<target>.tar`. Extract it into
`artifacts/gdcef-<target>/` before packing, preserving Unix permissions and
symlinks. The final ZIP commands above also preserve symlinks with `-y`.

## Custom ARM64 source builds

Both prebuilt distributions include ARM64 native libraries. If you need to
maintain your own build, use the source entry points below and verify them on the
destination system. A successful cross-compilation does not verify ARM64 editor,
rendering, or exported-game behavior. PR CI builds the native Linux target;
the full platform build matrix runs on main, tags, and manual workflows.

## Common prerequisites

Use a clean checkout of the exact tag or commit you intend to maintain. Install
Git and mise, then run commands from the repository root. `mise install` provides
the Rust nightly and `export-cef-dir` pinned by `mise.toml`. After activating
the toolchain, the commands below derive `CEF_VERSION` from that checkout's
`Cargo.lock` so the runtime matches the Rust bindings. A C++ compiler, CMake, and
Godot 4.6+ for the target OS/architecture are also required. Builds can consume
substantial disk space and memory.

`mise run bundle` dispatches by the **host OS**: use Windows for Windows
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
pwsh
```

Install CMake and PowerShell 7 (`pwsh`) and make them available on `PATH`. Run:

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

Do not point `CEF_PATH` at an x64 runtime. The bundled output includes both native
libraries, the helper, DLLs, locales and CEF resources, and is deployed to
`addons/godot_cef/bin/aarch64-pc-windows-msvc/<plugin-version>-cef-<cef-version>/`.
Cargo's target build output is
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
CEF_VERSION="$(mise run --quiet cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --target aarch64-unknown-linux-gnu --force "$CEF_PATH"
rustup target add aarch64-unknown-linux-gnu
mise run bundle --release --target aarch64-unknown-linux-gnu
```

The repository's `.cargo/config.toml` selects `aarch64-linux-gnu-gcc` and permits
unresolved dependencies from `libcef.so` during cross linking. Those libraries
must still exist on the target system. Host development packages alone do not
supply an ARM64 runtime/sysroot; install target-architecture dependencies or
configure a compatible ARM64 sysroot if required by the compiler or CEF build.
Check the CEF runtime's glibc/system-library requirements against the destination.

The Linux bundler uses `aarch64-linux-gnu-strip`, copies runtime assets and deploys
to `addons/godot_cef/bin/aarch64-unknown-linux-gnu/<plugin-version>-cef-<cef-version>/`.
Cargo's default target output
is `target/aarch64-unknown-linux-gnu/release/`. A native Linux ARM64 build is also
accepted, but the current linker/strip configuration still requires the named
`aarch64-linux-gnu-*` tools; adjust your local toolchain if your distribution uses
other names. No native ARM64 host setup is validated here.

## Install and validate a custom build

`mise run bundle` deploys the complete runtime to the repository's
`addons/godot_cef/bin/<target>/<plugin-version>-cef-<cef-version>/`, with generated
installation metadata and both extension descriptors. The build first compiles
the helper, compresses it, and embeds it in the browser library; the setup library
does not link CEF. Use `mise run bundle` for distributable output rather than
copying the result of a plain `cargo build`.

Copy the whole assembled `addons/godot_cef/` into the project. To combine platform
builds, collect their assembly artifacts and use `mise run pack` so metadata and
library registrations are generated together. Do not copy only a DLL/SO or
manually combine descriptors from different versions. Preserve Linux helper and
`chrome-sandbox` executable permissions. Restart the editor after replacing an
addon that was already loaded.

On the ARM64 destination, verify native Godot editor and exported-game extension
loading, helper startup, page rendering/input and exported runtime dependencies.
On Linux, run `ldd` on `libgdcef.so`, `gdcef_helper` and `libcef.so` on that
ARM64 system. Windows needs the matching MSVC runtime and CEF dependencies.
Cross-compilation success does not guarantee editor/export runtime compatibility.
Windows/Linux Vulkan hook acceleration still requires x86_64; start ARM64 runtime
validation with software rendering; see [Vulkan support](./vulkan-support).
