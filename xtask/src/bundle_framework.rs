use crate::bundle_common::{
    FrameworkInfoPlist, deploy_bundle_to_addon, get_target_dir, get_target_dir_for_target,
    run_cargo, run_lipo,
};
use std::fs;
use std::path::{Path, PathBuf};

const PLATFORM_TARGET: &str = "universal-apple-darwin";

const RESOURCES_PATH: &str = "Resources";
const TARGET_ARM64: &str = "aarch64-apple-darwin";
const TARGET_X64: &str = "x86_64-apple-darwin";

fn create_framework_layout(fmwk_path: &Path) -> PathBuf {
    fs::create_dir_all(fmwk_path.join(RESOURCES_PATH)).unwrap();
    fmwk_path.join(RESOURCES_PATH)
}

fn create_framework_info_plist(
    resources_path: &Path,
    lib_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let info_plist = FrameworkInfoPlist::new(lib_name);
    plist::to_file_xml(resources_path.join("Info.plist"), &info_plist)?;
    Ok(())
}

fn create_framework(
    fmwk_path: &Path,
    lib_name: &str,
    bin: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let fmwk_path = fmwk_path.join("Godot CEF.framework");
    if fmwk_path.exists() {
        fs::remove_dir_all(&fmwk_path)?;
    }

    let resources_path = create_framework_layout(&fmwk_path);
    create_framework_info_plist(&resources_path, lib_name)?;
    fs::copy(bin, fmwk_path.join(lib_name))?;
    Ok(fmwk_path)
}

fn bundle(
    target_dir: &Path,
    universal_dylib: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let fmwk_path = create_framework(target_dir, "libgdcef.dylib", universal_dylib)?;

    println!("Created: {}", fmwk_path.display());
    Ok(fmwk_path)
}

pub fn run(release: bool, target_dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo_args_arm64 = vec![
        "build",
        "--lib",
        "--package",
        "gdcef",
        "--target",
        TARGET_ARM64,
    ];
    if release {
        cargo_args_arm64.push("--release");
    }
    run_cargo(&cargo_args_arm64)?;

    let mut cargo_args_x64 = vec![
        "build",
        "--lib",
        "--package",
        "gdcef",
        "--target",
        TARGET_X64,
    ];
    if release {
        cargo_args_x64.push("--release");
    }
    run_cargo(&cargo_args_x64)?;

    let target_dir_arm64 = get_target_dir_for_target(release, TARGET_ARM64, target_dir);
    let target_dir_x64 = get_target_dir_for_target(release, TARGET_X64, target_dir);
    let output_dir = get_target_dir(release, target_dir);

    let dylib_arm64 = target_dir_arm64.join("libgdcef.dylib");
    let dylib_x64 = target_dir_x64.join("libgdcef.dylib");
    let universal_dylib = output_dir.join("libgdcef_universal.dylib");

    run_lipo(&dylib_arm64, &dylib_x64, &universal_dylib)?;

    let fmwk_path = bundle(&output_dir, &universal_dylib)?;
    fs::remove_file(&universal_dylib)?;
    deploy_bundle_to_addon(&fmwk_path, PLATFORM_TARGET)?;

    Ok(())
}
