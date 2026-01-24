use crate::bundle_common::{
    AppInfoPlist, copy_directory, deploy_bundle_to_addon, get_cef_dir_arm64, get_cef_dir_x64,
    get_target_dir, get_target_dir_for_target, run_cargo, run_lipo,
};
use std::fs;
use std::path::{Path, PathBuf};

const PLATFORM_TARGET: &str = "universal-apple-darwin";

const EXEC_PATH: &str = "Contents/MacOS";
const FRAMEWORKS_PATH: &str = "Contents/Frameworks";
const RESOURCES_PATH: &str = "Contents/Resources";
const FRAMEWORK: &str = "Chromium Embedded Framework.framework";
const FRAMEWORK_ARM64: &str = "Chromium Embedded Framework (ARM64).framework";
const FRAMEWORK_X64: &str = "Chromium Embedded Framework (X86_64).framework";
const TARGET_ARM64: &str = "aarch64-apple-darwin";
const TARGET_X64: &str = "x86_64-apple-darwin";
const HELPERS: &[&str] = &[
    "Godot CEF Helper (GPU)",
    "Godot CEF Helper (Renderer)",
    "Godot CEF Helper (Plugin)",
    "Godot CEF Helper (Alerts)",
    "Godot CEF Helper",
];

fn create_app_layout(app_path: &Path) -> PathBuf {
    [EXEC_PATH, RESOURCES_PATH, FRAMEWORKS_PATH]
        .iter()
        .for_each(|p| fs::create_dir_all(app_path.join(p)).unwrap());
    app_path.join("Contents")
}

fn create_app_info_plist(
    contents_path: &Path,
    exec_name: &str,
    is_helper: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let info_plist = AppInfoPlist::new(exec_name, is_helper);
    plist::to_file_xml(contents_path.join("Info.plist"), &info_plist)?;
    Ok(())
}

fn create_app(
    app_path: &Path,
    exec_name: &str,
    bin: &Path,
    is_helper: bool,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let app_path = app_path.join(exec_name).with_extension("app");
    let contents_path = create_app_layout(&app_path);
    create_app_info_plist(&contents_path, exec_name, is_helper)?;
    fs::copy(bin, app_path.join(EXEC_PATH).join(exec_name))?;
    Ok(app_path)
}

fn bundle(
    target_dir: &Path,
    universal_helper: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let main_app_path = create_app(target_dir, "Godot CEF", universal_helper, false)?;

    let cef_path_arm64 = get_cef_dir_arm64()
        .ok_or("CEF ARM64 directory not found. Please set CEF_PATH_ARM64 environment variable.")?;
    let to_arm64 = main_app_path.join(FRAMEWORKS_PATH).join(FRAMEWORK_ARM64);
    if to_arm64.exists() {
        fs::remove_dir_all(&to_arm64)?;
    }
    copy_directory(&cef_path_arm64.join(FRAMEWORK), &to_arm64)?;
    println!("Copied: {}", FRAMEWORK_ARM64);

    let cef_path_x64 = get_cef_dir_x64()
        .ok_or("CEF X64 directory not found. Please set CEF_PATH_X64 environment variable.")?;
    let to_x64 = main_app_path.join(FRAMEWORKS_PATH).join(FRAMEWORK_X64);
    if to_x64.exists() {
        fs::remove_dir_all(&to_x64)?;
    }
    copy_directory(&cef_path_x64.join(FRAMEWORK), &to_x64)?;
    println!("Copied: {}", FRAMEWORK_X64);

    for helper in HELPERS {
        create_app(
            &main_app_path.join(FRAMEWORKS_PATH),
            helper,
            universal_helper,
            true,
        )?;
    }

    println!("Created: {}", main_app_path.display());
    Ok(main_app_path)
}

pub fn run(release: bool, target_dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo_args_arm64 = vec!["build", "--bin", "gdcef_helper", "--target", TARGET_ARM64];
    if release {
        cargo_args_arm64.push("--release");
    }
    run_cargo(&cargo_args_arm64)?;

    let mut cargo_args_x64 = vec!["build", "--bin", "gdcef_helper", "--target", TARGET_X64];
    if release {
        cargo_args_x64.push("--release");
    }
    run_cargo(&cargo_args_x64)?;

    let target_dir_arm64 = get_target_dir_for_target(release, TARGET_ARM64, target_dir);
    let target_dir_x64 = get_target_dir_for_target(release, TARGET_X64, target_dir);
    let output_dir = get_target_dir(release, target_dir);

    let helper_arm64 = target_dir_arm64.join("gdcef_helper");
    let helper_x64 = target_dir_x64.join("gdcef_helper");
    let universal_helper = output_dir.join("gdcef_helper_universal");

    run_lipo(&helper_arm64, &helper_x64, &universal_helper)?;

    let app_path = bundle(&output_dir, &universal_helper)?;
    fs::remove_file(&universal_helper)?;
    deploy_bundle_to_addon(&app_path, PLATFORM_TARGET)?;

    Ok(())
}
