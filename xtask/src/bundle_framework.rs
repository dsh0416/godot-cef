use crate::bundle_common::{FrameworkInfoPlist, get_target_dir, run_cargo};
use std::fs;
use std::path::{Path, PathBuf};

const RESOURCES_PATH: &str = "Resources";

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

fn bundle(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let fmwk_path = create_framework(
        target_dir,
        "libgdcef.dylib",
        &target_dir.join("libgdcef.dylib"),
    )?;

    println!("Created: {}", fmwk_path.display());
    Ok(())
}

pub fn run(release: bool, target_dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo_args = vec!["build", "--lib", "--package", "gdcef"];
    if release {
        cargo_args.push("--release");
    }
    run_cargo(&cargo_args)?;

    let target_dir = get_target_dir(release, target_dir);
    bundle(&target_dir)?;

    Ok(())
}
