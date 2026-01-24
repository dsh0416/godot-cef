//! Windows bundling - copies CEF assets alongside the built binaries

use crate::bundle_common::{
    copy_directory, deploy_to_addon, get_cef_dir, get_target_dir, run_cargo,
};
use std::fs;
use std::path::Path;

const PLATFORM_TARGET: &str = "x86_64-pc-windows-msvc";

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

fn bundle(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    copy_cef_assets(target_dir)?;
    deploy_to_addon(target_dir, PLATFORM_TARGET, DEPLOY_FILES, DEPLOY_DIRS)?;
    println!("Windows bundle complete: {}", target_dir.display());
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
