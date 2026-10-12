# Build tasks

Run these tasks from the repository root with the pinned mise toolchain:

```sh
mise run cef-version
mise run validate-versions
mise run bundle --release --target x86_64-unknown-linux-gnu
mise run assemble --release --target x86_64-unknown-linux-gnu
mise run pack --artifacts artifacts --output staging/full/dist/addons/godot_cef --variant full
mise run validate --addon staging/full/dist/addons/godot_cef --variant full
```

`bundle` declares a dependency on `build`, forwarding the profile, target and
target directory before running `assemble`. `assemble` never invokes Cargo;
`build` builds both `gdcef` and `gdcef_helper` with `--locked`. Cargo handles Rust
incremental compilation. Both tasks use explicit target triples, including host
builds, so binaries live in `<target-dir>/<triple>/<debug|release>/`. The output
root comes from `--target-dir`, then `CARGO_TARGET_DIR`, then the repository's
`target/`. macOS builds both architectures and additionally stages its universal
framework in `<target-dir>/<debug|release>/Godot CEF.framework`.

Set `CEF_PATH` for Linux/Windows, or `CEF_PATH_ARM64` and `CEF_PATH_X64` on macOS.
Each macOS Cargo invocation receives the corresponding `CEF_PATH`. CI restores
CEF using the exact runtime version from `Cargo.lock` before building. Local
downloads use `export-cef-dir` as described in `CONTRIBUTING.md`; version reading
and artifact packaging do not need a Rust compiler or a CEF installation.

`cef-version` and `validate-versions` depend on `build:setup`, which installs only
this package's locked runtime dependency (`smol-toml`). The separate manifest and
lockfile keep VitePress, Vue and Mermaid out of native CI. Setup output goes to
stderr so `mise run --quiet cef-version` can be captured safely on first use.
CI installs this package once with `setup-build-scripts` and invokes the version
script directly to keep stdout machine-readable.

Node 26 executes the TypeScript source directly. Use erasable syntax, explicit
`.ts` import extensions and `import type`; there is no transpilation step.
`node:util` parses CLI arguments, `node:fs` stages artifacts and `node:child_process`
invokes Cargo and platform tools without a shell. macOS uses its native `plutil`,
`lipo`, `install_name_tool` and `codesign`. Copies preserve executable modes and
relative framework symlinks. Linux strips staged CEF libraries without changing
the cached runtime or Cargo's binaries.

`pack` accepts partial artifact sets, matching the previous xtask behavior.
Follow it with `validate --variant full|store` to require every target for a
distribution and reject excluded targets. Store manifests omit Windows/Linux
ARM64 libraries and their dependency dictionaries. Both variants keep macOS
universal binaries. The workflow creates the same `dist/addons/godot_cef` ZIP
layout as before.

Run `mise run build:check` to install development dependencies, type-check with
`tsc --noEmit`, and execute the Node tests. Tests cover version agreement, Full
and Store packages, cross compilation, file copying, and the macOS assembly and
signing sequence using fixture binaries. Actual native linking and signing are
checked by the existing platform build jobs. The Rust process supervisor and
HTTP integration runner remain available through `cargo xtask integration`.
