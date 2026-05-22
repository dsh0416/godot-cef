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
- **Godot Engine 4.5+** — Download from [godotengine.org](https://godotengine.org/)
- **Platform-specific dependencies** (see below)

The commands below assume mise shell integration is active. If your shell is not configured for mise activation yet, prefix commands with `mise exec --`.

### Installing CEF Binaries

`mise install` installs the `export-cef-dir` tool and exposes the pinned `CEF_VERSION` from `mise.toml`. Download CEF binaries for your platform:

#### Linux

```bash
export CEF_PATH="$HOME/.local/share/cef"
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"
export LD_LIBRARY_PATH="$CEF_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
```

You'll also need system dependencies:

```bash
sudo apt-get install -y \
    build-essential cmake libgtk-3-dev libnss3-dev \
    libatk1.0-dev libatk-bridge2.0-dev libcups2-dev \
    libdrm-dev libxkbcommon-dev libxcomposite-dev \
    libxdamage-dev libxrandr-dev libgbm-dev \
    libpango1.0-dev libasound2-dev
```

#### macOS

```bash
# Native architecture
export CEF_PATH="$HOME/.local/share/cef"
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
export-cef-dir --version $env:CEF_VERSION --force $env:CEF_PATH
$env:PATH="$env:PATH;$env:CEF_PATH"
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
├── gdcef/              # Main GDExtension library
│   └── src/
│       ├── cef_texture/        # CefTexture node implementation
│       ├── accelerated_osr/    # GPU-accelerated rendering
│       ├── vulkan_hook/        # Vulkan extension injection
│       └── ...
├── gdcef_helper/       # CEF subprocess helper
├── cef_app/            # CEF application/browser configuration
├── xtask/              # Build system and bundling tasks
├── addons/             # Godot addon files
└── docs/               # Documentation (VitePress)
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

### Running Tests

```bash
# Run all tests
cargo test --workspace --all-features

# Run specific test
cargo test test_name
```

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

## Questions?

If you have questions about contributing:

- Open a [Discussion](https://github.com/dsh0416/godot-cef/discussions) on GitHub
- Check existing issues and PRs for similar topics

Thank you for contributing! 🎉
