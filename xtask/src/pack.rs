//! Pack command - assembles all platform artifacts into a single Godot addon

use crate::bundle_common::{copy_directory, validate_required_paths};
use crate::platform::{PLATFORM_SPECS, PlatformSpec};
use std::fs;
use std::path::Path;

fn copy_platform_artifacts(
    artifacts_dir: &Path,
    output_bin_dir: &Path,
    platform: &PlatformSpec,
) -> Result<bool, Box<dyn std::error::Error>> {
    let src_dir = artifacts_dir.join(platform.artifact_name);

    if !src_dir.exists() {
        println!("  Skipping {} (not found)", platform.artifact_name);
        return Ok(false);
    }

    let dst_dir = output_bin_dir.join(platform.target);
    if dst_dir.exists() {
        fs::remove_dir_all(&dst_dir)?;
    }

    copy_directory(&src_dir, &dst_dir)?;
    validate_required_paths(&dst_dir, platform.required_files, platform.required_dirs)?;

    println!(
        "  Copied: {} -> bin/{}/",
        platform.artifact_name, platform.target
    );
    Ok(true)
}

fn copy_addon_files(addon_src: &Path, output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let gdext_src = addon_src.join("godot_cef.gdextension");
    if gdext_src.exists() {
        fs::copy(&gdext_src, output_dir.join("godot_cef.gdextension"))?;
        println!("  Copied: godot_cef.gdextension");
    }

    let icons_src = addon_src.join("icons");
    if icons_src.exists() {
        let icons_dst = output_dir.join("icons");
        if icons_dst.exists() {
            fs::remove_dir_all(&icons_dst)?;
        }
        copy_directory(&icons_src, &icons_dst)?;
        println!("  Copied: icons/");
    }

    Ok(())
}

pub fn run(
    artifacts_dir: &Path,
    output_dir: &Path,
    addon_src: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Packing Godot addon from artifacts...");
    println!("  Artifacts: {}", artifacts_dir.display());
    println!("  Output: {}", output_dir.display());

    if output_dir.exists() {
        fs::remove_dir_all(output_dir)?;
    }
    let bin_dir = output_dir.join("bin");
    fs::create_dir_all(&bin_dir)?;

    if let Some(addon_path) = addon_src {
        copy_addon_files(addon_path, output_dir)?;
    } else {
        let workspace_addon = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(
                || Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf(),
                std::path::Path::to_path_buf,
            )
            .join("addons/godot_cef");
        if workspace_addon.exists() {
            copy_addon_files(&workspace_addon, output_dir)?;
        }
    }

    let mut platforms_found = 0;
    for platform in PLATFORM_SPECS {
        if copy_platform_artifacts(artifacts_dir, &bin_dir, platform)? {
            platforms_found += 1;
        }
    }

    if platforms_found == 0 {
        return Err("No platform artifacts found!".into());
    }

    println!(
        "Pack complete! {} platform(s) included in {}",
        platforms_found,
        output_dir.display()
    );

    Ok(())
}
