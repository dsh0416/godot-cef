# Contributing to Godot CEF

Thank you for your interest in contributing to Godot CEF! This document provides guidelines and instructions for contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Pull Request Process](#pull-request-process)
- [Reporting Issues](#reporting-issues)
- [Code Style](#code-style)
- [Testing](#testing)
- [Documentation](#documentation)

## Code of Conduct

Please be respectful and considerate in all interactions. We aim to maintain a welcoming and inclusive community for everyone.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/godot-cef.git
   cd godot-cef
   ```
3. **Add the upstream remote**:
   ```bash
   git remote add upstream https://github.com/dsh0416/godot-cef.git
   ```

## Development Setup

### Prerequisites

- **mise** — Install from [mise.jdx.dev](https://mise.jdx.dev/) and enable shell integration for your shell
- **Project toolchain** — Installed from `mise.toml`
  ```bash
  mise trust
  mise install
  ```
- **Godot Engine 4.6+** — Installed at the integration-test version by `mise install`
- **Platform-specific dependencies** (see below)

The commands below assume mise shell integration is active. If your shell is not configured for mise activation yet, prefix commands with `mise exec --`.

### Installing CEF Binaries

`mise install` installs the `export-cef-dir` tool. After activating the toolchain, derive the matching runtime version from the `cef` / `cef-dll-sys` build metadata in `Cargo.lock` and pass it explicitly to the exporter. Download CEF binaries for your platform:

#### Linux

```bash
export CEF_PATH="$HOME/.local/share/cef"
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"
export LD_LIBRARY_PATH="$CEF_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
```

For Linux ARM64 cross builds, download the matching CEF runtime and build with
the ARM64 Rust target:

```bash
export CEF_PATH="$HOME/.local/share/cef_aarch64"
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --target aarch64-unknown-linux-gnu --force "$CEF_PATH"
rustup target add aarch64-unknown-linux-gnu
cargo xtask bundle --release --target aarch64-unknown-linux-gnu
```

The repository config allows unresolved symbols from `libcef.so` during Linux
ARM64 cross linking, because those CEF system dependencies are provided by the
target ARM64 Linux runtime rather than the x64 build host.

You'll also need system dependencies:

```bash
sudo apt-get install -y \
    build-essential cmake libgtk-3-dev libnss3-dev \
    libatk1.0-dev libatk-bridge2.0-dev libcups2-dev \
    libdrm-dev libxkbcommon-dev libxcomposite-dev \
    libxdamage-dev libxrandr-dev libgbm-dev \
    libpango1.0-dev libasound2-dev

# Additional tools for Linux ARM64 cross builds
sudo apt-get install -y \
    gcc-aarch64-linux-gnu g++-aarch64-linux-gnu binutils-aarch64-linux-gnu
```

#### macOS

```bash
# Native architecture
export CEF_PATH="$HOME/.local/share/cef"
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"

# For universal builds (optional)
export CEF_PATH_X64="$HOME/.local/share/cef_x86_64"
export-cef-dir --version "$CEF_VERSION" --target x86_64-apple-darwin --force "$CEF_PATH_X64"
export CEF_PATH_ARM64="$HOME/.local/share/cef_arm64"
export-cef-dir --version "$CEF_VERSION" --target aarch64-apple-darwin --force "$CEF_PATH_ARM64"
```

#### Windows (PowerShell)

```powershell
$env:CEF_PATH="$env:USERPROFILE/.local/share/cef"
$env:CEF_VERSION = cargo run --locked --quiet -p xtask -- cef-version
if ($LASTEXITCODE -ne 0) { throw "Could not resolve CEF runtime version" }
export-cef-dir --version $env:CEF_VERSION --force $env:CEF_PATH
$env:PATH="$env:PATH;$env:CEF_PATH"
```

For Windows ARM64 cross builds from an x64 Windows machine, use the ARM64 CEF
runtime and Rust target:

```powershell
$env:CEF_PATH="$env:USERPROFILE/.local/share/cef_arm64"
$env:CEF_VERSION = cargo run --locked --quiet -p xtask -- cef-version
if ($LASTEXITCODE -ne 0) { throw "Could not resolve CEF runtime version" }
export-cef-dir --version $env:CEF_VERSION --target aarch64-pc-windows-msvc --force $env:CEF_PATH
rustup target add aarch64-pc-windows-msvc
cargo xtask bundle --release --target aarch64-pc-windows-msvc
```

### Building

```bash
# Debug build
cargo xtask bundle

# Release build
cargo xtask bundle --release
```

### Project Structure

```
godot-cef/
├── crates/
│   ├── gdcef/              # Main GDExtension library
│   │   └── src/
│   │       ├── cef_texture/        # CefTexture node implementation
│   │       ├── cef_texture2d/      # CefTexture2D implementation
│   │       ├── accelerated_osr/    # GPU-accelerated rendering
│   │       ├── godot_protocol/     # res:// and user:// scheme handlers
│   │       └── vulkan_hook/        # Vulkan extension injection
│   ├── gdcef_helper/       # CEF subprocess helper
│   ├── gdcef_itest/        # Test-only GDExtension driven by the Godot main loop
│   ├── cef_app/            # CEF application/browser configuration
│   └── software_render/    # CPU popup compositing helpers
├── xtask/                  # Build, bundle, pack, validation, and integration runner
├── benches/                # Criterion benchmarks
├── addons/godot_cef/       # Godot addon files and bundled bin/ outputs
├── tests/integration/      # Godot project and browser-page fixtures
└── docs/                   # Documentation site (VitePress)
```

## Making Changes

1. **Create a feature branch** from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following the [code style guidelines](#code-style)

3. **Test your changes** (see [Testing](#testing))

4. **Commit with clear messages**:
   ```bash
   git commit -m "feat: add support for XYZ"
   ```
   
   We follow [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat:` — New feature
   - `fix:` — Bug fix
   - `docs:` — Documentation changes
   - `refactor:` — Code refactoring
   - `test:` — Adding/updating tests
   - `chore:` — Maintenance tasks

## Pull Request Process

1. **Ensure your branch is up to date**:
   ```bash
   git fetch upstream
   git rebase upstream/main
   ```

2. **Push your branch** to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

3. **Open a Pull Request** against `main` branch

4. **Fill out the PR template** with:
   - Clear description of changes
   - Related issue numbers (if applicable)
   - Testing performed
   - Screenshots/videos for UI changes

5. **Address review feedback** and update your PR as needed

6. **CI checks must pass**:
   - Build succeeds on all platforms (macOS, Windows, Linux)
   - All tests pass
   - Clippy lints pass
   - Code is properly formatted

## Reporting Issues

When reporting issues, please include:

- **Clear title** describing the problem
- **Environment details**:
  - OS and version
  - Godot version
  - Graphics API (Vulkan/DirectX/Metal)
  - GPU model
- **Steps to reproduce** the issue
- **Expected vs actual behavior**
- **Logs/screenshots** if applicable

Use the appropriate issue template when available.

## Code Style

### Rust

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix all warnings
- Follow Rust naming conventions
- Document public APIs with doc comments
- Use meaningful variable and function names

```bash
# Format code
cargo fmt --all

# Check lints
cargo clippy --workspace --all-features -- -D warnings
```

### General Guidelines

- Keep functions focused and small
- Add comments for complex logic
- Avoid unnecessary dependencies
- Handle errors gracefully
- Consider cross-platform implications

## Testing

### CI gates and caches

`CI` is the only automatic entry workflow. It calls reusable Test, Build,
Documentation, and Coverage workflows and finishes with one stable **Gate** check that
maintainers can require in branch protection. There are no workflow path
filters, so every PR produces Gate. Gate runs with `always()` and checks both
the reusable workflow results and their explicitly exported required job
results. Failed, cancelled, or unexpectedly skipped required work cannot pass.

On PRs, main pushes, and manual runs, Gate requires both five-target Test/Clippy
matrices, formatting, version validation, macOS universal plus Windows/Linux x64
and ARM64 packaging, Linux x64 Godot headless integration tests, packing/validating
Full and Store addons, the docs build, and Linux unit/headless coverage uploaded
to Codecov.
On `v*` tag pushes, Test, Documentation, and Coverage are intentionally skipped;
Gate still requires all platform builds and both packages. Tag pushes and manual
tag runs create draft releases with both archives after Gate. Main pushes and
manual runs deploy Pages after Gate. Publication/deployment are downstream and
are not PR requirements. This workflow change does not configure branch
protection.

Tool setup explicitly installs only the locked tools needed by each job and
disables implicit tool installation in later commands. Its cache key includes
the mise version, runner OS/architecture, selected tools, and both
`mise.toml` and `mise.lock`. CEF caches use exact keys with the runner
OS/architecture, target triple, derived runtime version, and both mise files;
no fallback can restore a different CEF runtime. The macOS ARM64 runtime path is
shared between checking and universal packaging. The pnpm store is cached separately from `node_modules`,
with the toolchain and dependency lock/configuration in the key; installs still
use `--frozen-lockfile` and documentation is rebuilt.

Cargo caches compiled dependencies for Test/Clippy only, separated by target,
compiler, toolchain, Cargo configuration, and CEF environment. Packaging caches
Cargo downloads instead of large release targets. CI disables incremental
compilation so sccache can cache compiler work, and pins sccache itself to avoid
unplanned cache invalidation. Only main and `v*` tag pushes write shared caches;
PRs and manual runs restore them and use sccache in read-only mode. Cold-cache
runs still download and compile normally. Cache hits and performance depend on
available entries and repository cache quota.

### Running Tests

```bash
# Run all tests
export LD_LIBRARY_PATH="$CEF_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"   # Linux
export DYLD_LIBRARY_PATH="$CEF_PATH${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" # macOS
cargo test --workspace --all-features

# Run specific test
cargo test test_name

# Validate version/toolchain pins
cargo xtask validate-versions

# Validate a packaged addon layout
cargo xtask validate --addon dist/addons/godot_cef
```

On Windows, add the CEF runtime directory to `PATH` before running tests:

```powershell
$env:PATH="$env:CEF_PATH;$env:PATH"
cargo test --workspace --all-features
```

Use `cargo xtask validate-versions` after bumping Rust crate versions, the CEF
exporter, the docs package version, or `mise.toml`. It checks that
`Cargo.toml`, `Cargo.lock`, `package.json`, and `mise.toml` agree. The CEF crates
must have the same complete version. The exporter must match their major,
minor, and patch versions, plus the runtime metadata when present in its pin.

### Godot headless integration tests

The separate Rust `gdcef_itest` addon exercises the production extension inside
Godot 4.6, managed by `mise.toml` and `mise.lock`. Build a complete production
bundle first, then run:

```sh
cargo xtask bundle --release --target x86_64-unknown-linux-gnu
xvfb-run -a cargo xtask integration --release --target x86_64-unknown-linux-gnu \
  --godot "$(mise which godot)"
```

On Windows, omit `xvfb-run -a` and use the `*_console.exe` inside
`mise where godot`; the integration guide includes the PowerShell commands.
`--addon` selects a complete existing addon, `--output` selects an evidence
directory, and `--case CefTexture2D:permission_navigation` runs a single case.
The command builds only the test addon and reuses the production bundle.
Use `--test-addon <path>` to reuse an already built test library and skip that
build as well. The Rust `xtask` runner handles project staging, the loopback HTTP
server, process supervision, and reports; integration testing requires no Node.

Run the Rust harness guards without Godot or CEF:

```sh
cargo test --locked -p xtask integration::
```

CI runs all 14 cases after the native Linux x64 release build, and their result
is part of the required Build/Gate checks. Each case has a separate process and
CEF profile, independent inner/outer deadlines, a structured result, and a clean
exit requirement. JSON, JUnit and Godot logs are uploaded even on failures.

The engine uses `--headless`; Xvfb supplies CEF's X11 backend. This verifies
permissions, JavaScript/IPC and lifecycle behavior, but the dummy renderer does
not verify texture upload/readback, viewport pixels or GPU shared textures.
See [the integration guide](tests/integration/README.md) for coverage, local
prerequisites, failure semantics, and the migrated permission scenarios.

### Code coverage

The Coverage workflow instruments a separate Linux x64 debug build with
`cargo-llvm-cov`, pinned through mise, and the locked Rust toolchain's
`llvm-tools-preview` component. It runs the complete headless suite before the
workspace unit tests.
The headless-only report must contain executed lines from both `CefTexture` and
`CefTexture2D`; this checks that Godot's dynamically loaded production library
actually writes coverage data. The final LCOV report combines both test suites.
The test addon and benchmarks are excluded from reports; production crates and
the Rust xtask remain included. Coverage binaries never enter release packages.

To reproduce on Linux x64, first install the CEF build/runtime dependencies and
set `CEF_PATH` as for a normal bundle. Run the following in a subshell to keep the
instrumentation environment separate from subsequent normal builds:

```bash
mise install --locked rust aqua:taiki-e/cargo-llvm-cov godot
mise exec -- rustup component add llvm-tools-preview
(
  set -euo pipefail
  target=x86_64-unknown-linux-gnu
  export CARGO_TARGET_DIR="$PWD/target/coverage-build"
  export LD_LIBRARY_PATH="${CEF_PATH}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  coverage_env="$(mise exec -- cargo llvm-cov show-env --sh --target "$target")"
  eval "$coverage_env"
  mise exec -- cargo llvm-cov clean --workspace
  mise exec -- cargo xtask bundle --target "$target" --target-dir "$CARGO_TARGET_DIR"
  xvfb-run -a mise exec -- cargo xtask integration --target "$target" \
    --godot "$(mise which godot)" --output target/coverage/integration
  mise exec -- cargo test --locked --workspace --all-features --target "$target"
  mise exec -- cargo llvm-cov report --target "$target" \
    --ignore-filename-regex '(crates/gdcef_itest|benches)/' \
    --lcov --output-path target/coverage/lcov.info
)
```

The bundle command deploys instrumented binaries into the local
`addons/godot_cef/bin/` directory. Run a normal `cargo xtask bundle` afterwards
to restore your regular local addon. CI retains LCOV, the headless-only summary,
and integration evidence as `coverage-linux-x64` for 14 days.

Codecov project/patch statuses are informational while establishing the baseline;
test, profile validation, report generation, and upload failures still fail Gate.
The upload uses GitHub OIDC, with the action's tokenless fallback for public fork
PRs, so no `CODECOV_TOKEN` secret is needed. Repository maintainers must enable
the repository in [Codecov](https://app.codecov.io/gh/dsh0416/godot-cef) and grant
the [Codecov GitHub App](https://github.com/apps/codecov) access for PR reporting.

This report measures Rust paths executed on Linux x64. It does not measure CEF,
Godot internals, other platforms, or GPU behavior. Processes forcibly terminated
on a failure may not flush their profiles; failed runs are never uploaded as
successful coverage.

### Writing Tests

- Add unit tests for new functionality
- Test edge cases and error conditions
- Ensure tests are deterministic and don't depend on external state

### Manual Testing

For visual/rendering changes:

1. Build the extension with `cargo xtask bundle`
2. Copy artifacts to a Godot project
3. Test with different rendering backends
4. Verify on multiple platforms if possible

For release or packaging changes, also run `cargo xtask pack` with the
platform artifacts you changed and then `cargo xtask validate --addon` against
the staged addon directory.

### Distribution variants

`cargo xtask pack` defaults to the full addon, preserving all five platform
artifacts. For release packaging, stage and validate each variant independently:

```bash
cargo xtask pack --artifacts artifacts --output staging/full/dist/addons/godot_cef --variant full
cargo xtask validate --addon staging/full/dist/addons/godot_cef --variant full
cargo xtask pack --artifacts artifacts --output staging/store/dist/addons/godot_cef --variant store
cargo xtask validate --addon staging/store/dist/addons/godot_cef --variant store
```

Variant validation requires every selected target and rejects excluded target
directories. Omit `--variant` for the existing partial-addon validation behavior.
The Store packer derives its descriptor from the full source manifest by removing
Windows/Linux ARM64 entries; do not install both descriptors in one Godot project.
See [Distribution variants](docs/api/distribution-variants.md) for archive layout,
architecture coverage, and manual ARM64 builds. CI builds all architectures and
publishes both package artifacts; this PR does not itself publish a release.

### Lifecycle Cleanup Checklist

When changing browser lifecycle code, preserve these cleanup invariants for `CefTexture`:

- Browser is explicitly closed (`host.close_browser(true)`) before instance teardown finishes.
- Accelerated rendering RIDs are detached from `Texture2DRD` before freeing RIDs.
- Popup overlay node and popup texture state are released.
- Shared runtime handles (`render_size`, `cursor_type`, event/audio queues, sample-rate state) are cleared.
- CEF global retain/release count remains balanced per created texture instance.

If a change touches cleanup ordering, test repeated create/destroy cycles to confirm no leaked state and no stale texture references.

## Documentation

### Code Documentation

- Document all public types, functions, and modules
- Use rustdoc conventions

```rust
/// Brief description of the function.
///
/// # Arguments
///
/// * `param` - Description of the parameter
///
/// # Returns
///
/// Description of the return value
///
/// # Examples
///
/// ```
/// let result = my_function(arg);
/// ```
pub fn my_function(param: Type) -> ReturnType {
    // ...
}
```

### User Documentation

The documentation site is built with VitePress:

```bash
# Install dependencies
pnpm install

# Start dev server
pnpm docs:dev

# Build documentation
pnpm docs:build
```

Documentation files are in the `docs/` directory.

When updating public API docs, keep the English and `zh_CN` pages in sync or
note the translation follow-up clearly in the pull request.

## Questions?

If you have questions about contributing:

- Open a [Discussion](https://github.com/dsh0416/godot-cef/discussions) on GitHub
- Check existing issues and PRs for similar topics

Thank you for contributing! 🎉
