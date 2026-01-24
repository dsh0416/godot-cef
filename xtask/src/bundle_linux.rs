//! Linux bundling - copies CEF assets alongside the built binaries

use crate::bundle_common::{
    copy_directory, deploy_to_addon, get_cef_dir, get_target_dir, run_cargo,
};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const PLATFORM_TARGET: &str = "x86_64-unknown-linux-gnu";

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

    println!("Copying CEF assets from: {}", cef_dir.display());

    for file in CEF_FILES {
        let src = cef_dir.join(file);
        let dst = target_dir.join(file);

        if src.exists() {
            fs::copy(&src, &dst)?;
            println!("  Copied: {}", file);
        } else {
            println!("  Warning: {} not found in CEF directory", file);
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
        } else {
            println!("  Warning: {} directory not found in CEF directory", dir);
        }
    }

    Ok(())
}

fn strip_binary(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        println!("  Warning: {} not found, skipping strip", path.display());
        return Ok(());
    }

    println!("  Stripping: {}", path.display());

    let status = Command::new("strip")
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

fn strip_cef_binaries(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Stripping CEF binaries...");
    strip_binary(&target_dir.join("libcef.so"))?;
    strip_binary(&target_dir.join("libEGL.so"))?;
    strip_binary(&target_dir.join("libGLESv2.so"))?;
    strip_binary(&target_dir.join("libvk_swiftshader.so"))?;
    strip_binary(&target_dir.join("libvulkan.so.1"))?;
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

fn bundle(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    copy_cef_assets(target_dir)?;
    strip_cef_binaries(target_dir)?;
    deploy_to_addon(target_dir, PLATFORM_TARGET, DEPLOY_FILES, DEPLOY_DIRS)?;
    println!("Linux bundle complete: {}", target_dir.display());
    Ok(())
}

pub fn run(release: bool, target_dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo_args = vec!["build", "--lib", "--package", "gdcef"];
    if release {
        cargo_args.push("--release");
    }
    run_cargo(&cargo_args)?;

    let mut cargo_args = vec!["build", "--bin", "gdcef_helper"];
    if release {
        cargo_args.push("--release");
    }
    run_cargo(&cargo_args)?;

    let target_dir = get_target_dir(release, target_dir);
    bundle(&target_dir)?;

    Ok(())
}
