//! Validation command - checks packaged addon layout and required artifacts

use crate::bundle_common::validate_required_paths;
use crate::platform::{PLATFORM_SPECS, PackageVariant};
use std::path::Path;

pub fn run(
    addon_dir: &Path,
    variant: Option<PackageVariant>,
) -> Result<(), Box<dyn std::error::Error>> {
    let bin_dir = addon_dir.join("bin");
    if !bin_dir.exists() {
        return Err(format!(
            "Addon directory '{}' does not contain a bin/ directory",
            addon_dir.display()
        )
        .into());
    }

    if let Some(variant) = variant {
        let manifest = std::fs::read_to_string(addon_dir.join("godot_cef.gdextension"))?;
        for platform in PLATFORM_SPECS {
            if manifest.contains(platform.target) != variant.includes(platform.target) {
                return Err(format!(
                    "GDExtension manifest does not match {variant:?} target selection: {}",
                    platform.target
                )
                .into());
            }
        }
    }

    let mut validated = 0usize;
    for platform in PLATFORM_SPECS {
        let platform_dir = bin_dir.join(platform.target);
        if let Some(variant) = variant {
            if !variant.includes(platform.target) {
                if platform_dir.exists() {
                    return Err(format!(
                        "excluded target present in {variant:?} addon: {}",
                        platform.target
                    )
                    .into());
                }
                continue;
            }
            if !platform_dir.exists() {
                return Err(format!(
                    "missing required {variant:?} addon target: {}",
                    platform.target
                )
                .into());
            }
        }
        if !platform_dir.exists() {
            println!("Skipping {} (not present)", platform.target);
            continue;
        }

        validate_required_paths(
            &platform_dir,
            platform.required_files,
            platform.required_dirs,
        )?;
        println!("Validated {}", platform.target);
        validated += 1;
    }

    if validated == 0 {
        return Err("No platform directories found under addon bin/".into());
    }

    println!(
        "Validation complete: {} platform(s) checked in {}",
        validated,
        addon_dir.display()
    );
    Ok(())
}
