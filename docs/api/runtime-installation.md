---
title: Runtime Installation
description: Install CEF from the native editor plugin, prepare export platforms, or install offline.
---

# Runtime installation

Godot CEF has two native extensions. The setup extension loads without CEF and
provides the editor installation window. The browser extension provides
`CefTexture`, `CefTexture2D`, and the existing browser API. Both are written in
Rust; there is no GDScript installer or external command-line tool to install.

## First installation

1. Extract the addon into `addons/godot_cef` and open the project in Godot 4.6+.
2. In **Godot CEF Setup**, select any platforms or click **Select all platforms**.
3. Click **Install selected**. The plugin downloads the pinned CEF distribution,
   verifies it, and extracts the helper embedded in the native browser library.
4. Click **Save and restart editor** after installation finishes.

You can reopen setup through **Project > Tools > Godot CEF Setup**. Downloads are
explicit and run in the background. Cancellation or failure leaves previously
installed versions intact. Successful installations can be used offline.

All five targets are available on every host: Windows x86_64/ARM64, Linux
x86_64/ARM64, and universal macOS. The host platform is selected by default, but
you can deselect it to download only other platforms. The editor needs its own
platform installed to use CEF. Downloading macOS components on Windows or Linux
is supported; their final signing must be completed on a Mac.

Restarting is required because the browser extension installs rendering hooks
before Godot creates its rendering server. Scenes and scripts using CEF types
become available after the browser extension loads on restart.

## Packages and versions

The Store ZIP contains the two native libraries, embedded helper data, icons,
and installation metadata. It contains no standalone helper executable or CEF
runtime. Its active `.gdextension` describes only the setup library. Setup writes
the browser descriptor when the editor exits or restarts after installation.

The full ZIP contains the same libraries with CEF and the helpers already
installed. Both packages cover Windows and Linux x86_64/ARM64 and universal macOS.

Native libraries and browser components live in
`bin/<target>/<plugin-version>-cef-<cef-version>/`. Upgrades install into a new
version directory, so a running older browser is not overwritten. Do not mix
CEF releases or copy a CEF DLL from another application: the runtime version is
locked to the extension's build. Previous version directories are not deleted
automatically.

## Offline installation

The full package is the simplest offline option. Alternatively, select one
Windows or Linux platform in setup and use **Import CEF archive…** with the exact matching
`cef_binary_*_minimal.tar.bz2` archive. The archive's platform and CEF version
must match the installation metadata.

For an already extracted `export-cef-dir` runtime, retain its `archive.json`
metadata and use the command-line installer below. Setup checks this metadata
against the pinned CEF version and target. macOS requires both architecture runtimes and performs final bundle
signing on macOS. A macOS runtime prepared on another OS remains pending signing
until setup is completed on a Mac.

## Headless installation and CI

The native setup plugin accepts explicit arguments after Godot's `--` separator:

```sh
godot --headless --editor --path /path/to/project -- --godot-cef-install=host
godot --headless --editor --path /path/to/project -- --godot-cef-install=all
godot --headless --editor --path /path/to/project -- --godot-cef-install=x86_64-pc-windows-msvc
godot --headless --editor --path /path/to/project -- --godot-cef-install=host --godot-cef-source=/path/to/cef
```

`--godot-cef-source` accepts an extracted CEF directory or a matching archive.
Without it, setup downloads from the CEF distribution server. A comma-separated
target list installs a subset. The installer exits with zero on success and a
nonzero code on failure; `GODOT_CEF_INSTALL_OK` marks successful completion.
Run subsequent import, test, or export commands in a fresh Godot process.

Before exporting in CI, run the explicit runtime preflight and require its zero
exit code:

```sh
godot --headless --editor --path /path/to/project -- --godot-cef-check=x86_64-pc-windows-msvc
```

Use `host` to check the editor's platform. Every preflight also confirms that the
current host browser extension loaded, including when checking another export
platform. `GODOT_CEF_CHECK_OK` marks success. The preflight does not download missing
components. Run it separately before a normal export or a PCK-only export.

## Exporting

Install each destination platform before exporting. The exported game carries
its browser runtime and helpers and does not download them at startup. On macOS,
assemble the helper apps and CEF Frameworks before the final application signing.
The setup extension performs no installation or network activity in a running
exported game.

Godot 4.6's export plugin API cannot abort every export error. Missing native
files are reported to the native exporter, while other validation errors are
shown in the export log. The explicit preflight above is required for a reliable
CI failure, particularly for PCK-only exports. The small setup library remains
in Godot's exported extension list and is inactive outside the editor.

The runtime installation manifest and setup templates are editor data, not game
assets. Keep browser dependencies outside the PCK as required by GDExtension;
the generated browser descriptor declares the dependencies for installed targets.

## Building the addon

The build tasks first compile the helper, create its compressed payload, then
compile the browser library with that payload embedded. The setup library is
compiled independently without linking CEF. Build and packaging checks verify
the payload metadata against the final native library. See
[distribution variants](./distribution-variants.md) for commands.
