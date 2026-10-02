//! Assemble the native test driver without adding it to production packages.

use clap::Args;
use std::path::PathBuf;
use std::process::Command;

#[derive(Args)]
pub struct Options {
    /// Godot 4.5+ executable (use the console executable on Windows)
    #[arg(long)]
    godot: PathBuf,

    /// Complete production addon, including the helper and matching CEF runtime
    #[arg(long)]
    addon: Option<PathBuf>,

    /// Run one Class:case instead of the complete suite
    #[arg(long)]
    case: Option<String>,

    /// Parent directory for retained reports, logs, projects and CEF profiles
    #[arg(long)]
    output: Option<PathBuf>,

    /// Build the test driver in release mode
    #[arg(long)]
    release: bool,

    /// Explicit native Rust target; cross-compiled tests cannot run on this host
    #[arg(long)]
    target: Option<String>,
}

pub fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let native_target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return Err("headless integration tests currently support Linux and Windows".into()),
    };
    if let Some(target) = &options.target
        && target != native_target
    {
        return Err(format!("cannot run {target} integration tests on {native_target}").into());
    }

    // Resolve user paths before starting subprocesses. The runner always runs
    // from the repository, even when xtask is invoked from a subdirectory.
    let godot = options.godot.canonicalize()?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask has no repository parent")?
        .to_path_buf();
    let addon = options
        .addon
        .unwrap_or_else(|| root.join("addons/godot_cef"))
        .canonicalize()?;
    let output = std::path::absolute(
        options
            .output
            .unwrap_or_else(|| root.join("target/integration")),
    )?;

    // Cargo metadata honors CARGO_TARGET_DIR and .cargo/config.toml. Always
    // select the native target explicitly so an implicit cross-build setting
    // cannot accidentally run a stale library from another target directory.
    #[derive(serde::Deserialize)]
    struct Metadata {
        target_directory: PathBuf,
    }
    let metadata = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(&root)
        .output()?;
    if !metadata.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&metadata.stderr)
        )
        .into());
    }
    let metadata: Metadata = serde_json::from_slice(&metadata.stdout)?;
    let mut args = vec![
        "build",
        "--locked",
        "--package",
        "gdcef_itest",
        "--target",
        native_target,
    ];
    if options.release {
        args.push("--release");
    }
    println!("Running: cargo {}", args.join(" "));
    let status = Command::new("cargo")
        .args(args)
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err(format!("test addon build failed: {status}").into());
    }

    let profile = if options.release { "release" } else { "debug" };
    let target_dir = metadata.target_directory.join(native_target).join(profile);
    let library = match std::env::consts::OS {
        "windows" => "gdcef_itest.dll",
        _ => "libgdcef_itest.so",
    };
    let test_addon = target_dir.join(library).canonicalize()?;
    let mut command = Command::new("node");
    command
        .current_dir(&root)
        .arg(root.join("tests/integration/run.mjs"))
        .arg("--godot")
        .arg(godot)
        .arg("--addon")
        .arg(addon)
        .arg("--test-addon")
        .arg(test_addon)
        .arg("--output")
        .arg(output);
    if let Some(case) = options.case {
        command.arg("--case").arg(case);
    }
    let status = command.status()?;
    if !status.success() {
        return Err(format!("headless integration runner failed: {status}").into());
    }
    Ok(())
}
