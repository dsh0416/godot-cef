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

- **Rust (nightly)** — Install via [rustup](https://rustup.rs/)
  ```bash
  rustup default nightly
  ```
- **Godot Engine 4.5+** — Download from [godotengine.org](https://godotengine.org/)
- **Platform-specific dependencies** (see below)

### Installing CEF Binaries

First, install the CEF export tool:

```bash
cargo install export-cef-dir
```

Then download CEF binaries for your platform:

#### Linux

```bash
export-cef-dir --version "144.0.11" --force $HOME/.local/share/cef
export CEF_PATH="$HOME/.local/share/cef"
export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$CEF_PATH"
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
export-cef-dir --version "144.0.11" --force $HOME/.local/share/cef
export CEF_PATH="$HOME/.local/share/cef"

# For universal builds (optional)
export-cef-dir --version "144.0.11" --target x86_64-apple-darwin --force $HOME/.local/share/cef_x86_64
export CEF_PATH_X64="$HOME/.local/share/cef_x86_64"
export-cef-dir --version "144.0.11" --target aarch64-apple-darwin --force $HOME/.local/share/cef_arm64
export CEF_PATH_ARM64="$HOME/.local/share/cef_arm64"
```

#### Windows (PowerShell)

```powershell
export-cef-dir --version "144.0.11" --force $env:USERPROFILE/.local/share/cef
$env:CEF_PATH="$env:USERPROFILE/.local/share/cef"
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
