//! Windows bundling - copies CEF assets alongside the built binaries

use crate::bundle_common::{
    copy_directory, deploy_to_addon, get_cef_dir, get_target_dir, get_target_dir_for_target,
    run_cargo, validate_required_paths,
};
use crate::platform::{WINDOWS_RUNTIME_ASSETS, WINDOWS_X64_TARGET};
use std::fs;
use std::path::Path;

fn resolve_platform_target(
    target: Option<&str>,
) -> Result<&'static str, Box<dyn std::error::Error>> {
    match target {
        Some(WINDOWS_X64_TARGET) => Ok(WINDOWS_X64_TARGET),
        Some(other) => Err(format!("unsupported Windows target: {other}").into()),
        None if cfg!(target_arch = "x86_64") => Ok(WINDOWS_X64_TARGET),
        None => Err(
            "native Windows bundling requires x86_64; Windows ARM64 support has been removed"
                .into(),
        ),
    }
}

fn copy_cef_assets(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cef_dir = get_cef_dir()
        .ok_or("CEF directory not found. Please set CEF_PATH environment variable.")?;

    validate_required_paths(
        &cef_dir,
        WINDOWS_RUNTIME_ASSETS.cef_files,
        WINDOWS_RUNTIME_ASSETS.cef_dirs,
    )?;

    println!("Copying CEF assets from: {}", cef_dir.display());

    for file in WINDOWS_RUNTIME_ASSETS.cef_files {
        let src = cef_dir.join(file);
        let dst = target_dir.join(file);

        if src.exists() {
            fs::copy(&src, &dst)?;
            println!("  Copied: {}", file);
        }
    }

    for dir in WINDOWS_RUNTIME_ASSETS.cef_dirs {
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

fn bundle(target_dir: &Path, platform_target: &str) -> Result<(), Box<dyn std::error::Error>> {
    copy_cef_assets(target_dir)?;
    deploy_to_addon(
        target_dir,
        platform_target,
        WINDOWS_RUNTIME_ASSETS.deploy_files,
        WINDOWS_RUNTIME_ASSETS.deploy_dirs,
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x64_target_is_supported() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            resolve_platform_target(Some(WINDOWS_X64_TARGET))?,
            WINDOWS_X64_TARGET
        );
        Ok(())
    }

    #[test]
    fn windows_arm64_target_is_rejected() {
        assert!(resolve_platform_target(Some("aarch64-pc-windows-msvc")).is_err());
    }

    #[test]
    fn native_target_requires_x64_host() {
        assert_eq!(
            resolve_platform_target(None).is_ok(),
            cfg!(target_arch = "x86_64")
        );
    }
}
