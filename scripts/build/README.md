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
`build` first builds `gdcef_helper`, creates a deterministic helper archive, then
builds `gdcef` and the CEF-independent `gdcef_bootstrap` with `--locked`. The core
embeds the archive through `GDCEF_HELPER_ARCHIVE`; packaging verifies those exact
bytes in the linked library. Cargo handles Rust
incremental compilation. Both tasks use explicit target triples, including host
builds, so binaries live in `<target-dir>/<triple>/<debug|release>/`. The output
root comes from `--target-dir`, then `CARGO_TARGET_DIR`, then the repository's
`target/`. macOS builds both architectures and additionally stages its universal
framework in `<target-dir>/<debug|release>/Godot CEF.framework`.

Helper archives and their staging trees live in
`<target-dir>/gdcef-helper/<target>/<profile>/`. `assemble` requires those inputs
from a preceding `build`; directly compiling only `gdcef` is not a distributable
build. The addon's native libraries and installed CEF files live under
`bin/<target>/<plugin-version>-cef-<cef-version>/`. Upgrades install beside older
runtimes, avoiding replacement of loaded native libraries.

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

`stage-artifact --target <target> --output artifacts/gdcef-<target>` copies a built
platform's versioned directory into the flat artifact input expected by `pack`.
`platform-dir --target <target>` prints that installed platform directory for
inspection. Each artifact contains `runtime-platform.json` with its exact native
library paths, required runtime files, and the helper payload's byte range and
SHA-256 digest. No target code executes while packing cross-platform artifacts.
CI wraps each platform directory in a tar before uploading, then restores it
before packing. This preserves executable modes, framework symlinks and hidden
installation receipts across GitHub's artifact transport. Final ZIPs preserve
relative symlinks as well.

`pack` accepts partial artifact sets. Both variants support Windows/Linux x86_64
and ARM64 and macOS universal. **Full** includes the complete offline runtime and
an active core descriptor. **Store** includes only the bootstrap and core native
libraries (plus the macOS framework plist), icons, license and install metadata;
the core descriptor remains inactive as `godot_cef.gdextension.in` until the
native editor installer has downloaded matching CEF files and extracted helpers,
then the editor exits or restarts to activate the browser extension.
The bootstrap descriptor is active in both variants. Store archives contain no
standalone helpers, upstream CEF executables, or CEF libraries.

`validate --variant full|store` requires every target and checks the appropriate
distribution contract, including the embedded archive digest and the Store's
strict file allowlist. Omit `--variant` to validate a partial local bundle.
Packaging keeps the `dist/addons/godot_cef` ZIP layout. Run
`node --test scripts/build/pack.test.ts` for archive and distribution regressions.

macOS helper apps are assembled before embedding. The parent app and outer
framework are signed only after CEF has been added. The editor installer follows
the same order before activating the core; exported games receive an already
expanded runtime. Windows/Linux export dependencies and optional upstream license
files are generated from the installation metadata.

The existing platform CI jobs validate native builds, macOS signing and Full/Store
packages. The Rust process supervisor and HTTP integration runner remain
available through `cargo xtask integration`.
