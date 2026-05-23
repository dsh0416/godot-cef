//! Linux bundling - copies CEF assets alongside the built binaries

use crate::bundle_common::{
    copy_directory, deploy_to_addon, get_cef_dir, get_target_dir, get_target_dir_for_target,
    run_cargo, validate_required_paths,
};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const TARGET_X64: &str = "x86_64-unknown-linux-gnu";
const TARGET_ARM64: &str = "aarch64-unknown-linux-gnu";

fn default_platform_target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        TARGET_ARM64
    } else {
        TARGET_X64
    }
}

fn resolve_platform_target(
    target: Option<&str>,
) -> Result<&'static str, Box<dyn std::error::Error>> {
    match target {
        Some(TARGET_X64) => Ok(TARGET_X64),
        Some(TARGET_ARM64) => Ok(TARGET_ARM64),
        Some(other) => Err(format!("unsupported Linux target: {other}").into()),
        None => Ok(default_platform_target()),
    }
}

fn strip_tool_for_target(platform_target: &str) -> &'static str {
    match platform_target {
        TARGET_ARM64 => "aarch64-linux-gnu-strip",
        _ => "strip",
    }
}

/// CEF files that need to be copied to the target directory
const CEF_FILES: &[&str] = &[
    // Core CEF library
    "libcef.so",
    // Graphics libraries
    "libEGL.so",
    "libGLESv2.so",
    // Vulkan/SwiftShader
    "libvk_swiftshader.so",
    "libvulkan.so.1",
    "vk_swiftshader_icd.json",
    // Resources
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "v8_context_snapshot.bin",
    // Chrome sandbox
    "chrome-sandbox",
];

/// CEF directories that need to be copied
const CEF_DIRS: &[&str] = &["locales"];

fn copy_cef_assets(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cef_dir = get_cef_dir()
        .ok_or("CEF directory not found. Please set CEF_PATH environment variable.")?;

    validate_required_paths(&cef_dir, CEF_FILES, CEF_DIRS)?;

    println!("Copying CEF assets from: {}", cef_dir.display());

    for file in CEF_FILES {
        let src = cef_dir.join(file);
        let dst = target_dir.join(file);

        if src.exists() {
            fs::copy(&src, &dst)?;
            println!("  Copied: {}", file);
        }
    }

    for dir in CEF_DIRS {
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

/// Files to deploy to the addon directory
const DEPLOY_FILES: &[&str] = &[
    "libgdcef.so",
    "gdcef_helper",
    "libcef.so",
    "libEGL.so",
    "libGLESv2.so",
    "libvk_swiftshader.so",
    "libvulkan.so.1",
    "vk_swiftshader_icd.json",
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "v8_context_snapshot.bin",
    "chrome-sandbox",
];

/// Directories to deploy to the addon directory
const DEPLOY_DIRS: &[&str] = &["locales"];

fn bundle(target_dir: &Path, platform_target: &str) -> Result<(), Box<dyn std::error::Error>> {
    copy_cef_assets(target_dir)?;
    strip_cef_binaries(target_dir, platform_target)?;
    deploy_to_addon(target_dir, platform_target, DEPLOY_FILES, DEPLOY_DIRS)?;
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
