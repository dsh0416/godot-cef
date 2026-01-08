use crate::bundle_common::{AppInfoPlist, copy_directory, get_cef_dir, get_target_dir, run_cargo};
use std::fs;
use std::path::{Path, PathBuf};

const EXEC_PATH: &str = "Contents/MacOS";
const FRAMEWORKS_PATH: &str = "Contents/Frameworks";
const RESOURCES_PATH: &str = "Contents/Resources";
const FRAMEWORK: &str = "Chromium Embedded Framework.framework";
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

fn bundle(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let main_app_path = create_app(
        target_dir,
        "Godot CEF",
        &target_dir.join("gdcef_helper"),
        false,
    )?;

    let cef_path = get_cef_dir()
        .ok_or("CEF directory not found. Please set CEF_PATH environment variable.")?;
    let to = main_app_path.join(FRAMEWORKS_PATH).join(FRAMEWORK);
    if to.exists() {
        fs::remove_dir_all(&to)?;
    }
    copy_directory(&cef_path.join(FRAMEWORK), &to)?;

    for helper in HELPERS {
        create_app(
            &main_app_path.join(FRAMEWORKS_PATH),
            helper,
            &target_dir.join("gdcef_helper"),
            true,
        )?;
    }

    println!("Created: {}", main_app_path.display());
    Ok(())
}

pub fn run(release: bool, target_dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo_args = vec!["build", "--bin", "gdcef_helper"];
    if release {
        cargo_args.push("--release");
    }
    run_cargo(&cargo_args)?;

    let target_dir = get_target_dir(release, target_dir);
    bundle(&target_dir)?;

    Ok(())
}
