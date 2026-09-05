use crate::bundle_common::{
    FrameworkInfoPlist, TARGET_ARM64, TARGET_X64, deploy_bundle_to_addon, get_target_dir,
    get_target_dir_for_target, run_cargo_for_macos_targets, run_lipo, sign_macos_code,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PLATFORM_TARGET: &str = "universal-apple-darwin";

const RESOURCES_PATH: &str = "Resources";
const CEF_APP_PATH: &str = "Helpers/Godot CEF.app";
const INSTALL_NAME: &str = "@rpath/Godot CEF.framework/libgdcef.dylib";

fn create_framework_layout(fmwk_path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    fs::create_dir_all(fmwk_path.join(RESOURCES_PATH))?;
    Ok(fmwk_path.join(RESOURCES_PATH))
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

    let resources_path = create_framework_layout(&fmwk_path)?;
    create_framework_info_plist(&resources_path, lib_name)?;
    fs::copy(bin, fmwk_path.join(lib_name))?;
    Ok(fmwk_path)
}

fn bundle(
    target_dir: &Path,
    universal_dylib: &Path,
    cef_app: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let fmwk_path = create_framework(target_dir, "libgdcef.dylib", universal_dylib)?;
    let cef_app_path = fmwk_path.join(CEF_APP_PATH);
    fs::create_dir_all(cef_app_path.parent().ok_or("Invalid CEF app path")?)?;
    fs::rename(cef_app, cef_app_path)?;

    let library_path = fmwk_path.join("libgdcef.dylib");
    let status = Command::new("install_name_tool")
        .args(["-id", INSTALL_NAME])
        .arg(&library_path)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if !status.success() {
        return Err(format!("install_name_tool failed with status: {status}").into());
    }

    sign_macos_code(&fmwk_path)?;
    println!("Created: {}", fmwk_path.display());
    Ok(fmwk_path)
}

pub fn run(release: bool, target_dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let cef_app = crate::bundle_app::build(release, target_dir)?;
    run_cargo_for_macos_targets(&["build", "--lib", "--package", "gdcef"], release)?;

    let target_dir_arm64 = get_target_dir_for_target(release, TARGET_ARM64, target_dir);
    let target_dir_x64 = get_target_dir_for_target(release, TARGET_X64, target_dir);
    let output_dir = get_target_dir(release, target_dir);

    let dylib_arm64 = target_dir_arm64.join("libgdcef.dylib");
    let dylib_x64 = target_dir_x64.join("libgdcef.dylib");
    let universal_dylib = output_dir.join("libgdcef_universal.dylib");

    run_lipo(&dylib_arm64, &dylib_x64, &universal_dylib)?;

    let fmwk_path = bundle(&output_dir, &universal_dylib, &cef_app)?;
    fs::remove_file(&universal_dylib)?;
    deploy_bundle_to_addon(&fmwk_path, PLATFORM_TARGET)?;

    Ok(())
}
