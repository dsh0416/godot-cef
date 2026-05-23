//! Linux bundling - copies CEF assets alongside the built binaries

use crate::bundle_common::{
    copy_directory, deploy_to_addon, get_cef_dir, get_target_dir, get_target_dir_for_target,
    run_cargo, validate_required_paths,
};
use crate::platform::{LINUX_ARM64_TARGET, LINUX_RUNTIME_ASSETS, LINUX_X64_TARGET};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn default_platform_target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        LINUX_ARM64_TARGET
    } else {
        LINUX_X64_TARGET
    }
}

fn resolve_platform_target(
    target: Option<&str>,
) -> Result<&'static str, Box<dyn std::error::Error>> {
    match target {
        Some(LINUX_X64_TARGET) => Ok(LINUX_X64_TARGET),
        Some(LINUX_ARM64_TARGET) => Ok(LINUX_ARM64_TARGET),
        Some(other) => Err(format!("unsupported Linux target: {other}").into()),
        None => Ok(default_platform_target()),
    }
}

fn strip_tool_for_target(platform_target: &str) -> &'static str {
    match platform_target {
        LINUX_ARM64_TARGET => "aarch64-linux-gnu-strip",
        _ => "strip",
    }
}

fn copy_cef_assets(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cef_dir = get_cef_dir()
        .ok_or("CEF directory not found. Please set CEF_PATH environment variable.")?;

    validate_required_paths(
        &cef_dir,
        LINUX_RUNTIME_ASSETS.cef_files,
        LINUX_RUNTIME_ASSETS.cef_dirs,
    )?;

    println!("Copying CEF assets from: {}", cef_dir.display());

    for file in LINUX_RUNTIME_ASSETS.cef_files {
        let src = cef_dir.join(file);
        let dst = target_dir.join(file);

        if src.exists() {
            fs::copy(&src, &dst)?;
            println!("  Copied: {}", file);
        }
    }

    for dir in LINUX_RUNTIME_ASSETS.cef_dirs {
        let src = cef_dir.join(dir);
        let dst = target_dir.join(dir);

        if src.exists() {
            if dst.exists() {
                fs::remove_dir_all(&dst)?;
            }
            copy_directory(&src, &dst)?;
            println!("  Copied directory: {}", dir);
        }
    }

    Ok(())
}

fn strip_binary(path: &Path, strip_tool: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        println!("  Warning: {} not found, skipping strip", path.display());
        return Ok(());
    }

    println!("  Stripping: {}", path.display());

    let status = Command::new(strip_tool)
        .arg("--strip-debug")
        .arg(path)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err(format!("strip failed for {}: {}", path.display(), status).into());
    }

    Ok(())
}

fn strip_cef_binaries(
    target_dir: &Path,
    platform_target: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Stripping CEF binaries...");
    let strip_tool = strip_tool_for_target(platform_target);
    strip_binary(&target_dir.join("libcef.so"), strip_tool)?;
    strip_binary(&target_dir.join("libEGL.so"), strip_tool)?;
    strip_binary(&target_dir.join("libGLESv2.so"), strip_tool)?;
    strip_binary(&target_dir.join("libvk_swiftshader.so"), strip_tool)?;
    strip_binary(&target_dir.join("libvulkan.so.1"), strip_tool)?;
    Ok(())
}

fn bundle(target_dir: &Path, platform_target: &str) -> Result<(), Box<dyn std::error::Error>> {
    copy_cef_assets(target_dir)?;
    strip_cef_binaries(target_dir, platform_target)?;
    deploy_to_addon(
        target_dir,
        platform_target,
        LINUX_RUNTIME_ASSETS.deploy_files,
        LINUX_RUNTIME_ASSETS.deploy_dirs,
    )?;
    println!("Linux bundle complete: {}", target_dir.display());
    Ok(())
}

pub fn run(
    release: bool,
    target_dir: Option<&Path>,
    target: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let platform_target = resolve_platform_target(target)?;
    let mut cargo_args = vec!["build", "--package", "gdcef", "--package", "gdcef_helper"];
    if target.is_some() {
        cargo_args.push("--target");
        cargo_args.push(platform_target);
    }
    if release {
        cargo_args.push("--release");
    }
    run_cargo(&cargo_args)?;

    let target_dir = if target.is_some() {
        get_target_dir_for_target(release, platform_target, target_dir)
    } else {
        get_target_dir(release, target_dir)
    };
    bundle(&target_dir, platform_target)?;

    Ok(())
}
