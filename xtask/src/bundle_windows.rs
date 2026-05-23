//! Windows bundling - copies CEF assets alongside the built binaries

use crate::bundle_common::{
    copy_directory, deploy_to_addon, get_cef_dir, get_target_dir, get_target_dir_for_target,
    run_cargo, validate_required_paths,
};
use std::fs;
use std::path::Path;

const TARGET_X64: &str = "x86_64-pc-windows-msvc";
const TARGET_ARM64: &str = "aarch64-pc-windows-msvc";

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
        Some(other) => Err(format!("unsupported Windows target: {other}").into()),
        None => Ok(default_platform_target()),
    }
}

/// CEF files that need to be copied to the target directory
const CEF_FILES: &[&str] = &[
    // Core CEF library
    "libcef.dll",
    "chrome_elf.dll",
    // Graphics libraries
    "libEGL.dll",
    "libGLESv2.dll",
    "d3dcompiler_47.dll",
    "dxcompiler.dll",
    "dxil.dll",
    // Vulkan/SwiftShader
    "vk_swiftshader.dll",
    "vk_swiftshader_icd.json",
    "vulkan-1.dll",
    // Resources
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "v8_context_snapshot.bin",
    // Bootstrap executables
    "bootstrap.exe",
    "bootstrapc.exe",
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

/// Files to deploy to the addon directory
const DEPLOY_FILES: &[&str] = &[
    "gdcef.dll",
    "gdcef_helper.exe",
    "libcef.dll",
    "chrome_elf.dll",
    "libEGL.dll",
    "libGLESv2.dll",
    "d3dcompiler_47.dll",
    "dxcompiler.dll",
    "dxil.dll",
    "vk_swiftshader.dll",
    "vk_swiftshader_icd.json",
    "vulkan-1.dll",
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "v8_context_snapshot.bin",
    "bootstrap.exe",
    "bootstrapc.exe",
];

/// Directories to deploy to the addon directory
const DEPLOY_DIRS: &[&str] = &["locales"];

fn bundle(target_dir: &Path, platform_target: &str) -> Result<(), Box<dyn std::error::Error>> {
    copy_cef_assets(target_dir)?;
    deploy_to_addon(target_dir, platform_target, DEPLOY_FILES, DEPLOY_DIRS)?;
    println!("Windows bundle complete: {}", target_dir.display());
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
